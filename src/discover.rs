use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use reqwest::Client;
use url::Url;

use crate::error::BrogzError;

// Mirror of the TS `(?:src|href)=["']([^"']+\.(?:js|mjs|css))["']/gi` — the
// `(?i)` inline flag covers the `i` modifier from the original script. We
// intentionally do not allow whitespace around `=` because the TS regex did
// not either, and matching its behaviour 1:1 keeps verification trivial.
static ASSET_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:src|href)=["']([^"']+\.(?:js|mjs|css))["']"#)
        .expect("static asset regex compiles")
});

/// Extract asset paths from a rendered `index.html`.
///
/// Behaviour mirrors the original TypeScript script: `/index.html` is always
/// the first entry, external (http/https-prefixed) hrefs are skipped, and
/// duplicates are dropped while preserving first-seen order so the printed
/// table matches the legacy run.
pub fn parse_assets(html: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    let index = "/index.html".to_owned();
    seen.insert(index.clone());
    out.push(index);

    for caps in ASSET_REGEX.captures_iter(html) {
        let href = &caps[1];

        // TS used `startsWith('http')` — this also covers https, so we keep the
        // same loose check rather than parsing the URL.
        if href.starts_with("http") {
            continue;
        }

        let normalized = if href.starts_with('/') {
            href.to_owned()
        } else {
            format!("/{href}")
        };

        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }

    out
}

/// Fetch `<base>/index.html` and return the list of asset paths to measure.
///
/// The client is expected to be configured with auto-decompression disabled —
/// not strictly required for discovery (we ask for `identity`) but staying on
/// the same client keeps connection pooling effective.
pub async fn discover_urls(base: &Url, client: &Client) -> Result<Vec<String>, BrogzError> {
    let index_url = base.join("/index.html")?;

    let response = client
        .get(index_url.clone())
        .header("Accept-Encoding", "identity")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(BrogzError::MissingHtml { url: index_url.to_string() });
    }

    let html = response.text().await?;

    Ok(parse_assets(&html))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_assets_dedupes_and_keeps_order() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <link rel="stylesheet" href="/assets/app-abc123.css">
    <link rel="modulepreload" href='/assets/preload.mjs'>
    <link rel="icon" href="/favicon.ico">
    <script src="/assets/app-def456.js"></script>
    <script src='assets/relative.js'></script>
    <script src="https://cdn.example.com/external.js"></script>
    <script src="http://cdn.example.com/insecure.js"></script>
    <script src="/assets/app-def456.js"></script>
</head>
</html>"#;

        let paths = parse_assets(html);

        assert_eq!(
            paths,
            vec![
                "/index.html",
                "/assets/app-abc123.css",
                "/assets/preload.mjs",
                "/assets/app-def456.js",
                "/assets/relative.js",
            ]
        );
    }

    #[test]
    fn empty_html_still_includes_index() {
        assert_eq!(parse_assets("<html></html>"), vec!["/index.html"]);
    }

    #[test]
    fn match_is_case_insensitive() {
        let html = r#"<SCRIPT SRC="/app.JS"></SCRIPT><LINK HREF="/style.CSS">"#;

        let paths = parse_assets(html);

        assert_eq!(paths, vec!["/index.html", "/app.JS", "/style.CSS"]);
    }

    #[test]
    fn ignores_unrelated_extensions() {
        let html = r#"<link rel="icon" href="/favicon.ico">
                      <script src="/app.wasm"></script>
                      <script src="/app.js"></script>"#;

        assert_eq!(parse_assets(html), vec!["/index.html", "/app.js"]);
    }
}
