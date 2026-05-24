use brogz::Config;
use url::Url;
use wiremock::matchers::{header, method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

const INDEX_HTML: &[u8] = br#"<!DOCTYPE html>
<html>
<head>
    <link rel="stylesheet" href="/assets/app.css">
    <script src="/assets/app.js"></script>
</head>
<body></body>
</html>"#;

/// End-to-end: real discovery from /index.html, three encodings × two assets,
/// each asset returns a different byte count depending on Accept-Encoding so we
/// can verify the runner attributes the right wire bytes to the right encoding.
#[tokio::test]
async fn full_pipeline_discovers_and_measures_three_encodings() {
    let server = MockServer::start().await;

    // /index.html — discovery sends Accept-Encoding: identity, and the
    // subsequent measurement passes also need it on gzip/br paths.
    // A single Mock without a header matcher answers them all.
    Mock::given(method("GET"))
        .and(path_matcher("/index.html"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(INDEX_HTML))
        .mount(&server)
        .await;

    register_asset(&server, "/assets/app.js", 1000, 400, 350).await;
    register_asset(&server, "/assets/app.css", 500, 200, 180).await;

    let config = Config {
        base_url: Url::parse(&server.uri()).unwrap(),
        runs: 2,
        concurrency: 6,
        insecure: false,
        paths: None,
        progress: None,
    };

    let report = brogz::run(config).await.unwrap();

    // Discovery order — index always first, then HTML appearance order.
    let paths: Vec<&str> = report
        .measurements
        .iter()
        .map(|m| m.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["/index.html", "/assets/app.css", "/assets/app.js"]
    );

    let index = &report.measurements[0];
    assert_eq!(index.identity.bytes, INDEX_HTML.len() as u64);
    assert_eq!(index.identity.content_encoding, "identity");
    assert_eq!(index.gzip.bytes, INDEX_HTML.len() as u64);
    assert_eq!(index.br.bytes, INDEX_HTML.len() as u64);

    let css = &report.measurements[1];
    assert_eq!(css.identity.bytes, 500);
    assert_eq!(css.identity.content_encoding, "identity");
    assert_eq!(css.gzip.bytes, 200);
    assert_eq!(css.gzip.content_encoding, "gzip");
    assert_eq!(css.br.bytes, 180);
    assert_eq!(css.br.content_encoding, "br");

    let js = &report.measurements[2];
    assert_eq!(js.identity.bytes, 1000);
    assert_eq!(js.gzip.bytes, 400);
    assert_eq!(js.gzip.content_encoding, "gzip");
    assert_eq!(js.br.bytes, 350);
    assert_eq!(js.br.content_encoding, "br");

    let index_bytes = INDEX_HTML.len() as u64;
    assert_eq!(report.totals.identity, index_bytes + 500 + 1000);
    assert_eq!(report.totals.gzip, index_bytes + 200 + 400);
    assert_eq!(report.totals.br, index_bytes + 180 + 350);

    assert_eq!(report.runs, 2);
    assert!(report.base_url.starts_with("http://"));
    assert!(!report.base_url.ends_with('/'));
    assert!(!report.generated_at.is_empty());

    // JSON round-trip — what we hand the user must deserialize back into the
    // same shape, otherwise downstream `jq` diffs would silently rot.
    let json = serde_json::to_string_pretty(&report).unwrap();
    let parsed: brogz::Report = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.measurements.len(), 3);
    assert_eq!(parsed.totals.br, report.totals.br);
}

/// Register three responses for one path — identity / gzip / br — with the
/// given (fake) wire byte counts and matching Content-Encoding headers.
/// The bodies are deliberately not actually compressed; we only check that
/// brogz reports the size and CE the server advertised, since reqwest is
/// configured to never auto-decompress.
async fn register_asset(server: &MockServer, path: &str, raw: usize, gz: usize, br: usize) {
    for (encoding, size) in [("identity", raw), ("gzip", gz), ("br", br)] {
        Mock::given(method("GET"))
            .and(path_matcher(path))
            .and(header("accept-encoding", encoding))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", encoding)
                    .set_body_bytes(vec![b'x'; size]),
            )
            .mount(server)
            .await;
    }
}

/// Discovery failure (non-2xx on /index.html) surfaces as `MissingHtml`,
/// not a transport error — keeps the error message actionable.
#[tokio::test]
async fn missing_index_html_returns_actionable_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path_matcher("/index.html"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let config = Config {
        base_url: Url::parse(&server.uri()).unwrap(),
        runs: 1,
        concurrency: 1,
        insecure: false,
        paths: None,
        progress: None,
    };

    let err = brogz::run(config).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("index"), "unexpected error: {msg}");
}
