//! Progress events emitted by `orchestration::run` for live status display.
//!
//! The library is IO-agnostic — it does not print anything itself. A consumer
//! that wants progress wires a `ProgressCallback` into `Config::progress`; the
//! CLI binary does exactly that. Library users can ignore the field entirely.

use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Fired once after URL discovery (or immediately, if paths were supplied
    /// via `Config::paths`). `url_count * probes_per_url` is the total number
    /// of HTTP probes the run will issue.
    Discovered {
        url_count: usize,
        probes_per_url: usize,
    },
    /// Fired once per URL after its three-encoding measurement succeeds.
    /// Not fired on error — the error itself propagates out of `run`.
    UrlCompleted { path: String },
}

/// Synchronous sink invoked from inside measurement tasks. Keep the
/// implementation cheap (it runs on a runtime worker) — printing, atomic
/// counters, channel sends all fine; blocking IO or long computations are not.
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;
