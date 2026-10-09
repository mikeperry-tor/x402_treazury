//! Only unsigned read RPCs may retry, on the same client, endpoint and payload.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{error::Error, io::ErrorKind, time::Duration};
use tokio::time::Instant;

const BACKOFF: Duration = Duration::from_millis(200);

/// Fixed labels and numeric codes only; never display URLs, payloads or causes.
#[derive(Debug)]
pub(crate) struct RpcFailure {
    method: &'static str,
    category: &'static str,
    http: Option<u16>,
    code: Option<i64>,
    phase: &'static str,
    io_kind: Option<ErrorKind>,
    tls: bool,
    protocol: Option<&'static str>,
}
impl std::fmt::Display for RpcFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Base RPC {}: {}; phase {}",
            self.method, self.category, self.phase
        )?;
        if let Some(status) = self.http {
            write!(f, "; HTTP {status}")?;
        }
        if let Some(code) = self.code {
            write!(f, "; RPC code {code}")?;
        }
        if let Some(kind) = self.io_kind {
            write!(f, "; IO {kind:?}")?;
        }
        if let Some(protocol) = self.protocol {
            write!(f, "; protocol {protocol}")?;
        }
        if self.tls {
            write!(f, "; TLS")?;
        }
        Ok(())
    }
}
impl Error for RpcFailure {}

impl RpcFailure {
    pub(super) fn can_failover(&self) -> bool {
        if self.tls || matches!(self.protocol, Some("invalid_http" | "request_error")) {
            return false;
        }
        matches!(self.http, Some(403 | 408 | 429 | 500 | 502 | 503 | 504))
            || matches!(self.code, Some(-32005 | -32016 | -32601))
            || matches!(
                self.category,
                "timeout" | "connect" | "transport" | "body transport"
            )
    }
    fn new(
        method: &'static str,
        category: &'static str,
        phase: &'static str,
        http: Option<u16>,
    ) -> Self {
        Self {
            method,
            category,
            http,
            code: None,
            phase,
            io_kind: None,
            tls: false,
            protocol: None,
        }
    }
    fn transport(
        method: &'static str,
        phase: &'static str,
        http: Option<u16>,
        error: &reqwest::Error,
    ) -> Self {
        let category = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connect"
        } else if error.is_body() {
            "body transport"
        } else if error.is_builder() {
            "request construction"
        } else {
            "transport"
        };
        let mut failure = Self::new(method, category, phase, http);
        let mut cause = error.source();
        while let Some(error) = cause {
            if let Some(io) = error.downcast_ref::<std::io::Error>() {
                failure.io_kind = Some(io.kind());
                // io::Error::source may skip its wrapped error itself.
                failure.tls |= io
                    .get_ref()
                    .is_some_and(|inner| inner.is::<rustls::Error>());
            }
            failure.tls |= error.is::<rustls::Error>();
            if let Some(error) = error.downcast_ref::<hyper::Error>() {
                failure.protocol = Some(if error.is_incomplete_message() {
                    "incomplete_message"
                } else if error.is_closed() {
                    "closed"
                } else if error.is_canceled() {
                    "canceled"
                } else if error.is_timeout() {
                    "timeout"
                } else if error.is_parse() {
                    "invalid_http"
                } else if error.is_user() {
                    "request_error"
                } else {
                    "http_transport"
                });
            }
            cause = error
                .downcast_ref::<std::io::Error>()
                .and_then(|io| io.get_ref())
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| error.source());
        }
        if phase == "body" && !error.is_timeout() {
            failure.category = "body transport";
        }
        failure
    }
}

struct AttemptFailure {
    public: RpcFailure,
    retryable: bool,
}
impl From<RpcFailure> for AttemptFailure {
    fn from(public: RpcFailure) -> Self {
        Self {
            public,
            retryable: false,
        }
    }
}
fn transport_failure(
    method: &'static str,
    phase: &'static str,
    http: Option<u16>,
    error: reqwest::Error,
) -> AttemptFailure {
    let public = RpcFailure::transport(method, phase, http, &error);
    let retryable = !error.is_builder()
        && !error.is_redirect()
        && (!error.is_decode() || public.io_kind.is_some() || public.protocol.is_some())
        && !public.tls
        && !matches!(public.protocol, Some("invalid_http" | "request_error"));
    AttemptFailure { public, retryable }
}

async fn attempt(
    http: &reqwest::Client,
    url: &reqwest::Url,
    method: &'static str,
    payload: &Value,
) -> std::result::Result<Value, AttemptFailure> {
    let response = http
        .post(url.clone())
        .json(payload)
        .send()
        .await
        .map_err(|e| transport_failure(method, "send", None, e))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(RpcFailure::new(method, "HTTP rejection", "headers", Some(status)).into());
    }
    // Read separately so an interrupted body is distinguishable from malformed JSON.
    let body = response
        .bytes()
        .await
        .map_err(|e| transport_failure(method, "body", Some(status), e))?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| RpcFailure::new(method, "invalid JSON body", "decode", Some(status)))?;
    if value.get("error").is_some() {
        let mut error = RpcFailure::new(method, "RPC rejection", "envelope", Some(status));
        error.code = value["error"]["code"].as_i64();
        return Err(error.into());
    }
    if value["id"] != 1 || value["jsonrpc"] != "2.0" {
        return Err(RpcFailure::new(method, "invalid envelope", "envelope", Some(status)).into());
    }
    value
        .get("result")
        .filter(|v| !v.is_null() || method == "eth_getTransactionReceipt")
        .cloned()
        .ok_or_else(|| RpcFailure::new(method, "missing result", "envelope", Some(status)).into())
}

pub(super) async fn rpc(
    http: &reqwest::Client,
    url: &reqwest::Url,
    method: &'static str,
    params: Value,
) -> Result<Value> {
    ensure!(
        matches!(
            method,
            "eth_chainId" | "eth_getBlockByNumber" | "eth_getTransactionReceipt" | "eth_call"
        ),
        "unsupported read-only Base RPC method"
    );
    let payload = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    let started = Instant::now();
    // The network client owns connection and renewable read-inactivity limits.
    // Keep retries finite without imposing a second elapsed-time deadline.
    for number in 1..=2 {
        let failure = match attempt(http, url, method, &payload).await {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        let retry = number == 1 && failure.retryable;
        tracing::warn!(category = %failure.public, attempt = number, retry,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "Read-only Base RPC attempt failed; payment is not retried");
        if !retry {
            return Err(failure.public.into());
        }
        tokio::time::sleep(BACKOFF).await;
    }
    unreachable!("the final attempt always returns")
}

#[cfg(test)]
mod tests;
