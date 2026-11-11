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
    Readfetch,
}

impl std::str::FromStr for Format {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "curl" => Ok(Self::Curl),
            "agproxy" => Ok(Self::Agproxy),
            "horizonpulse" => Ok(Self::Horizonpulse),
            "readfetch" => Ok(Self::Readfetch),
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
        f.write_str("relay target destination policy rejected")
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
        self.destination(self.url.as_str()).map(|_| ())
    }
}

/// Runtime-owned x402 payment headers authorize a charge and are never replayed
/// through a paid relay, even when the caller did not mark the request as secret.
fn payment_header(name: &str) -> bool {
    matches!(name, "payment-signature" | "x-payment")
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
        if target
            .headers
            .keys()
            .any(|name| payment_header(name.as_str()))
            || !matches!(target.method.as_str(), "GET" | "HEAD" | "POST")
        {
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
            Self::Readfetch => {
                if target.url.as_str().chars().count() > 2083 {
                    return Ok(None);
                }
                value["headers"] = Value::Object(headers);
                if let Some(body) = body {
                    value["body"] = json!(body);
                }
                // Base64 preserves arbitrary target bytes, but expands the
                // response by about 4/3. Reserve JSON/header overhead and keep
                // the encoded body inside the caller's relay response budget.
                let budget = limit.saturating_sub(4_096);
                if budget < 1_024 {
                    return Ok(None);
                }
                let max_bytes = (budget.saturating_mul(3) / 4).clamp(1_024, 10_000_000);
                value["binary"] = json!(true);
                value["max_bytes"] = json!(max_bytes);
                // The vendor documents a 1 MiB JSON request body. Check the
                // complete serialized envelope, including headers and escaping,
                // before paid I/O.
                if serde_json::to_vec(&value)?.len() > 1_048_576 {
                    return Ok(None);
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
        let headers = if *self == Self::Readfetch {
            match value.get("headers") {
                Some(headers) if !headers.is_null() => response_headers(value)?,
                _ => HeaderMap::new(),
            }
        } else {
            response_headers(value)?
        };
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
            Self::Readfetch => decode_readfetch(value, target)?,
        };
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
fn decode_readfetch(
    value: &Value,
    target: &TargetRequest,
) -> Result<(u16, Url, bool, Vec<u8>, bool)> {
    let status = status(value, "status_code")?;
    let final_url = value
        .get("provenance")
        .and_then(|provenance| provenance.get("final_url"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .or_else(|| {
            value
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
        })
        .unwrap_or(target.url.as_str());
    let final_url = target.destination(final_url)?;
    let truncated = value
        .get("provenance")
        .and_then(|provenance| provenance.get("extraction"))
        .and_then(|extraction| extraction.get("truncated"))
        .filter(|truncated| !truncated.is_null())
        .or_else(|| {
            value
                .get("truncated")
                .filter(|truncated| !truncated.is_null())
        });
    let truncated = match truncated {
        Some(truncated) => truncated
            .as_bool()
            .context("invalid relay truncation field")?,
        None => false,
    };
    let body = match (
        value.get("content_base64").filter(|body| !body.is_null()),
        value.get("text").filter(|body| !body.is_null()),
    ) {
        (Some(body), None) => STANDARD
            .decode(body.as_str().context("invalid relay base64 body")?)
            .context("invalid relay base64")?,
        (None, Some(body)) => body
            .as_str()
            .context("invalid relay text body")?
            .as_bytes()
            .to_vec(),
        (None, None) => Vec::new(),
        (Some(_), Some(_)) => anyhow::bail!("ambiguous relay body encoding"),
    };
    // `billed` describes the readfetch charge, not whether the target produced
    // an HTTP result. Delivery failure must follow the target status/body.
    Ok((status, final_url, truncated, body, false))
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

    fn readfetch_envelope(
        target: &TargetRequest,
        status: u16,
        body: &[u8],
        truncated: bool,
    ) -> Value {
        json!({"status":"success","billed":true,"status_code":status,
            "headers":{},"content_base64":STANDARD.encode(body),
            "url":target.url.as_str(),
            "provenance":{"final_url":target.url.as_str(),
                "extraction":{"truncated":truncated}}})
    }

    fn envelope(format: Format, target: &TargetRequest) -> Value {
        match format {
            Format::Curl => json!({"url":target.url.as_str(),"method":target.method.as_str(),
                "statusCode":200,"headers":{},"body":"complete","redirectChain":[],"error":null}),
            Format::Agproxy => json!([{"status":200,"final_url":target.url.as_str(),
                "headers":{},"body":"complete","truncated":false}]),
            Format::Horizonpulse => json!({"ok":true,"status":200,"finalUrl":target.url.as_str(),
                "headers":{},"body":"complete","bodyEncoding":"text","truncated":false}),
            Format::Readfetch => readfetch_envelope(target, 200, b"complete", false),
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
        for format in [
            Format::Curl,
            Format::Agproxy,
            Format::Horizonpulse,
            Format::Readfetch,
        ] {
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
        assert!(
            Format::Readfetch.encode(&target, 0).unwrap().is_none(),
            "base64 and JSON cannot fit a zero-byte response budget"
        );
        let readfetch = Format::Readfetch
            .encode(&target, 1_000_000)
            .unwrap()
            .unwrap();
        assert_eq!(readfetch["binary"], true);
        let max_bytes = readfetch["max_bytes"].as_u64().unwrap();
        assert!((1_024..=746_928).contains(&max_bytes));
        assert_eq!(
            Format::Readfetch
                .encode(&target, 20_000_000)
                .unwrap()
                .unwrap()["max_bytes"],
            10_000_000
        );
        target.method = Method::POST;
        let exact = "{\n \"key\":  [1,2] }\n";
        target.body = Some(exact.as_bytes().to_vec());
        assert!(Format::Curl.encode(&target, 1024).unwrap().is_none());
        for format in [Format::Agproxy, Format::Horizonpulse, Format::Readfetch] {
            assert_eq!(
                format.encode(&target, 1_000_000).unwrap().unwrap()["body"],
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
    fn purchase_business_headers_require_faithful_relay_representation() {
        let mut target = target();
        target.method = Method::POST;
        target
            .headers
            .insert("x-quote-id", "quote-123".parse().unwrap());
        target.headers.insert(
            "idempotency-key",
            "24a08964-2c6e-43c5-a127-3765185dbcea".parse().unwrap(),
        );
        for format in [Format::Agproxy, Format::Readfetch] {
            let encoded = format.encode(&target, 1_000_000).unwrap().unwrap();
            assert_eq!(encoded["headers"]["x-quote-id"], "quote-123");
            assert_eq!(
                encoded["headers"]["idempotency-key"],
                "24a08964-2c6e-43c5-a127-3765185dbcea"
            );
        }
        assert!(Format::Curl.encode(&target, 1024).unwrap().is_none());
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_none()
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
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
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
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_some()
        );
        target
            .headers
            .insert("accept-encoding", "identity".parse().unwrap());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_none()
        );
        target.headers.clear();
        target
            .headers
            .append("accept", "application/json".parse().unwrap());
        target
            .headers
            .append("accept", "text/plain".parse().unwrap());
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_none()
        );
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
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
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
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_some()
        );
        target.body = Some(vec![0xff]);
        assert!(Format::Agproxy.encode(&target, 1024).unwrap().is_none());
        assert!(
            Format::Horizonpulse
                .encode(&target, 1024)
                .unwrap()
                .is_none()
        );
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_none()
        );
        target.body = None;
        target.method = Method::PUT;
        for format in [Format::Curl, Format::Agproxy, Format::Readfetch] {
            assert!(format.encode(&target, 1024).unwrap().is_none());
        }
        target.method = Method::POST;
        target.url = format!("https://api.example.com/{}", "x".repeat(2060))
            .parse()
            .unwrap();
        assert_eq!(target.url.as_str().chars().count(), 2084);
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_none()
        );
        target.url = format!("https://api.example.com/{}", "x".repeat(2059))
            .parse()
            .unwrap();
        assert_eq!(target.url.as_str().chars().count(), 2083);
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn business_headers_forward_when_representable_and_payment_headers_skip() {
        let mut target = target();
        for header in ["x-api-key", "api-key", "x-auth-token", "x-access-token"] {
            target.headers.clear();
            target.headers.insert(
                reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                "secret".parse().unwrap(),
            );
            for format in [Format::Agproxy, Format::Readfetch] {
                let encoded = format.encode(&target, 1_000_000).unwrap().unwrap();
                assert_eq!(encoded["headers"][header], "secret");
            }
            for format in [Format::Curl, Format::Horizonpulse] {
                assert!(format.encode(&target, 1024).unwrap().is_none());
            }
        }
        for (header, value) in [
            ("authorization", "Bearer business"),
            ("cookie", "session=value"),
            ("cookie2", "session=value"),
        ] {
            target.headers.clear();
            target.headers.insert(
                reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            for format in [Format::Agproxy, Format::Readfetch] {
                let encoded = format.encode(&target, 1_000_000).unwrap().unwrap();
                assert_eq!(encoded["headers"][header], value);
            }
            for format in [Format::Curl, Format::Horizonpulse] {
                assert!(format.encode(&target, 1024).unwrap().is_none());
            }
        }
        for header in ["payment-signature", "x-payment"] {
            target.headers.clear();
            target.headers.insert(
                reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                "secret".parse().unwrap(),
            );
            for format in [
                Format::Curl,
                Format::Agproxy,
                Format::Horizonpulse,
                Format::Readfetch,
            ] {
                assert!(format.encode(&target, 1024).unwrap().is_none());
            }
        }
        target.headers.clear();
        target.headers.insert(
            reqwest::header::HeaderName::from_bytes(b"proxy-authorization").unwrap(),
            "secret".parse().unwrap(),
        );
        for format in [
            Format::Curl,
            Format::Agproxy,
            Format::Horizonpulse,
            Format::Readfetch,
        ] {
            assert!(format.encode(&target, 1024).unwrap().is_none());
        }
        target.headers.clear();
        target.allowed_origins = vec!["https://other.example.com".into()];
        for format in [Format::Agproxy, Format::Readfetch] {
            assert!(
                format
                    .encode(&target, 1024)
                    .unwrap_err()
                    .is::<PolicyViolation>()
            );
        }
        target.allowed_origins.clear();
        target.url = Url::parse("https://127.0.0.1/").unwrap();
        for format in [Format::Agproxy, Format::Readfetch] {
            assert!(
                format
                    .encode(&target, 1024)
                    .unwrap_err()
                    .is::<PolicyViolation>()
            );
        }
    }

    #[test]
    fn decoding_preserves_target_status_body_and_unpaid_challenge_headers() {
        let target = target();
        for format in [
            Format::Curl,
            Format::Agproxy,
            Format::Horizonpulse,
            Format::Readfetch,
        ] {
            let mut value = envelope(format, &target);
            let row = if format == Format::Agproxy {
                &mut value[0]
            } else {
                &mut value
            };
            row[if format == Format::Curl {
                "statusCode"
            } else if format == Format::Readfetch {
                "status_code"
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
        let decoded = decode(
            Format::Readfetch,
            readfetch_envelope(&target, 206, &[0, 255], true),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.body, [0, 255]);
        assert!(decoded.truncated);
        for value in [
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body":"","body_base64":"","truncated":false}]),
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body_base64":"!","truncated":false}]),
            json!([{"status":200,"final_url":target.url.as_str(),"headers":{},"body":"","truncated":"false"}]),
            json!([]),
        ] {
            assert!(decode(Format::Agproxy, value, &target).is_err());
        }
        for value in [
            json!({"status_code":200,"headers":{},"content_base64":"!",
                "provenance":{"final_url":target.url.as_str()}}),
            json!({"status_code":200,"headers":{},"content_base64":"",
                "text":"","provenance":{"final_url":target.url.as_str()}}),
            json!({"headers":{},"content_base64":"","provenance":{"final_url":target.url.as_str()}}),
            json!({"status_code":200,"headers":{},"content_base64":"",
                "provenance":{"final_url":target.url.as_str(),"extraction":{"truncated":"false"}}}),
        ] {
            assert!(decode(Format::Readfetch, value, &target).is_err());
        }
    }

    #[test]
    fn readfetch_billing_and_optional_fields_do_not_define_delivery() {
        let target = target();
        let mut value = readfetch_envelope(&target, 200, b"complete", false);
        value["billed"] = json!(false);
        let decoded = decode(Format::Readfetch, value, &target).unwrap();
        assert_eq!(decoded.body, b"complete");
        assert!(!decoded.delivery_failed);

        let decoded = decode(
            Format::Readfetch,
            json!({"status_code":200,"headers":{},"text":"complete"}),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.final_url, target.url);
        assert_eq!(decoded.body, b"complete");
        assert!(!decoded.truncated);

        let decoded = decode(
            Format::Readfetch,
            json!({"status_code":204,"headers":{}}),
            &target,
        )
        .unwrap();
        assert!(decoded.body.is_empty());

        let decoded = decode(
            Format::Readfetch,
            json!({"status_code":402,"headers":{"payment-required":"challenge"},
                "content_base64":STANDARD.encode("challenge"),
                "provenance":{"final_url":target.url.as_str(),
                    "extraction":{"truncated":false}}}),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.status, 402);
        assert_eq!(decoded.headers["payment-required"], "challenge");
        assert_eq!(decoded.body, b"challenge");
    }

    #[test]
    fn readfetch_request_envelope_and_base64_budget_fit_vendor_and_outer_limits() {
        let mut target = target();
        assert!(Format::Readfetch.encode(&target, 1_024).unwrap().is_none());
        let encoded = Format::Readfetch
            .encode(&target, 1_000_000)
            .unwrap()
            .unwrap();
        let max_bytes = encoded["max_bytes"].as_u64().unwrap();
        assert!((1_024..=746_928).contains(&max_bytes));
        assert_eq!(
            Format::Readfetch
                .encode(&target, 20_000_000)
                .unwrap()
                .unwrap()["max_bytes"],
            10_000_000
        );
        target.method = Method::POST;
        target.body = Some(vec![b'x'; 1_100_000]);
        assert!(
            Format::Readfetch
                .encode(&target, 20_000_000)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn readfetch_request_size_checks_the_complete_serialized_envelope() {
        let mut target = target();
        target.method = Method::POST;
        target
            .headers
            .insert("x-quote-id", "quote-\"123".parse().unwrap());
        target.body = Some("\"\n💰".as_bytes().to_vec());
        let encoded = Format::Readfetch
            .encode(&target, 1_000_000)
            .unwrap()
            .unwrap();
        let padding = 1_048_576 - serde_json::to_vec(&encoded).unwrap().len();
        target.body.as_mut().unwrap().extend(vec![b'x'; padding]);

        let encoded = Format::Readfetch
            .encode(&target, 1_000_000)
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::to_vec(&encoded).unwrap().len(), 1_048_576);
        target.body.as_mut().unwrap().push(b'x');
        assert!(
            Format::Readfetch
                .encode(&target, 1_000_000)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn readfetch_null_optional_fields_and_missing_headers_are_tolerated() {
        let target = target();
        let decoded = decode(
            Format::Readfetch,
            json!({"status_code":200,"headers":null,
                "content_base64":null,"text":"complete",
                "provenance":{"final_url":"","extraction":{"truncated":null}},
                "url":target.url.as_str()}),
            &target,
        )
        .unwrap();
        assert!(decoded.headers.is_empty());
        assert_eq!(decoded.body, b"complete");
        assert!(!decoded.truncated);
        assert_eq!(decoded.final_url, target.url);

        let decoded = decode(
            Format::Readfetch,
            json!({"status_code":200,"headers":{},
                "content_base64":STANDARD.encode("complete"),"text":null,
                "truncated":null}),
            &target,
        )
        .unwrap();
        assert_eq!(decoded.body, b"complete");
        assert!(!decoded.truncated);
    }

    #[test]
    fn redirects_validate_reported_hops_without_inventing_missing_evidence() {
        let mut target = target();
        target.allowed_origins = vec![target.url.origin().ascii_serialization()];
        for format in [Format::Agproxy, Format::Horizonpulse, Format::Readfetch] {
            assert!(
                decode(format, envelope(format, &target), &target).is_ok(),
                "missing intermediate-hop evidence does not exclude an adapter"
            );
        }
        for (field, value) in [
            ("url", json!("https://other.example.com/")),
            ("redirectChain", json!([{}])),
        ] {
            let mut invalid = envelope(Format::Curl, &target);
            invalid[field] = value;
            assert!(decode(Format::Curl, invalid, &target).is_err(), "{field}");
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
        for format in [Format::Agproxy, Format::Horizonpulse, Format::Readfetch] {
            let mut value = envelope(format, &target);
            match format {
                Format::Agproxy => value[0]["final_url"] = json!("https://127.0.0.1/"),
                Format::Horizonpulse => value["finalUrl"] = json!("https://127.0.0.1/"),
                Format::Readfetch => value["provenance"]["final_url"] = json!("https://127.0.0.1/"),
                Format::Curl => unreachable!(),
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
        if crate::test_log_capture::run_in_child(
            "discovery_relay::adapters::tests::caller_limits_are_typed_terminal_and_head_bodies_are_not_accepted",
        ) {
            return;
        }
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let mut target = target();
        target.url = "https://api.example.com/PRIVATE_PATH?token=PRIVATE_TOKEN"
            .parse()
            .unwrap();
        for (format, setting) in [
            (Format::Curl, "max_spec_bytes"),
            (Format::Agproxy, "max_response_bytes"),
            (Format::Horizonpulse, "max_response_bytes"),
            (Format::Readfetch, "max_response_bytes"),
        ] {
            let mut value = envelope(format, &target);
            match format {
                Format::Agproxy => value[0]["body"] = json!("PRIVATE_BODY"),
                Format::Readfetch => {
                    value["content_base64"] = json!(STANDARD.encode("PRIVATE_BODY"))
                }
                _ => value["body"] = json!("PRIVATE_BODY"),
            }
            let error = format
                .decode(&serde_json::to_vec(&value).unwrap(), &target, 1, setting)
                .unwrap_err();
            let bound = error
                .downcast_ref::<crate::limits::DownloadLimit>()
                .unwrap();
            assert_eq!(bound.setting, setting);
            assert_eq!(bound.limit, 1);
            assert!(error.to_string().contains(&format!("{setting}=1")));
            assert!(!error.to_string().contains("PRIVATE_"));
            target.method = Method::HEAD;
            assert!(decode(format, envelope(format, &target), &target).is_err());
            target.method = Method::GET;
        }
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(logs.contains("download limit exceeded") && logs.contains("limit_bytes=1"));
        assert!(logs.contains("max_spec_bytes") && logs.contains("max_response_bytes"));
        assert!(!logs.contains("PRIVATE_") && !logs.contains("api.example.com"));
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
