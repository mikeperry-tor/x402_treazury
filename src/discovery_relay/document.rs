//! Complete unsigned documents; consumers retain parsing and publication ownership.
use super::{SearchRelay, TargetFailure, adapters::TargetRequest, sequence::Contract};
use crate::{
    network::{
        self, HttpPolicy, HttpTransport, IsolationId,
        acquisition::{self, Failure, FallbackPolicy},
    },
    payment::PaidClient,
};
use anyhow::{Result, ensure};
use reqwest::{StatusCode, header::HeaderMap};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub(crate) struct Options {
    pub url: String,
    pub identity: IsolationId,
    pub public_only: bool,
    pub allowed_origins: Vec<String>,
    pub transport: HttpPolicy,
    pub timeout: Option<Duration>,
    pub limit: usize,
    pub setting: &'static str,
}
impl Options {
    pub fn new(url: &str, identity: IsolationId, limit: usize, setting: &'static str) -> Self {
        Self {
            url: url.into(),
            identity,
            public_only: true,
            allowed_origins: Vec::new(),
            transport: Default::default(),
            timeout: None,
            limit,
            setting,
        }
    }
    fn target(&self) -> Result<TargetRequest> {
        Ok(TargetRequest {
            method: reqwest::Method::GET,
            url: self.url.parse()?,
            headers: HeaderMap::new(),
            body: None,
            allowed_origins: self.allowed_origins.clone(),
        })
    }
    fn validate_destination(&self) -> Result<()> {
        let url = if self.public_only {
            network::public_url(&self.url)?
        } else {
            self.url.parse()?
        };
        ensure!(
            self.allowed_origins.is_empty()
                || self
                    .allowed_origins
                    .contains(&url.origin().ascii_serialization()),
            "document destination origin not authorized"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Document {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Arc<Vec<u8>>,
    pub payment_may_have_occurred: bool,
    origin: Option<(Arc<HttpTransport>, Arc<HttpTransport>)>,
}
impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("status", &self.status)
            .field("bytes", &self.body.len())
            .field("payment_may_have_occurred", &self.payment_may_have_occurred)
            .finish()
    }
}
impl Document {
    pub fn local(body: Vec<u8>) -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Arc::new(body),
            payment_may_have_occurred: false,
            origin: None,
        }
    }
    pub fn with_payment_context<T>(&self, result: Result<T>) -> Result<T> {
        with_payment_context(self.payment_may_have_occurred, result)
    }
    /// Parsing runs once at the owning consumer. Rejection cannot purchase a
    /// second read, discard possible fees, or promote rejected origin content.
    pub fn validate<T>(
        &self,
        validate: impl FnOnce(StatusCode, &HeaderMap, &[u8]) -> Result<T>,
    ) -> Result<T> {
        let value = self.with_payment_context(validate(self.status, &self.headers, &self.body))?;
        if let Some((selected, current)) = &self.origin {
            network::global().promote_http(selected, current);
        }
        Ok(value)
    }
}

/// Apply the acquisition's existing fee fact through final publication/output.
pub(crate) fn with_payment_context<T>(possible: bool, result: Result<T>) -> Result<T> {
    result.map_err(|error| {
        if possible && !error.is::<PossiblePayment>() {
            error.context(PossiblePayment)
        } else {
            error
        }
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PossiblePayment;
impl std::fmt::Display for PossiblePayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Relay response_rejected or delivery failed; a relay authorization may still charge; do not automatically repeat this logical call")
    }
}
impl std::error::Error for PossiblePayment {}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IncompleteDocument {
    pub status: u16,
}
impl std::fmt::Display for IncompleteDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("document partial_response")
    }
}
impl std::error::Error for IncompleteDocument {}

#[derive(Clone, Copy, Debug)]
struct DocumentStatus(u16);
impl std::fmt::Display for DocumentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "document HTTP {}", self.0)
    }
}
impl std::error::Error for DocumentStatus {}

#[derive(Clone, Copy, Debug)]
enum Cause {
    Other,
    Status(u16),
    Partial(u16),
    Limit {
        kind: &'static str,
        setting: &'static str,
        limit: usize,
    },
}
/// Shared cache failures contain safe facts, never upstream prose or credentials.
#[derive(Clone, Debug)]
pub(crate) struct SharedFailure {
    reason: &'static str,
    cause: Cause,
    possible: bool,
}
impl SharedFailure {
    pub fn local(reason: &'static str) -> Self {
        Self {
            reason,
            cause: Cause::Other,
            possible: false,
        }
    }
    pub fn capture(error: &anyhow::Error, reason: &'static str) -> Self {
        let cause = if let Some(limit) = error.downcast_ref::<crate::limits::DownloadLimit>() {
            Cause::Limit {
                kind: limit.kind,
                setting: limit.setting,
                limit: limit.limit,
            }
        } else if let Some(partial) = error.downcast_ref::<IncompleteDocument>() {
            Cause::Partial(partial.status)
        } else {
            let status = error
                .downcast_ref::<DocumentStatus>()
                .map(|e| e.0)
                .or_else(|| error.downcast_ref::<TargetFailure>().and_then(|e| e.status))
                .or_else(|| {
                    error
                        .downcast_ref::<crate::payment::HttpFailure>()
                        .map(|e| e.status.as_u16())
                })
                .or_else(|| {
                    error
                        .downcast_ref::<reqwest::Error>()
                        .and_then(|e| e.status())
                        .map(|s| s.as_u16())
                });
            status.map(Cause::Status).unwrap_or(Cause::Other)
        };
        Self {
            reason,
            cause,
            possible: error.is::<PossiblePayment>(),
        }
    }
    pub fn possible_payment(&self) -> bool {
        self.possible
    }
    pub fn status(&self) -> Option<u16> {
        match self.cause {
            Cause::Status(status) | Cause::Partial(status) => Some(status),
            _ => None,
        }
    }
    pub fn error(&self) -> anyhow::Error {
        let error = match self.cause {
            Cause::Other => anyhow::anyhow!(self.reason),
            Cause::Status(status) => {
                anyhow::Error::new(DocumentStatus(status)).context(self.reason)
            }
            Cause::Partial(status) => {
                anyhow::Error::new(IncompleteDocument { status }).context(self.reason)
            }
            Cause::Limit {
                kind,
                setting,
                limit,
            } => crate::limits::exceeded(kind, setting, limit).context(self.reason),
        };
        if self.possible {
            error.context(PossiblePayment)
        } else {
            error
        }
    }
}

