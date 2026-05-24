//! brogz — measure wire bytes and timing for identity / gzip / brotli responses.
//!
//! See the crate README for a high-level overview.

pub mod config;
pub mod error;
pub mod report;
// pub mod discover;
// pub mod measure;
// pub mod aggregate;

pub use config::{Config, DEFAULT_RUNS};
pub use error::BrogzError;
pub use report::{Encoding, EncodingMeasurement, Report, Totals, UrlMeasurement};
