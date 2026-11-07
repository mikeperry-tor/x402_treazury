//! View acquisition details stay at debug; the requesting workflow owns failure severity.
use super::*;
use axum::http::StatusCode;
use tracing::instrument::WithSubscriber;

async fn endpoint(status: StatusCode) -> (String, Requests, tokio::task::JoinHandle<()>) {
    let calls = Requests::default();
    let counter = calls.clone();
    let app = Router::new().route(
        "/private-path",
        post(move |Json(request): Json<Value>| {
            counter.lock().unwrap().push(request);
            async move {
                (
                    status,
                    if status.is_success() {
                        format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x{:064x}\"}}",
                            0
                        )
                    } else {
                        "private upstream body".into()
                    },
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/private-path?key=private-key",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, calls, server)
}

#[tokio::test]
async fn balance_fallback_is_quiet_at_info_and_has_safe_provider_context_at_debug() {
    if crate::test_log_capture::run_in_child(
        "rotation::base::view_tests::logging::balance_fallback_is_quiet_at_info_and_has_safe_provider_context_at_debug",
    ) {
        return;
    }
    let (primary, rejected, first) = endpoint(StatusCode::FORBIDDEN).await;
    let (secondary, accepted, second) = endpoint(StatusCode::OK).await;
    let rpc = BaseRpc::with_fallbacks(&[primary, secondary]).unwrap();
    let logs = crate::rotation::base::tests::Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_env_filter("warn,x402_treazury::rotation::base=debug")
        .with_writer(move || writer.clone())
        .finish();
    let address = format!("0x{:040x}", 1);
    assert_eq!(
        rpc.current_balance(&address)
            .with_subscriber(subscriber)
            .await
            .unwrap(),
        U256::ZERO
    );
    assert_eq!(rejected.lock().unwrap().len(), 1);
    assert_eq!(accepted.lock().unwrap().len(), 1);
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("DEBUG"), "{logged}");
    for excluded in [
        "INFO",
        "WARN",
        "ERROR",
        "private",
        "127.0.0.1",
        address.as_str(),
    ] {
        assert!(!logged.contains(excluded), "{logged}");
    }
    for field in [
        "HTTP 403",
        "provider_index=1",
        "provider_index=2",
        "provider_count=2",
        "failover=true",
    ] {
        assert!(logged.contains(field), "missing {field}: {logged}");
    }
    first.abort();
    second.abort();
}

#[tokio::test]
async fn view_failure_stays_at_debug_for_the_requesting_workflow() {
    if crate::test_log_capture::run_in_child(
        "rotation::base::view_tests::logging::view_failure_stays_at_debug_for_the_requesting_workflow",
    ) {
        return;
    }
    for healthy_fallback in [true, false] {
        let (primary, rejected, first) = endpoint(StatusCode::FORBIDDEN).await;
        let (secondary, requests, second) = if healthy_fallback {
            let (healthy, requests, server, _stamp) = fixture(false).await;
            (healthy.url.to_string(), requests, server)
        } else {
            endpoint(StatusCode::FORBIDDEN).await
        };
        let rpc = BaseRpc::with_fallbacks(&[primary, secondary]).unwrap();
        let logs = crate::rotation::base::tests::Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_env_filter("warn,x402_treazury::rotation::base=debug")
            .with_writer(move || writer.clone())
            .finish();
        let result = rpc.view(query(false)).with_subscriber(subscriber).await;
        assert_eq!(rejected.lock().unwrap().len(), 1);
        if healthy_fallback {
            let view = result.unwrap();
            assert_eq!(view.anchor.height, 88);
            assert_eq!(view.balances["wallet"], U256::from(10));
            assert_eq!(
                requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request["method"] == "eth_call")
                    .count(),
                2
            );
        } else {
            let error = format!("{:#}", result.err().unwrap());
            assert!(error.contains("403"), "{error}");
            assert!(
                !error.contains("private") && !error.contains("127.0.0.1"),
                "{error}"
            );
            assert_eq!(requests.lock().unwrap().len(), 1);
        }
        let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("DEBUG"), "{logged}");
        assert!(
            !logged.contains("WARN") && !logged.contains("ERROR") && !logged.contains("INFO"),
            "{logged}"
        );
        if !healthy_fallback {
            for field in [
                "provider_index=2",
                "provider_count=2",
                "failover=false",
                "HTTP 403",
            ] {
                assert!(logged.contains(field), "{logged}");
            }
        }
        for secret in ["private", "127.0.0.1"] {
            assert!(!logged.contains(secret), "{logged}");
        }
        first.abort();
        second.abort();
    }
}
