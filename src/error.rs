use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrogzError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("{url} returned non-OK status {status}")]
    NonOkStatus { url: String, status: u16 },

    #[error("inconsistent statuses for {url} ({encoding}): {statuses}")]
    InconsistentStatus {
        url: String,
        encoding: &'static str,
        statuses: String,
    },

    #[error("could not fetch index HTML at {url}")]
    MissingHtml { url: String },
}
