//! One unsigned origin sequence. Consumers own validation, promotion and handoff.
use super::{HttpTransport, NetworkContext, RetryAfter, SelectionPolicy};
use reqwest::{StatusCode, header::HeaderMap};
use std::{future::Future, sync::Arc};

/// Static discovery preserves its broader remote HTTP/content fallback contract.
/// Runtime documents and API reads fall back only after blocking delivery failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackPolicy {
    RemoteFailures,
    BlockingFailures,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Transport,
    HttpStatus(StatusCode),
    RemoteContent,
    Local,
    Validation,
}

/// Original errors remain typed; display strings never decide replay authority.
#[derive(Debug)]
pub(crate) struct Failure {
    pub error: anyhow::Error,
    pub kind: FailureKind,
    pub retry: bool,
    pub fallback: bool,
    /// The final server delay belongs to the relay continuation, if any.
    pub retry_after: Option<RetryAfter>,
}
impl Failure {
    pub fn transport(error: reqwest::Error) -> Self {
        let eligible = super::eligible_retry_transport(&error);
        Self {
            error: error.into(),
            kind: FailureKind::Transport,
            retry: eligible,
            fallback: eligible,
            retry_after: None,
        }
    }

    /// Bounded body readers retain Reqwest transport causes; bounds and other
    /// local reader errors never become permission to buy another response.
    pub fn body(error: anyhow::Error) -> Self {
        match error.downcast::<reqwest::Error>() {
            Ok(error) => Self::transport(error),
            Err(error) => Self::local(error),
        }
    }

    pub fn status(
        error: anyhow::Error,
        status: StatusCode,
        headers: &HeaderMap,
        policy: FallbackPolicy,
    ) -> Self {
        Self {
            error,
            kind: FailureKind::HttpStatus(status),
            retry: super::eligible_retry_status(status),
            fallback: policy == FallbackPolicy::RemoteFailures
                || matches!(status.as_u16(), 400 | 403 | 408 | 429)
                || status.is_server_error(),
            retry_after: super::parse_retry_after(status, headers).map(RetryAfter::after),
        }
    }

    /// Call only for complete content whose consumer explicitly permits another
    /// acquisition. Invalid runtime documents/output remain terminal validation.
    pub fn remote_content(error: anyhow::Error, policy: FallbackPolicy) -> Self {
        Self {
            error,
            kind: FailureKind::RemoteContent,
            retry: false,
            fallback: policy == FallbackPolicy::RemoteFailures,
            retry_after: None,
        }
    }

    /// Construction, destination, authority and bounds failures cannot buy a relay.
    pub fn local(error: anyhow::Error) -> Self {
        Self {
            error,
            kind: FailureKind::Local,
            retry: false,
            fallback: false,
            retry_after: None,
        }
    }

    pub fn validation(error: anyhow::Error) -> Self {
        Self {
            kind: FailureKind::Validation,
            ..Self::local(error)
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, f)
    }
}
impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

#[derive(Debug)]
pub(crate) struct Acquired<T> {
    pub value: T,
    /// Promotion requires the consumer's accepted, complete response validation.
    pub current: Arc<HttpTransport>,
}

