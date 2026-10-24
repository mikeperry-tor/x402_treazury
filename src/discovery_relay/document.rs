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

/// Trusted caller context, never derived from a destination or upstream content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RequestKind {
    #[default]
    Document,
    OpenApiProbe,
    DirectoryProbe,
    DocumentationProbe,
}

#[derive(Clone)]
pub(crate) struct Options {
    id: uuid::Uuid,
    pub kind: RequestKind,
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
            id: uuid::Uuid::new_v4(),
            kind: RequestKind::Document,
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

pub(crate) use crate::payment::{PossiblePayment, with_payment_context};

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

#[tracing::instrument(name = "document_request", level = "warn", skip_all,
    fields(document_id = %options.id, request_kind = ?options.kind))]
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
    let mut attempts = (0, 0);
    let result = acquisition::run(
        factory,
        selected.clone(),
        false,
        |current, number, maximum| {
            attempts = (number, maximum);
            async move {
                let policy = FallbackPolicy::BlockingFailures;
                crate::qualification::check_retry_authority()
                    .await
                    .map_err(Failure::local)?;
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
                let body = crate::limits::read_unlogged(
                    response,
                    options.limit,
                    "document",
                    options.setting,
                )
                .await
                .map_err(Failure::body)?;
                Ok((status, headers, body))
            }
        },
    )
    .await
    .inspect_err(|failure| log_origin_failure(options, failure, attempts))?;
    let (status, headers, body) = result.value;
    Ok(Document {
        status,
        headers,
        body: Arc::new(body),
        payment_may_have_occurred: false,
        origin: Some((selected, result.current)),
    })
}

fn failure_reason(error: &anyhow::Error, http_status: Option<u16>) -> &'static str {
    if error.is::<crate::limits::DownloadLimit>() {
        "document exceeds the download limit; content rejected without partial output"
    } else if error.is::<IncompleteDocument>() {
        "partial response rejected; a complete document is required"
    } else {
        match http_status {
            Some(401) => "service requires authentication",
            Some(403) => "service denied access",
            Some(404 | 410) => "document does not exist at this location",
            Some(429) => "service rate limit reached",
            Some(300..=399) => "redirect received; origin requests do not follow redirects",
            Some(500..=599) => "remote service returned a server error",
            Some(_) => "service returned an unsuccessful HTTP status",
            None => "document request or response failed validation or transport",
        }
    }
}

fn log_origin_failure(options: &Options, failure: &Failure, attempts: (usize, usize)) {
    let http_status = match failure.kind {
        acquisition::FailureKind::HttpStatus(status) => Some(status.as_u16()),
        _ => SharedFailure::capture(&failure.error, "document request failed").status(),
    };
    let limit = failure.error.downcast_ref::<crate::limits::DownloadLimit>();
    let reason = failure_reason(&failure.error, http_status);
    let transport_reason = failure
        .error
        .downcast_ref::<reqwest::Error>()
        .map(crate::payment::diagnostics::transport_reason);
    let (attempt, max_attempts) = attempts;
    let relay_fallback_eligible = failure.fallback;
    let failure_kind = failure.kind;
    let route = &network::global().policy.mode;
    let resource = limit.map(|limit| limit.kind);
    let setting = limit.map(|limit| limit.setting);
    let limit_bytes = limit.map(|limit| limit.limit);
    if options.kind == RequestKind::Document {
        tracing::info!(
            http_status,
            reason,
            transport_reason,
            ?failure_kind,
            ?route,
            attempt,
            max_attempts,
            relay_fallback_eligible,
            resource,
            setting,
            limit_bytes,
            "Could not retrieve document from its origin; see status and reason"
        );
    } else {
        tracing::debug!(
            http_status,
            reason,
            transport_reason,
            ?failure_kind,
            ?route,
            attempt,
            max_attempts,
            relay_fallback_eligible,
            resource,
            setting,
            limit_bytes,
            "Optional discovery probe did not retrieve a document; candidate URLs may be unavailable"
        );
    }
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
    if !relay.enabled(super::Purpose::ProviderMetadata) {
        return Ok(None);
    }
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
    #[tracing::instrument(name = "document_request", level = "warn", skip_all,
        fields(document_id = %self.options.id, request_kind = ?self.options.kind))]
    pub(crate) async fn acquire(self, client: &PaidClient, failure: Failure) -> Result<Document> {
        if !failure.fallback || !client.has_document_authority() {
            return Err(failure.error);
        }
        let options = self.options;
        if options.kind == RequestKind::Document {
            tracing::info!("Configured paid relay selected as fallback to retrieve this document");
        }
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
    let result = match origin(options).await {
        Ok(document) => Ok(document),
        Err(failure) => fallback(client, options, failure).await,
    };
    result.inspect_err(|error| log_failure(options, error))
}

/// Only the complete acquisition owner knows whether fallback was unavailable
/// or exhausted. Origin and individual relay attempts are intermediate events.
pub(crate) fn log_failure(options: &Options, error: &anyhow::Error) {
    let failure = SharedFailure::capture(error, "document acquisition failed");
    let http_status = failure.status();
    let authorization_may_charge = failure.possible_payment();
    let reason = error
        .downcast_ref::<TargetFailure>()
        .map(|f| f.reason)
        .or_else(|| {
            error
                .downcast_ref::<super::RelayFailure>()
                .map(|f| f.reason)
        })
        .unwrap_or_else(|| failure_reason(error, http_status));
    let transport_reason = error
        .downcast_ref::<reqwest::Error>()
        .map(crate::payment::diagnostics::transport_reason);
    let route = &network::global().policy.mode;
    let limit = error.downcast_ref::<crate::limits::DownloadLimit>();
    let resource = limit.map(|limit| limit.kind);
    let setting = limit.map(|limit| limit.setting);
    let limit_bytes = limit.map(|limit| limit.limit);
    if options.kind == RequestKind::Document {
        tracing::warn!(
            document_id = %options.id,
            request_kind = ?options.kind,
            http_status,
            reason,
            transport_reason,
            ?route,
            resource,
            setting,
            limit_bytes,
            authorization_may_charge,
            "Could not retrieve document; paid fallback unavailable or failed"
        );
    } else {
        tracing::debug!(
            document_id = %options.id,
            request_kind = ?options.kind,
            http_status,
            reason,
            transport_reason,
            ?route,
            resource,
            setting,
            limit_bytes,
            authorization_may_charge,
            "Optional discovery probe exhausted document retrieval"
        );
    }
}

#[cfg(test)]
#[path = "document_diagnostics_tests.rs"]
mod diagnostics_tests;
