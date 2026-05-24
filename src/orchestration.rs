use futures::{StreamExt, stream};
use reqwest::Client;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use crate::aggregate::measure_encoding;
use crate::config::Config;
use crate::discover::discover_urls;
use crate::error::BrogzError;
use crate::measure::build_client;
use crate::report::{Encoding, Report, Totals, UrlMeasurement};

/// Run the full compression check end-to-end.
///
/// 1. Build one `reqwest::Client` (auto-decompression off; redirects+TLS per `Config`).
/// 2. Discover paths from `<base>/index.html` unless the caller supplied `config.paths`.
/// 3. For each path, measure the three encodings in parallel and collect into a `Report`.
///
/// URL-level parallelism uses `buffered` (not `buffer_unordered`) so that the
/// `measurements` array preserves discovery order — that is what makes
/// `jq -S` diffs against historical reports trivially empty.
pub async fn run(config: Config) -> Result<Report, BrogzError> {
    let client = build_client(config.insecure)?;

    let paths = match config.paths.clone() {
        Some(paths) => paths,
        None => discover_urls(&config.base_url, &client).await?,
    };

    let concurrency = config.concurrency.max(1);
    let runs = config.runs;
    let base_url = config.base_url.clone();
    let client_for_stream = client.clone();

    let measurements: Vec<UrlMeasurement> = stream::iter(paths)
        .map(|path| {
            let base = base_url.clone();
            let client = client_for_stream.clone();
            async move { measure_url(&base, &path, runs, concurrency, &client).await }
        })
        .buffered(concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let totals = Totals {
        identity: measurements.iter().map(|m| m.identity.bytes).sum(),
        gzip: measurements.iter().map(|m| m.gzip.bytes).sum(),
        br: measurements.iter().map(|m| m.br.bytes).sum(),
    };

    Ok(Report {
        base_url: trim_trailing_slash(config.base_url.as_str()),
        runs: config.runs,
        generated_at: now_iso_utc(),
        measurements,
        totals,
    })
}

/// Measure all three encodings for one URL in parallel.
///
/// The three encodings run concurrently via `try_join!` (paritetic with the
/// original TS `Promise.all`); inside each, `measure_encoding` does its own
/// `runs`-wide parallelism per the configured concurrency.
pub async fn measure_url(
    base: &Url,
    path: &str,
    runs: usize,
    concurrency: usize,
    client: &Client,
) -> Result<UrlMeasurement, BrogzError> {
    let url = base.join(path)?;

    let (identity, gzip, br) = tokio::try_join!(
        measure_encoding(client, &url, Encoding::Identity, runs, concurrency),
        measure_encoding(client, &url, Encoding::Gzip, runs, concurrency),
        measure_encoding(client, &url, Encoding::Br, runs, concurrency),
    )?;

    Ok(UrlMeasurement { path: path.to_owned(), identity, gzip, br })
}

fn now_iso_utc() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

// `Url::to_string()` always adds a trailing slash to root paths
// (`https://x` -> `https://x/`). The original TS report stored the value the
// user typed sans trailing slash — keep that for diff stability.
fn trim_trailing_slash(s: &str) -> String {
    s.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn run_end_to_end_with_explicit_paths() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_matcher("/index.html"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"<html></html>"[..]))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path_matcher("/app.js"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .set_body_bytes(&b"x".repeat(128)[..]),
            )
            .mount(&server)
            .await;

        let config = Config {
            base_url: Url::parse(&server.uri()).unwrap(),
            runs: 2,
            concurrency: 4,
            insecure: false,
            paths: Some(vec!["/index.html".to_owned(), "/app.js".to_owned()]),
        };

        let report = run(config).await.unwrap();

        assert_eq!(report.measurements.len(), 2);
        assert_eq!(report.measurements[0].path, "/index.html");
        assert_eq!(report.measurements[1].path, "/app.js");
        assert_eq!(report.measurements[0].identity.bytes, 13);
        assert_eq!(report.measurements[1].gzip.bytes, 128);
        assert_eq!(report.measurements[1].gzip.content_encoding, "gzip");
        assert_eq!(report.totals.identity, 13 + 128);
        assert_eq!(report.totals.gzip, 13 + 128);
        assert_eq!(report.totals.br, 13 + 128);
        assert_eq!(report.runs, 2);
        assert!(!report.generated_at.is_empty());
        assert!(!report.base_url.ends_with('/'));
    }

    #[test]
    fn trim_trailing_slash_preserves_path() {
        assert_eq!(trim_trailing_slash("https://app.example/"), "https://app.example");
        assert_eq!(trim_trailing_slash("https://app.example"), "https://app.example");
        assert_eq!(trim_trailing_slash("https://app.example/foo/"), "https://app.example/foo");
    }
}