/// The callback performs exactly one attempt, including complete-body acquisition
/// where required, and releases active-I/O permits before returning. Its retained
/// caller owns cancellation and checks any per-attempt authority before dispatch.
/// A terminal delay is returned intact; only the eventual handoff may consume it.
pub(crate) async fn run<T, F, Fut>(
    network: &NetworkContext,
    selected: Arc<HttpTransport>,
    direct_override: bool,
    mut attempt: F,
) -> Result<Acquired<T>, Failure>
where
    F: FnMut(Arc<HttpTransport>, usize, usize) -> Fut,
    Fut: Future<Output = Result<T, Failure>>,
{
    let max = if selected.selection() == SelectionPolicy::Participating {
        network.max_http_attempts(direct_override)
    } else {
        1
    };
    for number in 1..=max {
        let current = network
            .http_attempt(&selected, number)
            .map_err(Failure::local)?;
        match attempt(current.clone(), number, max).await {
            Ok(value) => return Ok(Acquired { value, current }),
            Err(failure) if failure.retry && number < max => {
                super::sleep_retry_after(failure.retry_after).await;
            }
            Err(failure) => return Err(failure),
        }
    }
    unreachable!("validated origin count is positive")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{HttpPolicy, IsolationId, Mode, NetworkPolicy};
    use std::time::Duration;

    fn context(retries: Option<usize>) -> NetworkContext {
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some("127.0.0.1:9150".parse().unwrap()),
            tor_isolation_retries: retries,
            ..Default::default()
        })
        .unwrap()
    }

    fn selected(network: &NetworkContext, participating: bool) -> Arc<HttpTransport> {
        let url = "https://example.com/read";
        network
            .http_transport(
                &IsolationId::discovery(url).unwrap(),
                url,
                None,
                true,
                HttpPolicy::default(),
                if participating {
                    SelectionPolicy::Participating
                } else {
                    SelectionPolicy::Legacy
                },
            )
            .unwrap()
    }

    fn status(status: u16, policy: FallbackPolicy) -> Failure {
        Failure::status(
            anyhow::anyhow!("fixture status"),
            StatusCode::from_u16(status).unwrap(),
            &HeaderMap::new(),
            policy,
        )
    }

    #[tokio::test]
    async fn configured_count_has_one_sequence_with_fresh_owner_bound_candidates() {
        for (retries, count) in [(Some(0), 1), (Some(1), 2), (None, 3)] {
            let network = context(retries);
            let initial = selected(&network, true);
            let mut scopes = Vec::new();
            let result = run(&network, initial.clone(), false, |current, number, max| {
                assert_eq!(max, count);
                assert_eq!(number, scopes.len() + 1);
                assert_eq!(current.identity, initial.identity);
                assert_eq!(current.origin(), initial.origin());
                assert!(!scopes.contains(&current.scope));
                scopes.push(current.scope.clone());
                async { Err::<(), _>(status(503, FallbackPolicy::BlockingFailures)) }
            })
            .await;
            assert_eq!(scopes.len(), count);
            assert!(result.unwrap_err().fallback);
        }
    }

    #[tokio::test]
    async fn direct_override_legacy_and_direct_operations_each_attempt_once() {
        for (network, participating, direct_override) in [
            (context(None), true, true),
            (context(None), false, false),
            (
                NetworkContext::new(NetworkPolicy::default()).unwrap(),
                true,
                false,
            ),
        ] {
            let mut calls = 0;
            let result = run(
                &network,
                selected(&network, participating),
                direct_override,
                |_, _, max| {
                    calls += 1;
                    assert_eq!(max, 1);
                    async { Err::<(), _>(status(503, FallbackPolicy::BlockingFailures)) }
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(calls, 1);
        }
    }

    #[tokio::test]
    async fn complete_success_and_terminal_validation_never_repeat_or_promote() {
        let network = context(None);
        let initial = selected(&network, true);
        let mut calls = 0;
        let acquired = run(&network, initial.clone(), false, |_, _, _| {
            calls += 1;
            async { Ok::<_, Failure>(Vec::<u8>::new()) }
        })
        .await
        .unwrap();
        assert_eq!(calls, 1);
        assert!(acquired.value.is_empty());
        assert!(Arc::ptr_eq(&initial, &acquired.current));
        let result = run(&network, initial, false, |_, _, _| {
            calls += 1;
            async {
                Err::<(), _>(Failure::validation(anyhow::anyhow!(
                    "complete invalid document"
                )))
            }
        })
        .await;
        assert_eq!(calls, 2);
        assert!(!result.unwrap_err().fallback);
    }

    #[tokio::test]
    async fn successful_candidate_requires_consumer_promotion() {
        let network = context(Some(1));
        let initial = selected(&network, true);
        let acquired = run(
            &network,
            initial.clone(),
            false,
            |_, number, _| async move {
                if number == 1 {
                    Err(status(503, FallbackPolicy::BlockingFailures))
                } else {
                    Ok(())
                }
            },
        )
        .await
        .unwrap();
        assert_ne!(initial.scope, acquired.current.scope);
        assert!(Arc::ptr_eq(&initial, &selected(&network, true)));
    }

    #[test]
    fn body_bounds_and_request_construction_errors_preserve_terminal_typed_causes() {
        let failure = Failure::body(crate::limits::exceeded("fixture body", "max_spec_bytes", 1));
        assert_eq!(failure.kind, FailureKind::Local);
        assert!(!failure.retry && !failure.fallback);
        assert!(failure.error.is::<crate::limits::DownloadLimit>());

        let error = crate::network::global()
            .discovery("https://example.com", None)
            .unwrap()
            .get("invalid URL")
            .build()
            .unwrap_err();
        let failure = Failure::transport(error);
        assert_eq!(failure.kind, FailureKind::Transport);
        assert!(!failure.retry && !failure.fallback);
        assert!(
            failure
                .error
                .downcast_ref::<reqwest::Error>()
                .unwrap()
                .is_builder()
        );
    }

    #[test]
    fn consumer_status_and_content_contracts_are_distinct_from_origin_retries() {
        for code in [400, 403, 408, 429, 500, 502, 503, 504] {
            let failure = status(code, FallbackPolicy::BlockingFailures);
            assert!(failure.retry && failure.fallback, "{code}");
        }
        for code in [501, 505, 520] {
            let failure = status(code, FallbackPolicy::BlockingFailures);
            assert!(!failure.retry && failure.fallback, "{code}");
        }
        for code in [200, 204, 206, 302, 401, 402, 404, 405, 410, 422] {
            let failure = status(code, FallbackPolicy::BlockingFailures);
            assert!(!failure.retry && !failure.fallback, "{code}");
            assert!(status(code, FallbackPolicy::RemoteFailures).fallback);
        }
        for policy in [
            FallbackPolicy::RemoteFailures,
            FallbackPolicy::BlockingFailures,
        ] {
            assert!(!Failure::local(anyhow::anyhow!("authority")).fallback);
            let failure = Failure::remote_content(anyhow::anyhow!("complete invalid JSON"), policy);
            assert!(!failure.retry);
            assert_eq!(failure.fallback, policy == FallbackPolicy::RemoteFailures);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retry_delay_is_between_attempts_and_final_delay_is_returned_for_handoff() {
        let network = context(Some(1));
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "7".parse().unwrap());
        let started = tokio::time::Instant::now();
        let result = run(&network, selected(&network, true), false, |_, number, _| {
            let failure = Failure::status(
                anyhow::anyhow!("fixture unavailable"),
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                FallbackPolicy::BlockingFailures,
            );
            async move { if number == 1 { Err(failure) } else { Ok(()) } }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(started.elapsed(), Duration::from_secs(7));

        let terminal_started = tokio::time::Instant::now();
        let failure = run(&network, selected(&network, true), true, |_, _, _| {
            let failure = Failure::status(
                anyhow::anyhow!("fixture unavailable"),
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                FallbackPolicy::BlockingFailures,
            );
            async move { Err::<(), _>(failure) }
        })
        .await
        .unwrap_err();
        assert!(failure.retry_after.is_some());
        assert_eq!(terminal_started.elapsed(), Duration::ZERO);
    }
}
