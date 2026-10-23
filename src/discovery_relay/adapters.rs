//! Vendor envelope translation only. Callers own status acceptance and payment authority.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Method, Url, header::HeaderMap};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Curl,
    Agproxy,
    Horizonpulse,
}

impl std::str::FromStr for Format {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "curl" => Ok(Self::Curl),
            "agproxy" => Ok(Self::Agproxy),
            "horizonpulse" => Ok(Self::Horizonpulse),
            _ => anyhow::bail!("unsupported relay_format"),
        }
    }
}

pub(crate) struct TargetRequest {
    pub method: Method,
    pub url: Url,
    /// Required business headers, without incidental transport defaults.
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub allowed_origins: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct PolicyViolation;
impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("relay target destination or credential policy rejected")
    }
}
impl std::error::Error for PolicyViolation {}

/// An attributable Curl delivery failure, distinct from an invalid paid envelope.
#[derive(Debug)]
pub(crate) struct TargetDeliveryFailure;
impl std::fmt::Display for TargetDeliveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("relay reported target delivery failure")
    }
}
impl std::error::Error for TargetDeliveryFailure {}

#[derive(Debug)]
pub(crate) struct Decoded {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub truncated: bool,
    pub delivery_failed: bool,
    pub final_url: Url,
}

impl TargetRequest {
    fn destination(&self, value: &str) -> Result<Url> {
        let url = crate::network::public_url(value).map_err(|_| PolicyViolation)?;
        ensure!(
            self.allowed_origins.is_empty()
                || self
                    .allowed_origins
                    .contains(&url.origin().ascii_serialization()),
            PolicyViolation
        );
        Ok(url)
    }

    fn validate(&self) -> Result<()> {
        self.destination(self.url.as_str())?;
        ensure!(
            !self.headers.keys().any(|name| credential(name.as_str())),
            PolicyViolation
        );
        Ok(())
    }
}

fn credential(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "cookie2"
            | "payment-signature"
            | "x-payment"
            | "x-api-key"
            | "api-key"
            | "x-auth-token"
            | "x-access-token"
    )
}

fn service_header(name: &str) -> bool {
    name.starts_with("proxy-")
        || matches!(
            name,
            "host"
                | "content-length"
                | "connection"
                | "accept-encoding"
                | "keep-alive"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
        )
}

impl Format {
    /// None means the vendor cannot represent the complete request, before paid I/O.
    pub(crate) fn encode(&self, target: &TargetRequest, limit: usize) -> Result<Option<Value>> {
        target.validate()?;
        if !matches!(target.method.as_str(), "GET" | "HEAD" | "POST") {
            return Ok(None);
        }
        let body = match target.body.as_deref() {
            Some(bytes) => match std::str::from_utf8(bytes) {
                Ok(body) => Some(body),
                Err(_) => return Ok(None),
            },
            None => None,
        };
        if target.method != Method::POST && body.is_some_and(|body| !body.is_empty()) {
            return Ok(None);
        }
        let mut headers = Map::new();
        for name in target.headers.keys() {
            // No documented adapter preserves duplicate header fields.
            let mut values = target.headers.get_all(name).iter();
            let value = values.next().expect("header key has a value");
            if values.next().is_some() {
                return Ok(None);
            }
            let Ok(value) = value.to_str() else {
                return Ok(None);
            };
            if service_header(name.as_str())
                || (*self == Self::Horizonpulse && name.as_str() != "accept")
            {
                // Horizon's published allowlist explicitly demonstrates accept;
                // it does not establish support for any other required header.
                return Ok(None);
            }
            headers.insert(name.to_string(), json!(value));
        }
        let mut value = json!({"url":target.url.as_str(),"method":target.method.as_str()});
        match self {
            Self::Curl => {
                if !headers.is_empty()
                    || body.is_some_and(|body| !body.is_empty())
                    || target.url.as_str().chars().count() > 8192
                {
                    return Ok(None);
                }
            }
            Self::Agproxy => {
                if target.url.as_str().chars().count() > 2000
                    || body.is_some_and(|body| body.len() > 1024 * 1024)
                {
                    return Ok(None);
                }
                value["max_mb"] = json!(limit.div_ceil(1_000_000).clamp(1, 10));
                value["headers"] = Value::Object(headers);
                if let Some(body) = body {
                    value["body"] = json!(body);
                }
                if serde_json::to_vec(&value)?.len() > 65_536 {
                    return Ok(None);
                }
            }
            Self::Horizonpulse => {
                if body.is_some_and(|body| body.len() > 65_536) {
                    return Ok(None);
                }
                value["headers"] = Value::Object(headers);
                if let Some(body) = body {
                    // Always use the documented string form; parsing/re-encoding
                    // a JSON target body would change the accepted request bytes.
                    value["body"] = json!(body);
                }
            }
        }
        Ok(Some(value))
    }

