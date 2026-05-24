use std::collections::HashSet;

use futures::{StreamExt, stream};
use reqwest::Client;
use url::Url;

use crate::error::BrogzError;
use crate::measure::probe;
use crate::report::{Encoding, EncodingMeasurement};

/// Median of integer samples with the same rounding semantics as the TypeScript
/// `Math.round((a + b) / 2)` for the even-length case (round half-up for
/// non-negative averages — `(a + b + 1) / 2` integer-divided).
///
/// Returns 0 for empty input; the caller should never reach this path because
/// `measure_encoding` rejects `runs == 0` upstream, but we stay total here.
pub fn median(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let mut sorted: Vec<u64> = values.to_vec();
    sorted.sort_unstable();

    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid] + 1) / 2
    } else {
        sorted[mid]
    }
}

/// Run `runs` probes against one `(url, encoding)` pair in parallel (capped at
/// `concurrency`), then collapse them into an `EncodingMeasurement`.
///
/// * **Status consistency is an error.** If any probe returns non-200 or the
///   probes disagree on status, we abort the URL — measuring "bytes" across a
///   200 and a 404 would be meaningless.
/// * **Size and Content-Encoding mismatches are warnings.** A flaky CDN that
///   occasionally negotiates a different encoding still produces a usable
///   median; we surface the noise via `tracing::warn!` and pick the first
///   sample's values for the report.
pub async fn measure_encoding(
    client: &Client,
    url: &Url,
    encoding: Encoding,
    runs: usize,
    concurrency: usize,
) -> Result<EncodingMeasurement, BrogzError> {
    let results = stream::iter(0..runs)
        .map(|_| probe(client, url, encoding))
        .buffer_unordered(concurrency.max(1))
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    if results.is_empty() {
        return Err(BrogzError::InconsistentStatus {
            url: url.to_string(),
            encoding: encoding.as_str(),
            statuses: "no probes ran (runs == 0)".to_owned(),
        });
    }

    let statuses: HashSet<u16> = results.iter().map(|r| r.status).collect();
    if statuses.len() > 1 || !statuses.contains(&200) {
        return Err(BrogzError::InconsistentStatus {
            url: url.to_string(),
            encoding: encoding.as_str(),
            statuses: join_sorted(statuses),
        });
    }

    let sizes: HashSet<u64> = results.iter().map(|r| r.bytes).collect();
    if sizes.len() > 1 {
        tracing::warn!(
            url = %url,
            encoding = %encoding,
            sizes = %join_sorted(sizes),
            "non-deterministic byte count across probes"
        );
    }

    let content_encodings: HashSet<String> =
        results.iter().map(|r| r.content_encoding.clone()).collect();
    if content_encodings.len() > 1 {
        let mut sorted: Vec<_> = content_encodings.into_iter().collect();
        sorted.sort();
        tracing::warn!(
            url = %url,
            requested = %encoding,
            content_encodings = %sorted.join(","),
            "inconsistent Content-Encoding across probes"
        );
    }

    let times_ms: Vec<u64> = results
        .iter()
        .map(|r| (r.elapsed.as_secs_f64() * 1000.0).round() as u64)
        .collect();

    Ok(EncodingMeasurement {
        bytes: results[0].bytes,
        content_encoding: results[0].content_encoding.clone(),
        median_ms: median(&times_ms),
    })
}

fn join_sorted<T: ToString + Ord>(set: HashSet<T>) -> String {
    let mut v: Vec<T> = set.into_iter().collect();
    v.sort();
    v.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::build_client;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn median_odd_length() {
        assert_eq!(median(&[10]), 10);
        assert_eq!(median(&[3, 1, 2]), 2);
        assert_eq!(median(&[100, 50, 200, 1, 75]), 75);
    }

    #[test]
    fn median_even_length_rounds_half_up() {
        // Mirrors Math.round((a + b) / 2) for positive halves.
        assert_eq!(median(&[1, 2, 3, 4]), 3); // (2 + 3) / 2 = 2.5 -> 3
        assert_eq!(median(&[10, 20]), 15); // exact
        assert_eq!(median(&[1, 4]), 3); // 2.5 -> 3
        assert_eq!(median(&[1, 2, 3, 6]), 3); // (2 + 3) / 2 = 2.5 -> 3
    }

    #[test]
    fn median_empty_is_zero() {
        assert_eq!(median(&[]), 0);
    }

    #[tokio::test]
    async fn measure_encoding_aggregates_consistent_probes() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/asset.js"))
            .and(header("accept-encoding", "gzip"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .set_body_bytes(&b"x".repeat(256)[..]),
            )
            .expect(5)
            .mount(&server)
            .await;

        let client = build_client(false).unwrap();
        let url = Url::parse(&format!("{}/asset.js", server.uri())).unwrap();

        let measurement = measure_encoding(&client, &url, Encoding::Gzip, 5, 5)
            .await
            .unwrap();

        assert_eq!(measurement.bytes, 256);
        assert_eq!(measurement.content_encoding, "gzip");
        // `median_ms` is timing-dependent; just check it's non-negative range.
    }

    #[tokio::test]
    async fn measure_encoding_errors_on_non_200() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/missing.js"))
            .respond_with(ResponseTemplate::new(404).set_body_bytes(&b"nope"[..]))
            .mount(&server)
            .await;

        let client = build_client(false).unwrap();
        let url = Url::parse(&format!("{}/missing.js", server.uri())).unwrap();

        let err = measure_encoding(&client, &url, Encoding::Identity, 3, 3)
            .await
            .unwrap_err();

        match err {
            BrogzError::InconsistentStatus { statuses, .. } => {
                assert_eq!(statuses, "404");
            }
            other => panic!("expected InconsistentStatus, got {other:?}"),
        }
    }
}
