//! Conservative asserted string formats for offline qualification, not annotations.
use anyhow::{Result, ensure};
use regex::Regex;
use std::sync::LazyLock;
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[0-9]{4}-[0-9]{2}-[0-9]{2}\z").unwrap());
static DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A[0-9]{4}-[0-9]{2}-[0-9]{2}[Tt][0-9]{2}:[0-9]{2}:[0-5][0-9](?:\.[0-9]+)?(?:[Zz]|[+-][0-9]{2}:[0-9]{2})\z").unwrap()
});
// RFC 3986 scheme + authority/path/query/fragment character grammar. Url then
// validates authority syntax; no decoding, normalization or network I/O is used
// to change the submitted value. Reject IRIs and relative references explicitly.
static URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A[A-Za-z][A-Za-z0-9+.-]*:(?://(?:[A-Za-z0-9._~!$&'()*+,;=:@\[\]-]|%[0-9A-Fa-f]{2})*)?(?:[A-Za-z0-9._~!$&'()*+,;=:@/-]|%[0-9A-Fa-f]{2})*(?:\?(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[0-9A-Fa-f]{2})*)?(?:#(?:[A-Za-z0-9._~!$&'()*+,;=:@/?-]|%[0-9A-Fa-f]{2})*)?\z").unwrap()
});
pub fn supported(format: &str) -> Result<()> {
    ensure!(
        matches!(format, "date" | "date-time" | "uri"),
        "unsupported schema string format; cannot certify arguments"
    );
    Ok(())
}
fn uri(text: &str) -> bool {
    if !URI.is_match(text) || reqwest::Url::parse(text).is_err() {
        return false;
    }
    let rest = text.split_once(':').expect("matched URI scheme").1;
    if let Some(authority) = rest.strip_prefix("//") {
        let authority = authority.split(['/', '?', '#']).next().unwrap();
        // URL parsers may repair repeated @ or brackets in userinfo. Qualification
        // must reject those raw characters instead of accepting the repaired URL.
        if authority.matches('@').count() > 1 {
            return false;
        }
        if let Some((userinfo, _)) = authority.split_once('@')
            && userinfo.contains(['[', ']'])
        {
            return false;
        }
    }
    true
}
pub fn check(format: &str, text: &str) -> Result<()> {
    supported(format)?;
    let valid = match format {
        "date" => {
            DATE.is_match(text) && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
        }
        "date-time" => {
            DATETIME.is_match(text) && chrono::DateTime::parse_from_rfc3339(text).is_ok()
        }
        "uri" => uri(text),
        _ => false,
    };
    ensure!(valid, "argument string format mismatch ({format})");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_format_values_keep_optional_schema_fields_usable() {
        for (format, good, bad) in [
            (
                "date",
                vec!["2024-02-29", "2026-01-01"],
                vec!["2025-02-29", "2026-1-01", "2026-01-01\n"],
            ),
            (
                "date-time",
                vec!["2026-01-01T12:30:45Z", "2026-01-01t12:30:45.123+01:00"],
                vec![
                    "2026-01-01 12:30:45Z",
                    "2026-01-01T12:30:45",
                    "2026-01-01T25:00:00Z",
                    "2026-01-01T00:00:60Z",
                ],
            ),
            (
                "uri",
                vec![
                    "https://example.com/a%20b?q=x#section",
                    "urn:isbn:9780000000000",
                    "https://[::1]/",
                ],
                vec![
                    "/relative",
                    "https://example.com/a b",
                    "https://example.com/%xx",
                    "https://example.com/雪",
                    "https://example.com/a#b#c",
                    "https://example.com/[bad]",
                    "https://example.com\\bad",
                    "https://a@@example.com/",
                    "https://a[b]@example.com/",
                ],
            ),
        ] {
            for text in good {
                assert!(check(format, text).is_ok(), "{format}: {text}");
            }
            for text in bad {
                assert!(check(format, text).is_err(), "{format}: {text}");
            }
        }
    }
}
