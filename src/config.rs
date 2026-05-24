use url::Url;

use crate::progress::ProgressCallback;

pub const DEFAULT_RUNS: usize = 10;

#[derive(Clone)]
pub struct Config {
    pub base_url: Url,
    pub runs: usize,
    pub concurrency: usize,
    pub insecure: bool,
    /// `None` triggers asset discovery from `/index.html`.
    pub paths: Option<Vec<String>>,
    /// Optional sink for `ProgressEvent`s. `None` disables progress reporting.
    pub progress: Option<ProgressCallback>,
}

impl Config {
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            runs: DEFAULT_RUNS,
            concurrency: DEFAULT_RUNS * 3,
            insecure: false,
            paths: None,
            progress: None,
        }
    }
}

// Derived manually because `ProgressCallback` is a trait object that does not
// implement `Debug`. Skipping the field keeps `{:?}` formatting available on
// `Config` for callers that log it.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("base_url", &self.base_url)
            .field("runs", &self.runs)
            .field("concurrency", &self.concurrency)
            .field("insecure", &self.insecure)
            .field("paths", &self.paths)
            .field("progress", &self.progress.as_ref().map(|_| "<callback>"))
            .finish()
    }
}
