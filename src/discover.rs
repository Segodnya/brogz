use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use reqwest::Client;
use url::Url;

use crate::error::BrogzError;
use crate::orchestration::join_under_base;

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
/// `/index.html` is always the first entry. Every `href` is resolved against
/// `base` so root-relative (`/foo.js`), protocol-relative (`//cdn/foo.js`),
/// path-relative (`foo.js`), and fully-qualified URLs go through one common
/// origin check — references on a different origin are dropped (a different
/// origin means a separate host that the user did not ask us to measure).
/// Duplicates are dropped while preserving first-seen order so the printed
/// table matches the legacy run.
pub fn parse_assets(html: &str, base: &Url) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    let index = "/index.html".to_owned();
    seen.insert(index.clone());
    out.push(index);

    let base_origin = base.origin();

    for caps in ASSET_REGEX.captures_iter(html) {
        let href = &caps[1];

        // `Url::join` follows WHATWG resolution semantics, so all four
        // common forms — root-relative, protocol-relative, fully-qualified,
        // path-relative — funnel through one cross-origin check below.
        let Ok(resolved) = base.join(href) else {
            continue;
        };

        if resolved.origin() != base_origin {
            continue;
        }

        // Path + query is what identifies an asset on the wire; fragment is
        // never sent in an HTTP request, drop it.
        let path = match resolved.query() {
            Some(q) => format!("{}?{}", resolved.path(), q),
            None => resolved.path().to_owned(),
        };

        if seen.insert(path.clone()) {
            out.push(path);
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
    let index_url = join_under_base(base, "/index.html")?;

    let response = client
        .get(index_url.clone())
        .header("Accept-Encoding", "identity")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(BrogzError::MissingHtml {
            url: index_url.to_string(),
        });
    }

    let html = response.text().await?;

    Ok(parse_assets(&html, base))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.test").unwrap()
    }

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

        let paths = parse_assets(html, &base());

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
        assert_eq!(parse_assets("<html></html>", &base()), vec!["/index.html"]);
    }

    #[test]
    fn match_is_case_insensitive() {
        let html = r#"<SCRIPT SRC="/app.JS"></SCRIPT><LINK HREF="/style.CSS">"#;

        let paths = parse_assets(html, &base());

        assert_eq!(paths, vec!["/index.html", "/app.JS", "/style.CSS"]);
    }

    #[test]
    fn skips_protocol_relative_cross_host_assets() {
        // Real-world failure mode: some.site serves all its assets from
        // `//cdn.some.site/...`. Without this filter we would resolve them
        // against `https://some.site` and request `https://some.site//cdn...`,
        // which 404s and aborts the run.
        let html = r#"<link href="//cdn.some.site/frontend/dist/Main.css">
                      <script src="//cdn.example.com/sdk.js"></script>
                      <script src="/assets/local.js"></script>"#;

        assert_eq!(
            parse_assets(html, &Url::parse("https://some.site").unwrap()),
            vec!["/index.html", "/assets/local.js"]
        );
    }

    #[test]
    fn fully_qualified_same_origin_is_kept_as_path() {
        // A self-href written out in full (`https://app.example/foo.js`) should
        // come back as a root-relative path, not be dropped as "external".
        // The old prefix-based filter missed this — only the resolve+origin
        // approach handles it cleanly.
        let html = r#"<script src="https://app.example/assets/main.js"></script>
                      <link href="https://app.example/style.css">
                      <script src="https://app.example:8443/portbump.js"></script>"#;

        assert_eq!(
            parse_assets(html, &Url::parse("https://app.example").unwrap()),
            vec!["/index.html", "/assets/main.js", "/style.css"]
        );
    }

    #[test]
    fn cross_scheme_is_treated_as_cross_origin() {
        // Per WHATWG, `http://host` and `https://host` are different origins.
        // We refuse to silently downgrade — keeps the report honest about
        // what was actually fetched.
        let html = r#"<script src="http://app.example/insecure.js"></script>"#;

        assert_eq!(
            parse_assets(html, &Url::parse("https://app.example").unwrap()),
            vec!["/index.html"]
        );
    }

    #[test]
    fn ignores_unrelated_extensions() {
        let html = r#"<link rel="icon" href="/favicon.ico">
                      <script src="/app.wasm"></script>
                      <script src="/app.js"></script>"#;

        assert_eq!(parse_assets(html, &base()), vec!["/index.html", "/app.js"]);
    }
}
