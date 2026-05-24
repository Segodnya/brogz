# brogz

[![CI](https://github.com/Segodnya/brogz/actions/workflows/ci.yml/badge.svg)](https://github.com/Segodnya/brogz/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Measure wire bytes and timing for `identity`, `gzip`, and `brotli` responses
across a site's static assets. One static binary — no Node, no `curl` in your
`PATH`.

> `br` + `gz` — the two encodings we compare.

```
┌─────────────────────────────────────────┬──────────┬─────────┬───────┬─────────┬───────┬──────────┐
│ path                                    │ raw      │ gzip    │ gz CE │ br      │ br CE │ br vs gz │
╞═════════════════════════════════════════╪══════════╪═════════╪═══════╪═════════╪═══════╪══════════╡
│ /index.html                             │  38.59KB │  9.89KB │ gzip  │  7.85KB │ br    │   -20.6% │
│ /assets/index-yrrrWp82.js               │  17.19KB │  6.75KB │ gzip  │  5.88KB │ br    │   -12.9% │
│ /assets/vendor-DCLJsiZC.js              │ 185.20KB │ 57.48KB │ gzip  │ 49.58KB │ br    │   -13.7% │
│ Σ TOTAL                                 │ 306.95KB │ 96.27KB │       │ 82.96KB │       │   -13.8% │
└─────────────────────────────────────────┴──────────┴─────────┴───────┴─────────┴───────┴──────────┘
```

## Install

```bash
# Homebrew (macOS, Linux)
brew tap Segodnya/brogz
brew install brogz

# crates.io (anywhere with a Rust toolchain)
cargo install brogz

# or grab a prebuilt binary from the latest GitHub Release:
# https://github.com/Segodnya/brogz/releases
```

Prebuilt targets: `x86_64`/`aarch64-unknown-linux-gnu`, `x86_64`/`aarch64-apple-darwin`,
`x86_64-pc-windows-msvc`.

The Homebrew tap lives at [Segodnya/homebrew-brogz](https://github.com/Segodnya/homebrew-brogz)
and is bumped automatically on every new release.

## Usage

```text
brogz <URL> [OPTIONS]

Arguments:
  <URL>                       Base URL, e.g. https://app.example

Options:
  -r, --runs <N>              Probes per URL × encoding [default: 10]
  -c, --concurrency <N>       Max parallel requests [default: 3 * runs]
  -k, --insecure              Skip TLS verification
  -o, --out <FILE>            Write JSON report to file (in addition to stdout table)
  -p, --path <PATH>           Skip discovery; measure these paths instead (repeatable)
      --no-color              Disable colored output
  -q, --quiet                 Suppress progress lines on stderr
  -h, --help, -V, --version
```

Common invocations:

```bash
# Auto-discover assets from /index.html, default 10 probes per URL × encoding.
brogz https://app.example

# More probes for tighter medians; default concurrency follows runs.
brogz https://app.example --runs 20

# Save the JSON report alongside the stdout table.
brogz https://app.example -o report.json

# Skip discovery entirely — measure exactly these paths.
brogz https://app.example -p /index.html -p /assets/app.js -p /assets/app.css

# Dev environments behind an internal CA.
brogz https://scheduling-dev.example -k
```

### What it measures

For every URL × encoding pair, brogz sends `--runs` parallel HTTP requests
with the matching `Accept-Encoding` header, records the *wire* byte count and
total wall-clock time, and reports:

| field             | meaning                                                          |
|-------------------|------------------------------------------------------------------|
| `bytes`           | median wire body length across probes (not decompressed)         |
| `bytesMin`        | smallest wire length seen across probes                          |
| `bytesMax`        | largest wire length seen across probes                           |
| `contentEncoding` | `Content-Encoding` header from the final response                |
| `medianMs`        | median of per-probe wall-clock time (`Math.round`-compatible)    |

On a static asset all probes agree, so `bytes == bytesMin == bytesMax`. Public
HTML (CSRF tokens, request IDs, A/B variants) produces slightly different
bodies per request — the median is representative, and `bytesMin`/`bytesMax`
expose the spread. `Content-Encoding` mismatches across probes log a `warn!`
to stderr; status mismatches or non-200 responses abort the URL.

### Output

A unicode table (with optional colors) to stdout. The full report — including
per-probe medians and totals — is written to JSON when `--out` is set:

```json
{
  "baseUrl": "https://app.example",
  "runs": 10,
  "generatedAt": "2026-05-24T10:30:45Z",
  "measurements": [
    {
      "path": "/index.html",
      "identity": { "bytes": 39516, "bytesMin": 39516, "bytesMax": 39516, "contentEncoding": "identity", "medianMs": 28 },
      "gzip":     { "bytes": 10128, "bytesMin": 10112, "bytesMax": 10145, "contentEncoding": "gzip",     "medianMs": 30 },
      "br":       { "bytes":  8037, "bytesMin":  8001, "bytesMax":  8062, "contentEncoding": "br",       "medianMs": 32 }
    }
  ],
  "totals": { "identity": 314313, "gzip": 98581, "br": 84953 }
}
```

The JSON shape is pinned by a unit test so historical reports can be diffed
with `jq -S` across versions without spurious churn.

## Library usage

The crate exposes everything the CLI uses, behind a feature-gated split so
library consumers do not pull in clap / comfy-table / crossterm:

```toml
[dependencies]
brogz = { version = "0.1", default-features = false }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
url   = "2"
```

```rust
use brogz::Config;
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = brogz::run(Config {
        base_url: Url::parse("https://app.example")?,
        runs: 10,
        concurrency: 30,
        insecure: false,
        paths: None, // None -> discover from /index.html
        progress: None, // or Some(Arc::new(|e| { ... })) to receive ProgressEvent
    }).await?;

    println!("br savings vs gzip: {} -> {} bytes",
        report.totals.gzip, report.totals.br);
    Ok(())
}
```

The lower-level building blocks are also public: `discover_urls`,
`measure_url`, `measure_encoding`, `probe`, `build_client`, `median`.

## Exit codes

| code | meaning                                                  |
|------|----------------------------------------------------------|
| 0    | success                                                  |
| 1    | runtime error (transport, non-200, inconsistent status)  |
| 2    | invalid CLI arguments                                    |

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual-licensed as above, without any additional terms or
conditions.
