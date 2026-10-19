//! Unsigned single-range GETs. Callers supply the real request's identity-bound client.
use super::{Config, budget::Reservation};
use reqwest::{Client, Response, StatusCode, Version, header};
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Representation {
    pub length: u64,
    pub validator: Option<(String, String)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure(pub &'static str);
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Failure {}
#[derive(Clone)]
pub struct Range {
    pub start: u64,
    pub length: u64,
    pub expected: Option<Representation>,
}
impl Range {
    pub fn qualification(config: &Config) -> Self {
        Self {
            start: 0,
            length: config.qualification_range_bytes,
            expected: None,
        }
    }
}
fn response_metadata(
    response: &Response,
    config: &Config,
    range: &Range,
) -> Result<Representation, Failure> {
    if response.version() != Version::HTTP_2 {
        return Err(Failure("cover_http2_unavailable"));
    }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(Failure(match response.status().as_u16() {
            200 => "cover_range_ignored",
            301 | 302 | 303 | 307 | 308 => "cover_redirect_refused",
            401 => "cover_auth_required",
            402 => "cover_payment_refused",
            403 => "cover_forbidden",
            416 => "cover_range_unsatisfiable",
            429 => "cover_rate_limited",
            _ => "cover_http_error",
        }));
    }
    let h = response.headers();
    if h.get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes() != b"identity")
    {
        return Err(Failure("cover_compression_refused"));
    }
    let raw = h
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .ok_or(Failure("cover_content_range_missing"))?;
    let (bounds, total) = raw
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .ok_or(Failure("cover_content_range_invalid"))?;
    let (start, end) = bounds
        .split_once('-')
        .ok_or(Failure("cover_content_range_invalid"))?;
    let number = |s: &str| -> Result<u64, Failure> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err(Failure("cover_content_range_invalid"));
        }
        s.parse()
            .map_err(|_| Failure("cover_content_range_invalid"))
    };
    let (start, end, total) = (number(start)?, number(end)?, number(total)?);
    if start != range.start
        || end.checked_sub(start).and_then(|v| v.checked_add(1)) != Some(range.length)
        || total <= end
        || total > config.max_resource_bytes
    {
        return Err(Failure("cover_content_range_mismatch"));
    }
    if let Some(length) = h.get(header::CONTENT_LENGTH)
        && length.to_str().ok().and_then(|s| s.parse::<u64>().ok()) != Some(range.length)
    {
        return Err(Failure("cover_content_length_mismatch"));
    }
    let mut validator = None;
    for key in [header::ETAG, header::LAST_MODIFIED] {
        if let Some(value) = h.get(&key) {
            if value.len() > 1024 {
                return Err(Failure("cover_validator_too_large"));
            }
            let value = value
                .to_str()
                .map_err(|_| Failure("cover_validator_invalid"))?;
            validator = Some((key.as_str().into(), value.into()));
            break;
        }
    }
    let representation = Representation {
        length: total,
        validator,
    };
    if range
        .expected
        .as_ref()
        .is_some_and(|expected| expected != &representation)
    {
        return Err(Failure("cover_representation_changed"));
    }
    Ok(representation)
}
/// Numeric response observations; never retain headers, URLs or upstream prose.
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub start: u64,
    pub requested_bytes: u64,
    pub http_status: Option<u16>,
    pub http_version: Option<Version>,
    pub declared_body_bytes: Option<u64>,
    pub received_bytes: u64,
    pub elapsed_ms: u64,
    pub stage: &'static str,
}
pub(crate) struct Outcome {
    pub result: Result<Representation, Failure>,
    pub observation: Observation,
}

/// No sign/retry middleware, redirects or full-download fallback. Cancellation drops
/// only this response stream and retains already received bytes in the reservation.
pub async fn download(
    client: &Client,
    config: &Config,
    range: Range,
    deadline: Instant,
    budget: Reservation,
    padding: Option<(header::HeaderName, header::HeaderValue)>,
) -> Result<Representation, Failure> {
    download_observed(client, config, range, deadline, budget, padding)
        .await
        .result
}

pub(crate) async fn download_observed(
    client: &Client,
    config: &Config,
    range: Range,
    deadline: Instant,
    mut budget: Reservation,
    padding: Option<(header::HeaderName, header::HeaderValue)>,
) -> Outcome {
    let started = Instant::now();
    let mut observation = Observation {
        start: range.start,
        requested_bytes: range.length,
        http_status: None,
        http_version: None,
        declared_body_bytes: None,
        received_bytes: 0,
        elapsed_ms: 0,
        stage: "request",
    };
    if range.length == 0 || range.start.checked_add(range.length).is_none() {
        return Outcome {
            result: Err(Failure("cover_range_invalid")),
            observation,
        };
    }
    let end = range.start + range.length - 1;
    let mut request = client
        .get(&config.url)
        .header(header::RANGE, format!("bytes={}-{}", range.start, end))
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some((name, value)) = padding {
        request = request.header(name, value);
    }
    let operation = async {
        budget.dispatched();
        let mut response = request
            .send()
            .await
            .map_err(|_| Failure("cover_transport_error"))?;
        observation.http_status = Some(response.status().as_u16());
        observation.http_version = Some(response.version());
        observation.declared_body_bytes = response.content_length();
        observation.stage = "response_metadata";
        let representation = response_metadata(&response, config, &range)?;
        observation.stage = "response_body";
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure("cover_body_error"))?
        {
            budget.received(chunk.len() as u64);
            observation.received_bytes = observation
                .received_bytes
                .saturating_add(chunk.len() as u64);
            if observation.received_bytes > range.length {
                return Err(Failure("cover_body_overflow"));
            }
        }
        if observation.received_bytes != range.length {
            return Err(Failure("cover_body_truncated"));
        }
        Ok(representation)
    };
    let result = tokio::time::timeout_at(deadline, operation)
        .await
        .unwrap_or(Err(Failure("cover_deadline")));
    budget.finish(Instant::now());
    observation.elapsed_ms = started.elapsed().as_millis() as u64;
    Outcome {
        result,
        observation,
    }
}
#[cfg(test)]
mod tests;