    pub(crate) fn decode(
        &self,
        bytes: &[u8],
        target: &TargetRequest,
        limit: usize,
        setting: &'static str,
    ) -> Result<Decoded> {
        target.validate()?;
        let envelope: Value = serde_json::from_slice(bytes).context("invalid relay JSON")?;
        let value = if *self == Self::Agproxy {
            let rows = envelope.as_array().context("invalid relay result array")?;
            ensure!(rows.len() == 1, "invalid relay result count");
            &rows[0]
        } else {
            &envelope
        };
        ensure!(value.is_object(), "invalid relay envelope");
        let (status, final_url, truncated, body, delivery_failed) = match self {
            Self::Curl => decode_curl(value, target)?,
            Self::Agproxy => {
                let final_url = target.destination(text(value, "final_url")?)?;
                let truncated = boolean(value, "truncated")?;
                let body = match (value.get("body"), value.get("body_base64")) {
                    (Some(body), None) => body
                        .as_str()
                        .context("invalid relay body")?
                        .as_bytes()
                        .to_vec(),
                    (None, Some(body)) => STANDARD
                        .decode(body.as_str().context("invalid relay base64 body")?)
                        .context("invalid relay base64")?,
                    _ => anyhow::bail!("ambiguous relay body encoding"),
                };
                (status(value, "status")?, final_url, truncated, body, false)
            }
            Self::Horizonpulse => {
                ensure!(
                    value.get("ok") == Some(&Value::Bool(true)),
                    "invalid relay success envelope"
                );
                let final_url = target.destination(text(value, "finalUrl")?)?;
                let body = match text(value, "bodyEncoding")? {
                    "text" => text(value, "body")?.as_bytes().to_vec(),
                    "base64" => STANDARD
                        .decode(text(value, "body")?)
                        .context("invalid relay base64")?,
                    _ => anyhow::bail!("unsupported relay body encoding"),
                };
                (
                    status(value, "status")?,
                    final_url,
                    boolean(value, "truncated")?,
                    body,
                    false,
                )
            }
        };
        let headers = response_headers(value)?;
        ensure!(
            target.method != Method::HEAD || body.is_empty(),
            "nonempty relay HEAD body"
        );
        if body.len() > limit {
            return Err(crate::limits::exceeded(
                "relayed response body",
                setting,
                limit,
            ));
        }
        Ok(Decoded {
            status,
            headers,
            body,
            truncated,
            delivery_failed,
            final_url,
        })
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .context("invalid relay text field")
}
fn boolean(value: &Value, key: &str) -> Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .context("invalid relay boolean field")
}
fn status(value: &Value, key: &str) -> Result<u16> {
    let status = value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
        .context("invalid relay target status")?;
    ensure!((100..=599).contains(&status), "invalid relay target status");
    Ok(status)
}
fn response_headers(value: &Value) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in value
        .get("headers")
        .and_then(Value::as_object)
        .context("invalid relay headers")?
    {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .context("invalid relay header name")?;
        ensure!(!headers.contains_key(&name), "duplicate relay header name");
        let value = reqwest::header::HeaderValue::from_str(
            value.as_str().context("invalid relay header value")?,
        )
        .context("invalid relay header value")?;
        headers.insert(name, value);
    }
    Ok(headers)
}
fn decode_curl(value: &Value, target: &TargetRequest) -> Result<(u16, Url, bool, Vec<u8>, bool)> {
    ensure!(
        text(value, "url")? == target.url.as_str(),
        "relay target mismatch"
    );
    ensure!(
        text(value, "method")? == target.method.as_str(),
        "relay method mismatch"
    );
    let mut final_url = target.url.clone();
    for hop in value
        .get("redirectChain")
        .and_then(Value::as_array)
        .context("invalid relay redirect chain")?
    {
        final_url = target.destination(hop.as_str().context("invalid relay redirect hop")?)?;
    }
    // Check complete attribution/shape before granting target-failure advancement.
    response_headers(value)?;
    let body = text(value, "body")?.as_bytes().to_vec();
    let error = value.get("error");
    ensure!(
        error.is_none_or(|v| v.is_null() || v.is_string()),
        "invalid relay error field"
    );
    let truncated = match value.get("truncated") {
        None => false,
        Some(value) => value.as_bool().context("invalid relay truncation field")?,
    };
    let delivery_failed = error
        .and_then(Value::as_str)
        .is_some_and(|error| !error.is_empty());
    if delivery_failed {
        // Curl uses zero when the target never returned an HTTP response.
        let code = value
            .get("statusCode")
            .and_then(Value::as_u64)
            .context("invalid relay target status")?;
        ensure!(
            code == 0 || (100..=599).contains(&code),
            "invalid relay target status"
        );
        if code == 0 {
            return Err(TargetDeliveryFailure.into());
        }
    }
    // A known HTTP result retains its headers and terminal consumer rules even
    // when Curl separately reports incomplete delivery.
    Ok((
        status(value, "statusCode")?,
        final_url,
        truncated,
        body,
        delivery_failed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> TargetRequest {
        TargetRequest {
            method: Method::GET,
            url: Url::parse("https://api.example.com/path?a=%2F&b=two+words").unwrap(),
            headers: HeaderMap::new(),
            body: None,
            allowed_origins: Vec::new(),
        }
    }

    fn envelope(format: Format, target: &TargetRequest) -> Value {
        match format {
            Format::Curl => json!({"url":target.url.as_str(),"method":target.method.as_str(),
                "statusCode":200,"headers":{},"body":"complete","redirectChain":[],"error":null}),
            Format::Agproxy => json!([{"status":200,"final_url":target.url.as_str(),
                "headers":{},"body":"complete","truncated":false}]),
            Format::Horizonpulse => json!({"ok":true,"status":200,"finalUrl":target.url.as_str(),
                "headers":{},"body":"complete","bodyEncoding":"text","truncated":false}),
        }
    }
    fn decode(format: Format, value: Value, target: &TargetRequest) -> Result<Decoded> {
        format.decode(
            &serde_json::to_vec(&value).unwrap(),
            target,
            1024,
            "max_response_bytes",
        )
    }

    #[test]
    fn request_encoding_preserves_exact_url_and_post_text() {
        let mut target = target();
        for format in [Format::Curl, Format::Agproxy, Format::Horizonpulse] {
            let encoded = format.encode(&target, usize::MAX).unwrap().unwrap();
            assert_eq!(encoded["url"], target.url.as_str());
            assert_eq!(encoded["method"], "GET");
        }
        assert_eq!(
            Format::Agproxy
                .encode(&target, usize::MAX)
                .unwrap()
                .unwrap()["max_mb"],
            10
        );
        assert_eq!(
            Format::Agproxy.encode(&target, 0).unwrap().unwrap()["max_mb"],
            1
        );
        target.method = Method::POST;
        let exact = "{\n \"key\":  [1,2] }\n";
        target.body = Some(exact.as_bytes().to_vec());
        assert!(Format::Curl.encode(&target, 1024).unwrap().is_none());
        for format in [Format::Agproxy, Format::Horizonpulse] {
            assert_eq!(
                format.encode(&target, 1024).unwrap().unwrap()["body"],
                exact
            );
        }
        target.body = None;
        assert_eq!(
            Format::Curl.encode(&target, 1024).unwrap().unwrap()["method"],
            "POST"
        );
        target.method = Method::HEAD;
        assert_eq!(
            Format::Curl.encode(&target, 1024).unwrap().unwrap()["method"],
            "HEAD"
        );
    }

    #[test]
    fn filtered_required_headers_and_external_request_caps_skip_before_dispatch() {
        let mut target = target();
        target
            .headers
            .insert("accept", "application/json".parse().unwrap());
        assert!(Format::Curl.encode(&target, 1024).unwrap().is_none());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_some());
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_some()
        );
        target
            .headers
            .insert("content-type", "application/json".parse().unwrap());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_some());
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_none()
        );
        target
            .headers
            .insert("accept-encoding", "identity".parse().unwrap());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        target.headers.clear();
        target
            .headers
            .append("accept", "application/json".parse().unwrap());
        target
            .headers
            .append("accept", "text/plain".parse().unwrap());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        target.headers.clear();
        target.method = Method::POST;
        target.body = Some(vec![b'x'; 65_536]);
        assert!(
            Format::Agproxy.encode(&target, 1024).unwrap().is_none(),
            "outer JSON cap includes its envelope"
        );
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_some()
        );
        target.body.as_mut().unwrap().push(b'x');
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_none()
        );
        target.body = Some(vec![0xff]);
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn credentials_and_destination_policy_are_terminal_across_adapters() {
        let mut target = target();
        for header in [
            "authorization",
            "cookie",
            "payment-signature",
            "x-payment",
            "x-api-key",
        ] {
            target.headers.clear();
            target.headers.insert(
                reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                "secret".parse().unwrap(),
            );
            for format in [Format::Curl, Format::Agproxy, Format::Horizonpulse] {
                assert!(
                    format
                        .encode(&target, 1024)
                        .unwrap_err()
                        .is::<PolicyViolation>()
                );
            }
        }
        target.headers.clear();
        target.allowed_origins = vec!["https://other.example.com".into()];
        assert!(
            Format::Agproxy
                .encode(&target, 1024)
                .unwrap_err()
                .is::<PolicyViolation>()
        );
        target.allowed_origins.clear();
        target.url = Url::parse("https://127.0.0.1/").unwrap();
        assert!(
            Format::Agproxy
                .encode(&target, 1024)
                .unwrap_err()
                .is::<PolicyViolation>()
        );
    }

    #[test]
    fn decoding_preserves_target_status_body_and_unpaid_challenge_headers() {
        let target = target();
        for format in [Format::Curl, Format::Agproxy, Format::Horizonpulse] {
            let mut value = envelope(format, &target);
            let row = if format == Format::Agproxy {
                &mut value[0]
            } else {
                &mut value
            };
            row[if format == Format::Curl {
                "statusCode"
            } else {
                "status"
            }] = json!(402);
            row["headers"] = json!({"payment-required":"challenge"});
            let decoded = decode(format, value, &target).unwrap();
            assert_eq!(decoded.status, 402);
            assert_eq!(decoded.headers["payment-required"], "challenge");
            assert_eq!(decoded.body, b"complete");
            assert!(!decoded.truncated);
            assert_eq!(decoded.final_url, target.url);
        }
    }

    #[test]
    fn binary_encodings_and_explicit_truncation_are_separate_from_malformed_envelopes() {
        let target = target();
        let decoded = decode(
            Format::Agproxy,
            json!([{"status":206,"final_url":target.url.as_str(),
            "headers":{},"body_base64":"AP8=","truncated":true}]),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.body, [0, 255]);
        assert!(decoded.truncated);
        let decoded = decode(
            Format::Horizonpulse,
            json!({"ok":true,"status":206,"finalUrl":target.url.as_str(),
            "headers":{},"body":"AP8=","bodyEncoding":"base64","truncated":false}),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.body, [0, 255]);
        for value in [
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body":"","body_base64":"","truncated":false}]),
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body_base64":"!","truncated":false}]),
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body":"","truncated":"false"}]),
            json!([]),
        ] {
            assert!(decode(Format::Agproxy, value, &target).is_err());
        }
    }

    #[test]
    fn redirects_validate_reported_hops_without_inventing_missing_evidence() {
        let mut target = target();
        target.allowed_origins = vec![target.url.origin().ascii_serialization()];
        for format in [Format::Agproxy, Format::Horizonpulse] {
            assert!(
                decode(format, envelope(format, &target), &target).is_ok(),
                "missing intermediate-hop evidence does not exclude an adapter"
            );
        }
        let mut value = envelope(Format::Curl, &target);
        value["redirectChain"] = json!([target.url.as_str(), "https://api.example.com/new"]);
        assert_eq!(
            decode(Format::Curl, value.clone(), &target)
                .unwrap()
                .final_url
                .as_str(),
            "https://api.example.com/new"
        );
        value["redirectChain"] = json!(["https://other.example.com/", target.url.as_str()]);
        assert!(
            decode(Format::Curl, value, &target)
                .unwrap_err()
                .is::<PolicyViolation>()
        );
        for format in [Format::Agproxy, Format::Horizonpulse] {
            let mut value = envelope(format, &target);
            if format == Format::Agproxy {
                value[0]["final_url"] = json!("https://127.0.0.1/");
            } else {
                value["finalUrl"] = json!("https://127.0.0.1/");
            }
            assert!(
                decode(format, value, &target)
                    .unwrap_err()
                    .is::<PolicyViolation>()
            );
        }
    }

    #[test]
    fn caller_limits_are_typed_terminal_and_head_bodies_are_not_accepted() {
        let mut target = target();
        for format in [Format::Curl, Format::Agproxy, Format::Horizonpulse] {
            let value = envelope(format, &target);
            let error = format
                .decode(
                    &serde_json::to_vec(&value).unwrap(),
                    &target,
                    1,
                    "max_response_bytes",
                )
                .unwrap_err();
            assert!(error.is::<crate::limits::DownloadLimit>());
            target.method = Method::HEAD;
            assert!(decode(format, envelope(format, &target), &target).is_err());
            target.method = Method::GET;
        }
    }

    #[test]
    fn curl_target_failure_requires_complete_attribution_and_shape() {
        let target = target();
        let mut value = envelope(Format::Curl, &target);
        value["statusCode"] = json!(0);
        value["error"] = json!("unreachable");
        assert!(
            decode(Format::Curl, value.clone(), &target)
                .unwrap_err()
                .is::<TargetDeliveryFailure>()
        );
        value["headers"] = json!({"bad\nheader":"x"});
        assert!(
            !decode(Format::Curl, value.clone(), &target)
                .unwrap_err()
                .is::<TargetDeliveryFailure>()
        );
        value["headers"] = json!({});
        value["method"] = json!("POST");
        assert!(
            !decode(Format::Curl, value, &target)
                .unwrap_err()
                .is::<TargetDeliveryFailure>()
        );
    }
}
