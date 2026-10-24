//! Runtime API relay handoff; the existing call owns acquisition and conversion.
use super::{adapters::TargetRequest, sequence::Contract};
use crate::{
    network::acquisition::Failure,
    output::HttpOutput,
    payment::{PaidClient, SubmissionProgress},
};
use anyhow::Result;
use std::sync::Arc;

// Relay payment uses PaidClient again with recursive fallback disabled. Erase
// only this indirect future's type, retaining the existing call owner and Send.
pub(crate) fn fallback<'a>(
    client: &'a PaidClient,
    request: &'a reqwest::Request,
    failure: Failure,
    progress: &'a SubmissionProgress,
    capacity: &'a [Arc<tokio::sync::Semaphore>],
) -> futures_util::future::BoxFuture<'a, Result<HttpOutput>> {
    Box::pin(async move {
        if !failure.fallback || !client.has_document_authority() {
            return Err(failure.error);
        }
        let Some(relay) = client.request_relay() else {
            return Err(failure.error);
        };
        let body = match request.body() {
            Some(body) => match body.as_bytes() {
                Some(bytes) => Some(bytes.to_vec()),
                None => return Err(failure.error),
            },
            None => None,
        };
        let target = TargetRequest {
            method: request.method().clone(),
            url: request.url().clone(),
            headers: request.headers().clone(),
            body,
            allowed_origins: Vec::new(),
        };
        let prepared = relay.prepare(&target, client.response_limit())?;
        if prepared.is_empty() {
            return Err(failure.error);
        }
        let result = prepared
            .acquire(
                client.clone(),
                &target,
                Contract::Api,
                client.response_limit(),
                "max_response_bytes",
                Some(client.response_limit()),
                client.max_help_bytes(),
                client.timeout(),
                capacity,
                None,
                failure.retry_after,
            )
            .await;
        if result.as_ref().is_ok_and(|r| r.payment_may_have_occurred)
            || result
                .as_ref()
                .is_err_and(|e| e.is::<super::document::PossiblePayment>())
        {
            progress.mark_possible();
        }
        let response = Arc::unwrap_or_clone(result?);
        let mime_type = if request.method() == reqwest::Method::HEAD {
            Some("text/plain".into())
        } else {
            response
                .headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap().trim().to_ascii_lowercase())
        };
        Ok(HttpOutput {
            bytes: response.body,
            mime_type,
            paid_submission: false,
            relay_payment_may_have_occurred: response.payment_may_have_occurred,
            advisories: if response.payment_may_have_occurred {
                vec![
                    "This call may have charged; do not automatically repeat this logical call."
                        .into(),
                ]
            } else {
                Vec::new()
            },
        })
    })
}
