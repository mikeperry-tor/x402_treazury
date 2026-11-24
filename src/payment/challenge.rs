//! Bounded unsigned challenges shared by static and managed signers.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::header::HeaderMap;
use serde_json::Value;

const HEADER: &str = "payment-required";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinError {
    DuplicateHeaders,
    ConflictingV1Offers,
    Missing(X402Version),
}

impl PinError {
    pub(crate) fn from_code(code: &str) -> Option<Self> {
        [
            Self::DuplicateHeaders,
            Self::ConflictingV1Offers,
            Self::Missing(X402Version::V1),
            Self::Missing(X402Version::V2),
        ]
        .into_iter()
        .find(|error| error.code() == code)
    }

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::DuplicateHeaders => "duplicate_payment_challenge_headers",
            Self::ConflictingV1Offers => "conflicting_v1_offers",
            Self::Missing(X402Version::V1) => "missing_pinned_v1_challenge",
            Self::Missing(X402Version::V2) => "missing_pinned_v2_challenge",
        }
    }

    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::DuplicateHeaders => "the provider sent duplicate payment challenge headers",
            Self::ConflictingV1Offers => "the provider sent conflicting x402 version 1 offers",
            Self::Missing(X402Version::V1) => {
                "the provider did not send the configured x402 version 1 challenge"
            }
            Self::Missing(X402Version::V2) => {
                "the provider did not send the configured x402 version 2 challenge header"
            }
        }
    }
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}
impl std::error::Error for PinError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum X402Version {
    V1 = 1,
    V2 = 2,
}

impl TryFrom<u8> for X402Version {
    type Error = &'static str;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err("x402_version must be 1 or 2"),
        }
    }
}

impl From<X402Version> for u8 {
    fn from(value: X402Version) -> Self {
        value as u8
    }
}

pub(crate) async fn bound_challenge(
    response: reqwest::Response,
    limit: usize,
    version: Option<X402Version>,
) -> Result<reqwest::Response> {
    let mut envelope = axum::http::Response::builder().status(response.status());
    *envelope.headers_mut().expect("valid status") = response.headers().clone();
    let mut body = crate::limits::read(
        response,
        limit,
        "payment challenge body",
        "max_response_bytes",
    )
    .await
    .context("payment challenge rejected before signing")?;
    let headers = envelope.headers_mut().expect("valid status");
    if let Some(version) = version {
        select_version(headers, &mut body, version)?;
    } else if !headers.contains_key(HEADER)
        && let Some(challenge) = v1_body(&body)
    {
        // Managed admission and qualification read the normalized header.
        set_header(headers, challenge)?;
    }
    Ok(envelope.body(body)?.into())
}

fn select_version(headers: &mut HeaderMap, body: &mut Vec<u8>, version: X402Version) -> Result<()> {
    ensure!(
        headers.get_all(HEADER).iter().count() <= 1,
        PinError::DuplicateHeaders,
    );
    let header: Option<Value> = headers
        .get(HEADER)
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    match version {
        X402Version::V1 => {
            let header = header.filter(|value| value["x402Version"] == 1);
            let body = v1_body(body);
            if let (Some(header), Some(body)) = (&header, &body) {
                ensure!(
                    header["accepts"] == body["accepts"],
                    PinError::ConflictingV1Offers,
                );
            }
            let challenge = header.or(body).context(PinError::Missing(version))?;
            set_header(headers, challenge)?;
        }
        X402Version::V2 => {
            ensure!(
                header.is_some_and(|value| value["x402Version"] == 2),
                PinError::Missing(version),
            );
            // The static SDK otherwise falls back to a v1 body if its typed
            // v2 deserialization fails. This buffered challenge is for signing.
            body.clear();
        }
    }
    Ok(())
}

fn v1_body(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(|challenge| challenge["x402Version"] == 1)
}

fn set_header(headers: &mut HeaderMap, challenge: Value) -> Result<()> {
    headers.insert(
        HEADER,
        STANDARD.encode(serde_json::to_vec(&challenge)?).parse()?,
    );
    Ok(())
}
