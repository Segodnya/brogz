use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    Identity,
    Gzip,
    Br,
}

impl Encoding {
    /// Value to send in the `Accept-Encoding` request header.
    pub const fn header_value(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Br => "br",
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.header_value()
    }
}

impl std::fmt::Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingMeasurement {
    pub bytes: u64,
    /// Raw `Content-Encoding` header value from the final response — kept as a
    /// `String` so unexpected values (e.g. `zstd`, server-side typos) survive
    /// the round-trip intact.
    pub content_encoding: String,
    pub median_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlMeasurement {
    pub path: String,
    pub identity: EncodingMeasurement,
    pub gzip: EncodingMeasurement,
    pub br: EncodingMeasurement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub identity: u64,
    pub gzip: u64,
    pub br: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub base_url: String,
    pub runs: usize,
    pub generated_at: String,
    pub measurements: Vec<UrlMeasurement>,
    pub totals: Totals,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pins the JSON shape against the original TypeScript model so that old
    // reports stay readable and `jq -S` diffs against legacy fixtures remain
    // empty. If this test breaks, the on-wire format changed — bump the report
    // schema version intentionally.
    #[test]
    fn report_json_matches_typescript_shape() {
        let report = Report {
            base_url: "https://app.example".to_owned(),
            runs: 10,
            generated_at: "2026-05-24T00:00:00.000Z".to_owned(),
            measurements: vec![UrlMeasurement {
                path: "/index.html".to_owned(),
                identity: EncodingMeasurement {
                    bytes: 1024,
                    content_encoding: "identity".to_owned(),
                    median_ms: 12,
                },
                gzip: EncodingMeasurement {
                    bytes: 512,
                    content_encoding: "gzip".to_owned(),
                    median_ms: 14,
                },
                br: EncodingMeasurement {
                    bytes: 400,
                    content_encoding: "br".to_owned(),
                    median_ms: 15,
                },
            }],
            totals: Totals { identity: 1024, gzip: 512, br: 400 },
        };

        let json = serde_json::to_value(&report).unwrap();
        let expected = serde_json::json!({
            "baseUrl": "https://app.example",
            "runs": 10,
            "generatedAt": "2026-05-24T00:00:00.000Z",
            "measurements": [{
                "path": "/index.html",
                "identity": { "bytes": 1024, "contentEncoding": "identity", "medianMs": 12 },
                "gzip":     { "bytes": 512,  "contentEncoding": "gzip",     "medianMs": 14 },
                "br":       { "bytes": 400,  "contentEncoding": "br",       "medianMs": 15 },
            }],
            "totals": { "identity": 1024, "gzip": 512, "br": 400 },
        });

        assert_eq!(json, expected);

        // Round-trip — deserializing our own output must succeed.
        let _: Report = serde_json::from_value(json).unwrap();
    }

    #[test]
    fn encoding_serializes_as_lowercase() {
        assert_eq!(serde_json::to_string(&Encoding::Identity).unwrap(), "\"identity\"");
        assert_eq!(serde_json::to_string(&Encoding::Gzip).unwrap(), "\"gzip\"");
        assert_eq!(serde_json::to_string(&Encoding::Br).unwrap(), "\"br\"");
    }
}
