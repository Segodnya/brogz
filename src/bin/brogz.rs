use std::path::PathBuf;
use std::process::ExitCode;

use brogz::{Config, Report};
use clap::Parser;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "brogz", version, about, long_about = None)]
struct Cli {
    /// Base URL, e.g. https://app.example
    url: Url,

    /// Probes per URL × encoding
    #[arg(short = 'r', long, default_value_t = brogz::DEFAULT_RUNS)]
    runs: usize,

    /// Max parallel requests in flight [default: 3 * runs]
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,

    /// Skip TLS verification
    #[arg(short = 'k', long)]
    insecure: bool,

    /// Write JSON report to file (in addition to stdout table)
    #[arg(short = 'o', long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Skip auto-discovery; measure these paths instead (repeatable)
    #[arg(short = 'p', long = "path", value_name = "PATH")]
    paths: Vec<String>,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Warnings from aggregate (size / Content-Encoding mismatches) go to stderr
    // so they do not pollute stdout (which carries the table). `RUST_LOG` still
    // overrides if the user wants finer control.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_target(false)
        .init();

    let runs = cli.runs;
    if runs == 0 {
        eprintln!("error: --runs must be >= 1");
        return ExitCode::from(2);
    }
    let concurrency = cli.concurrency.unwrap_or(runs * 3);

    let base_display = cli.url.as_str().trim_end_matches('/');
    println!("BASE_URL: {base_display}");
    println!("Runs per URL × encoding: {runs}");
    if cli.insecure {
        println!("TLS verification: OFF (--insecure)");
    }
    println!();

    let config = Config {
        base_url: cli.url.clone(),
        runs,
        concurrency,
        insecure: cli.insecure,
        paths: (!cli.paths.is_empty()).then_some(cli.paths),
    };

    let report = match brogz::run(config).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    print_table(&report, !cli.no_color);

    if let Some(out) = cli.out {
        if let Err(e) = write_report(&out, &report) {
            eprintln!("error: failed to write report to {}: {e}", out.display());
            return ExitCode::from(1);
        }
        println!("\nReport saved to {}", out.display());
    }

    ExitCode::SUCCESS
}

fn print_table(report: &Report, color: bool) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            "path", "raw", "gzip", "gz CE", "br", "br CE", "br vs gz",
        ]);

    for m in &report.measurements {
        let is_better = m.br.bytes > 0 && m.br.bytes < m.gzip.bytes;
        let row_color = color.then_some(if is_better { Color::Green } else { Color::Red });

        table.add_row(colored_row(
            row_color,
            [
                m.path.clone(),
                format_kb(m.identity.bytes),
                format_kb(m.gzip.bytes),
                m.gzip.content_encoding.clone(),
                format_kb(m.br.bytes),
                m.br.content_encoding.clone(),
                format_delta(m.br.bytes, m.gzip.bytes),
            ],
        ));
    }

    let totals_color = color.then_some(Color::Cyan);
    table.add_row(colored_row(
        totals_color,
        [
            "Σ TOTAL".to_owned(),
            format_kb(report.totals.identity),
            format_kb(report.totals.gzip),
            String::new(),
            format_kb(report.totals.br),
            String::new(),
            format_delta(report.totals.br, report.totals.gzip),
        ],
    ));

    println!("{table}");
}

fn colored_row(color: Option<Color>, cells: [String; 7]) -> Vec<Cell> {
    cells
        .into_iter()
        .map(|s| match color {
            Some(c) => Cell::new(s).fg(c),
            None => Cell::new(s),
        })
        .collect()
}

fn format_kb(bytes: u64) -> String {
    format!("{:.2}KB", bytes as f64 / 1024.0)
}

fn format_delta(current: u64, base: u64) -> String {
    if base == 0 {
        return "—".to_owned();
    }
    let pct = (current as f64 - base as f64) / base as f64 * 100.0;
    let sign = if pct > 0.0 { "+" } else { "" };
    format!("{sign}{pct:.1}%")
}

fn write_report(path: &std::path::Path, report: &Report) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, format!("{json}\n"))?;
    Ok(())
}