pub(crate) async fn origin(options: &Options) -> Result<Document, Failure> {
    options.validate_destination().map_err(Failure::local)?;
    let factory = network::global();
    let selected = factory
        .http_participating(
            &options.identity,
            &options.url,
            options.timeout,
            options.public_only,
            options.transport,
        )
        .map_err(Failure::local)?;
    let result = acquisition::run(
        factory,
        selected.clone(),
        false,
        |current, _, _| async move {
            let policy = FallbackPolicy::BlockingFailures;
            let response = current
                .client
                .get(&options.url)
                .send()
                .await
                .map_err(|error| Failure::transport(error.without_url()))?;
            network::log_http(&response, "document");
            let status = response.status();
            let headers = response.headers().clone();
            if status == StatusCode::PARTIAL_CONTENT
                || headers.contains_key(reqwest::header::CONTENT_RANGE)
            {
                return Err(Failure::validation(
                    IncompleteDocument {
                        status: status.as_u16(),
                    }
                    .into(),
                ));
            }
            if !status.is_success() {
                return Err(Failure::status(
                    DocumentStatus(status.as_u16()).into(),
                    status,
                    &headers,
                    policy,
                ));
            }
            let body = crate::limits::read(response, options.limit, "document", options.setting)
                .await
                .map_err(Failure::body)?;
            Ok((status, headers, body))
        },
    )
    .await
    .inspect_err(|failure| {
        let reason = match failure.kind {
            acquisition::FailureKind::HttpStatus(_) => "HTTP response rejected",
            acquisition::FailureKind::Transport
                if failure
                    .error
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_timeout) =>
            {
                "transport timeout"
            }
            acquisition::FailureKind::Transport => "transport failure",
            _ => "document rejected",
        };
        tracing::warn!(reason, "Unsigned document acquisition failed");
    })?;
    let (status, headers, body) = result.value;
    Ok(Document {
        status,
        headers,
        body: Arc::new(body),
        payment_may_have_occurred: false,
        origin: Some((selected, result.current)),
    })
}

/// Immutable existing relay preparation, without authority or accepted I/O.
pub(crate) struct PreparedDocument {
    prepared: super::sequence::Prepared,
    target: TargetRequest,
    options: Options,
}

/// Prepare once before a lazy caller loads an existing payer.
pub(crate) fn prepare_fallback(
    relay: &SearchRelay,
    options: &Options,
) -> Result<Option<PreparedDocument>> {
    network::public_url(&options.url)?;
    let target = options.target()?;
    let prepared = relay.prepare(&target, options.limit)?;
    Ok((!prepared.is_empty()).then(|| PreparedDocument {
        prepared,
        target,
        options: options.clone(),
    }))
}

impl PreparedDocument {
    pub(crate) async fn acquire(self, client: &PaidClient, failure: Failure) -> Result<Document> {
        if !failure.fallback || !client.has_document_authority() {
            return Err(failure.error);
        }
        let options = self.options;
        let response = self
            .prepared
            .acquire(
                client.clone(),
                &self.target,
                Contract::Document,
                options.limit,
                options.setting,
                Some(client.response_limit()),
                client.max_help_bytes(),
                options.timeout,
                &[],
                None,
                failure.retry_after,
            )
            .await
            .map_err(|error| {
                if let Some(failure) = error.downcast_ref::<TargetFailure>()
                    && failure.reason == "partial_response"
                {
                    let status = failure.status.unwrap_or(206);
                    error.context(IncompleteDocument { status })
                } else {
                    error
                }
            })?;
        let response = Arc::unwrap_or_clone(response);
        Ok(Document {
            status: StatusCode::from_u16(response.status)?,
            headers: response.headers,
            body: Arc::new(response.body),
            payment_may_have_occurred: response.payment_may_have_occurred,
            origin: None,
        })
    }
}

pub(crate) async fn fallback(
    client: &PaidClient,
    options: &Options,
    failure: Failure,
) -> Result<Document> {
    if !failure.fallback {
        return Err(failure.error);
    }
    let Some(relay) = client.document_relay() else {
        return Err(failure.error);
    };
    let Some(prepared) = prepare_fallback(&relay, options)? else {
        return Err(failure.error);
    };
    prepared.acquire(client, failure).await
}

pub(crate) async fn read(client: &PaidClient, options: &Options) -> Result<Document> {
    match origin(options).await {
        Ok(document) => Ok(document),
        Err(failure) => fallback(client, options, failure).await,
    }
}
