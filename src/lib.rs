//! brogz — measure wire bytes and timing for identity / gzip / brotli responses.
//!
//! See the crate README for a high-level overview.

pub mod aggregate;
pub mod config;
pub mod discover;
pub mod error;
pub mod measure;
pub mod orchestration;
pub mod report;

pub use aggregate::{measure_encoding, median};
pub use config::{Config, DEFAULT_RUNS};
pub use discover::{discover_urls, parse_assets};
pub use error::BrogzError;
pub use measure::{ProbeResult, build_client, probe};
pub use orchestration::{measure_url, run};
pub use report::{Encoding, EncodingMeasurement, Report, Totals, UrlMeasurement};
