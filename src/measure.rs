use std::time::{Duration, Instant};

use reqwest::{Client, redirect};
use url::Url;

use crate::error::BrogzError;
use crate::report::Encoding;

/// Result of one HTTP probe — wire bytes, wall-clock time to drain the body,
/// the final `Content-Encoding` (defaulted to `"identity"` when absent), and
/// the final HTTP status. Consistency checks live in `aggregate.rs`.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub bytes: u64,
    pub elapsed: Duration,
    pub content_encoding: String,
    pub status: u16,
}

/// Build a `reqwest::Client` matching the original `curl` invocation:
///
/// * follow up to 10 redirects (parity with `curl -L`)
/// * `--insecure` toggles `danger_accept_invalid_certs`
/// * the `gzip`/`brotli`/`deflate` features are intentionally **not** enabled
///   on the `reqwest` dependency — without them hyper never auto-decompresses,
///   so `response.bytes().len()` reflects what actually went over the wire.
///   That is the whole point of this tool.
pub fn build_client(insecure: bool) -> Result<Client, BrogzError> {
    let client = Client::builder()
        .redirect(redirect::Policy::limited(10))
        .danger_accept_invalid_certs(insecure)
        .build()?;

    Ok(client)
}

/// Issue a single GET with `Accept-Encoding: <encoding>`, measure wall-clock
/// from request send through body drain, and return raw byte count.
///
/// Errors only on transport failure — non-2xx statuses are preserved in the
/// returned `ProbeResult` so the aggregator can collect all replies before
/// deciding whether the sample is consistent.
pub async fn probe(
    client: &Client,
    url: &Url,
    encoding: Encoding,
) -> Result<ProbeResult, BrogzError> {
    let start = Instant::now();

    let response = client
        .get(url.clone())
        .header("Accept-Encoding", encoding.header_value())
        .send()
        .await?;

    let status = response.status().as_u16();

    // Final-response headers — reqwest already followed redirects, so these
    // belong to the response whose body we are about to consume.
    let content_encoding = response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "identity".to_owned());

    // Drain the body — measuring elapsed *after* this is critical, otherwise
    // we would only time TTFB and miss transfer cost.
    let bytes = response.bytes().await?.len() as u64;

    let elapsed = start.elapsed();

    Ok(ProbeResult {
        bytes,
        elapsed,
        content_encoding,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The load-bearing invariant: when the server advertises a compressed
    /// `Content-Encoding`, we must report the *wire* byte count, not whatever
    /// hyper would have decompressed it to. The body deliberately is not valid
    /// brotli — if auto-decompression ever sneaks back in, the body drain will
    /// either error or produce a wildly different length and this test fails.
    #[tokio::test]
    async fn probe_reports_raw_bytes_when_server_claims_br() {
        let server = MockServer::start().await;

        // 32 bytes of arbitrary non-brotli payload.
        let body: &[u8] = b"absolutely-not-brotli-bytes-xxxx";
        assert_eq!(body.len(), 32);

        Mock::given(method("GET"))
            .and(path("/asset.js"))
            .and(header("accept-encoding", "br"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "br")
                    .set_body_bytes(body),
            )
            .mount(&server)
            .await;

        let client = build_client(false).unwrap();
        let url = Url::parse(&format!("{}/asset.js", server.uri())).unwrap();

        let result = probe(&client, &url, Encoding::Br).await.unwrap();

        assert_eq!(result.bytes, 32);
        assert_eq!(result.content_encoding, "br");
        assert_eq!(result.status, 200);
    }

    #[tokio::test]
    async fn probe_defaults_missing_content_encoding_to_identity() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/raw.html"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"hello"[..]))
            .mount(&server)
            .await;

        let client = build_client(false).unwrap();
        let url = Url::parse(&format!("{}/raw.html", server.uri())).unwrap();

        let result = probe(&client, &url, Encoding::Identity).await.unwrap();

        assert_eq!(result.bytes, 5);
        assert_eq!(result.content_encoding, "identity");
        assert_eq!(result.status, 200);
    }

    #[tokio::test]
    async fn probe_returns_non_ok_status_without_erroring() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/missing.js"))
            .respond_with(ResponseTemplate::new(404).set_body_bytes(&b"not found"[..]))
            .mount(&server)
            .await;

        let client = build_client(false).unwrap();
        let url = Url::parse(&format!("{}/missing.js", server.uri())).unwrap();

        let result = probe(&client, &url, Encoding::Gzip).await.unwrap();

        assert_eq!(result.status, 404);
        assert_eq!(result.bytes, 9);
    }
}
