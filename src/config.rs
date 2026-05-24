use url::Url;

pub const DEFAULT_RUNS: usize = 10;

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: Url,
    pub runs: usize,
    pub concurrency: usize,
    pub insecure: bool,
    /// `None` triggers asset discovery from `/index.html`.
    pub paths: Option<Vec<String>>,
}

impl Config {
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            runs: DEFAULT_RUNS,
            concurrency: DEFAULT_RUNS * 3,
            insecure: false,
            paths: None,
        }
    }
}
