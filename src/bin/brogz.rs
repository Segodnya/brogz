use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use brogz::{Config, EncodingMeasurement, ProgressCallback, ProgressEvent, Report};
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

    /// Suppress progress lines on stderr (warnings/errors are unaffected)
    #[arg(short = 'q', long)]
    quiet: bool,
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
        progress: (!cli.quiet).then(build_progress_reporter),
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

/// Build a progress callback that prints to stderr. Uses atomic counters
/// because `buffered` runs URL futures in parallel — completion events fire
/// from multiple tasks without ordering guarantees.
fn build_progress_reporter() -> ProgressCallback {
    let counter = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    Arc::new(move |event| match event {
        ProgressEvent::Discovered {
            url_count,
            probes_per_url,
        } => {
            total.store(url_count, Ordering::Relaxed);
            let probes = url_count * probes_per_url;
            eprintln!(
                "Discovered {url_count} URL{} ({probes} probe{} total)",
                if url_count == 1 { "" } else { "s" },
                if probes == 1 { "" } else { "s" },
            );
        }
        ProgressEvent::UrlCompleted { path } => {
            let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
            let t = total.load(Ordering::Relaxed);
            eprintln!("  [{n}/{t}] {path}");
        }
    })
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
                format_bytes_cell(&m.identity),
                format_bytes_cell(&m.gzip),
                m.gzip.content_encoding.clone(),
                format_bytes_cell(&m.br),
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
            format_totals_cell(report, |m| &m.identity),
            format_totals_cell(report, |m| &m.gzip),
            String::new(),
            format_totals_cell(report, |m| &m.br),
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

/// Spread under this is rounding noise (sub-byte differences after compression
/// of identical bodies). Anything above signals dynamic content — surface it.
const VARIANCE_THRESHOLD_PCT: f64 = 1.0;

/// Append "±X.X%" when probe-to-probe byte variance is meaningful — lets the
/// user spot dynamic content (CSRF tokens, request IDs, A/B variants) at a
/// glance without opening `--out report.json`.
fn format_bytes_cell(m: &EncodingMeasurement) -> String {
    let base = format_kb(m.bytes);
    let spread_pct = variance_pct(m.bytes, m.bytes_min, m.bytes_max);
    if spread_pct >= VARIANCE_THRESHOLD_PCT {
        format!("{base} ±{spread_pct:.1}%")
    } else {
        base
    }
}

/// Totals variance is computed from sum-of-mins / sum-of-maxes across URLs.
/// That is a conservative upper bound on real total variance (per-URL extrema
/// don't have to coincide), which is fine for a quick visual cue.
fn format_totals_cell(
    report: &Report,
    pick: impl Fn(&brogz::UrlMeasurement) -> &EncodingMeasurement,
) -> String {
    let median: u64 = report.measurements.iter().map(|m| pick(m).bytes).sum();
    let min: u64 = report.measurements.iter().map(|m| pick(m).bytes_min).sum();
    let max: u64 = report.measurements.iter().map(|m| pick(m).bytes_max).sum();

    let base = format_kb(median);
    let spread_pct = variance_pct(median, min, max);
    if spread_pct >= VARIANCE_THRESHOLD_PCT {
        format!("{base} ±{spread_pct:.1}%")
    } else {
        base
    }
}

fn variance_pct(median: u64, min: u64, max: u64) -> f64 {
    if median == 0 {
        return 0.0;
    }
    max.saturating_sub(min) as f64 / median as f64 * 100.0
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
