//! API retry scheduling through retained application owners, using fixture SOCKS.
use super::*;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

#[tokio::test]
async fn tor_retry_wait_preserves_ownership_and_rechecks_authority() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "payment::tor_wait_tests::tor_retry_wait_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[derive(Clone, Default)]
struct Observations {
    requests: Arc<Mutex<BTreeMap<String, Vec<tokio::time::Instant>>>>,
    changed: Arc<tokio::sync::Notify>,
    release_recovery: Arc<tokio::sync::Notify>,
}

impl Observations {
    fn count(&self, path: &str) -> usize {
        self.requests.lock().unwrap().get(path).map_or(0, Vec::len)
    }

    async fn wait_for(&self, path: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.count(path) >= count {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("fixture request did not arrive");
    }
}

async fn handler(State(observed): State<Observations>, uri: Uri, headers: HeaderMap) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    let ordinal = {
        let mut requests = observed.requests.lock().unwrap();
        let times = requests.entry(uri.path().to_owned()).or_default();
        times.push(tokio::time::Instant::now());
        times.len()
    };
    observed.changed.notify_waiters();
    if uri.path().starts_with("/retained") && ordinal == 2 {
        observed.release_recovery.notified().await;
    }
    if matches!(uri.path(), "/beyond-429" | "/beyond-503") && ordinal == 1 {
        let status = if uri.path() == "/beyond-429" {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        return (
            status,
            [("retry-after", "18446744073709551615")],
            "unused error body",
        )
            .into_response();
    }
    if uri.path() == "/exhausted" {
        return if ordinal == 3 {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "18446744073709551615")],
                "unused final error body",
            )
                .into_response()
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, "unused error body").into_response()
        };
    }
    if uri.path() != "/independent" && ordinal == 1 {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            "unused error body",
        )
            .into_response()
    } else if uri.path() == "/retained-invalid" {
        (StatusCode::OK, [("content-type", "text/plain")], vec![0xff]).into_response()
    } else {
        (StatusCode::OK, [("content-type", "text/plain")], "free").into_response()
    }
}

fn route(path: &str) -> RoutedRequest {
    RoutedRequest {
        method: "POST".into(),
        url: format!("http://wait-api.example.com{path}"),
        query: BTreeMap::new(),
        body: Some(serde_json::json!({"exact":"business argument"})),
    }
}

fn client() -> PaidClient {
    PaidClient::new(
        Payer::new(&format!("{:064x}", 71), SpendPolicy::dollars("1").unwrap()).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor policy; invoked by parent"]
async fn tor_retry_wait_child() {
    use crate::{
        network::{Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    let observed = Observations::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(handler).with_state(observed.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let proxy = Socks::start(
        BTreeMap::from([("wait-api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            ..Default::default()
        })
        .unwrap(),
    );

    // A disconnected waiter relinquishes delivery, while its accepted owner
    // honors Retry-After, consumes the free result and remains visible to drain.
    let retained = client();
    let identity =
        crate::network::IsolationId::evm(&retained.payer.as_ref().unwrap().read().unwrap().address)
            .unwrap();
    let selected = || {
        crate::network::global()
            .http_participating(
                &identity,
                "http://wait-api.example.com",
                None,
                false,
                Default::default(),
            )
            .unwrap()
    };
    let original = selected();
    let waiter_client = retained.clone();
    let waiter =
        tokio::spawn(async move { waiter_client.execute_response(route("/retained")).await });
    observed.wait_for("/retained", 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(observed.count("/retained"), 1);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());

    // New work on a separate client can finish while that owner is sleeping.
    let independent = client();
    let output = tokio::time::timeout(
        Duration::from_millis(500),
        independent.execute_response(route("/independent")),
    )
    .await
    .expect("Retry-After blocked unrelated API work")
    .unwrap();
    assert_eq!(output.bytes, b"free");
    assert_eq!(observed.count("/retained"), 1);
    observed.release_recovery.notify_one();
    retained.drain().await;
    assert_eq!(observed.count("/retained"), 2);
    let winner = selected();
    assert!(!Arc::ptr_eq(&original, &winner));
    assert_eq!(
        proxy.records.lock().unwrap().last().unwrap().password,
        crate::network::global().credentials(&winner.scope).1,
        "the disconnected response waiter must retain the successful candidate"
    );
    {
        let requests = observed.requests.lock().unwrap();
        let times = &requests["/retained"];
        assert!(times[1].duration_since(times[0]) >= Duration::from_secs(1));
    }

    // Text and configured-output conversion belong to that same retained
    // workflow, rather than a delivery continuation that cancellation can lose.
    for path in ["/retained-text", "/retained-rendered", "/retained-invalid"] {
        let original = selected();
        let retained = client();
        let waiter_client = retained.clone();
        let waiter = tokio::spawn(async move {
            if path != "/retained-rendered" {
                assert_eq!(waiter_client.execute(route(path)).await.unwrap(), "free");
            } else {
                let output = waiter_client
                    .execute_rendered_scoped(route(path), None, None, &Default::default())
                    .await
                    .unwrap();
                assert_eq!(output.text, "free");
            }
        });
        observed.wait_for(path, 1).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        observed.release_recovery.notify_one();
        retained.drain().await;
        assert_eq!(observed.count(path), 2);
        let winner = selected();
        if path == "/retained-invalid" {
            assert!(
                Arc::ptr_eq(&original, &winner),
                "rejected text cannot promote"
            );
            continue;
        }
        assert!(!Arc::ptr_eq(&original, &winner));
        assert_eq!(
            proxy.records.lock().unwrap().last().unwrap().password,
            crate::network::global().credentials(&winner.scope).1
        );
    }

    // Terminate the workflow INSIDE its retained owner. Nested execution reuses
    // that owner; dropping this nested future really cancels unsigned dispatch.
    let terminated = client();
    let owner_client = terminated.clone();
    let nested_client = terminated.clone();
    let cancel = Arc::new(tokio::sync::Notify::new());
    let owner_cancel = cancel.clone();
    let owner = tokio::spawn(async move {
        owner_client
            .run_owned(async move {
                tokio::select! {
                    result = nested_client.execute_response(route("/terminated")) => result,
                    _ = owner_cancel.notified() => Err(anyhow::anyhow!("fixture owner terminated")),
                }
            })
            .await
    });
    observed.wait_for("/terminated", 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.notify_one();
    let error = owner.await.unwrap().unwrap().unwrap_err();
    assert!(error.to_string().contains("fixture owner terminated"));
    terminated.drain().await;

    // A valid delay beyond the monotonic clock's range still forbids a later
    // dispatch. Keep both eligible status owners alive through other real work,
    // then terminate their nested workflows rather than merely their waiters.
    let mut beyond_clock = Vec::new();
    for path in ["/beyond-429", "/beyond-503"] {
        let client = client();
        let owner_client = client.clone();
        let nested_client = client.clone();
        let cancel = Arc::new(tokio::sync::Notify::new());
        let owner_cancel = cancel.clone();
        let owner = tokio::spawn(async move {
            owner_client
                .run_owned(async move {
                    tokio::select! {
                        result = nested_client.execute_response(route(path)) => result,
                        _ = owner_cancel.notified() => Err(anyhow::anyhow!("fixture owner terminated")),
                    }
                })
                .await
        });
        observed.wait_for(path, 1).await;
        beyond_clock.push((path, client, cancel, owner));
    }
    let output = tokio::time::timeout(
        Duration::from_millis(500),
        independent.execute_response(route("/independent")),
    )
    .await
    .expect("unrepresentable Retry-After blocked unrelated API work")
    .unwrap();
    assert_eq!(output.bytes, b"free");

    // No future attempt is authorized after exhaustion, so a final enormous
    // Retry-After is discarded with that error response instead of waiting.
    let exhausted = client();
    let error = tokio::time::timeout(
        Duration::from_millis(500),
        exhausted.execute_response(route("/exhausted")),
    )
    .await
    .expect("final exhausted status must not honor an unused Retry-After")
    .unwrap_err();
    assert!(format!("{error:#}").contains("503"), "{error:#}");
    assert_eq!(observed.count("/exhausted"), 3);
    exhausted.drain().await;

    // The qualification claim is valid at entry and expires during the actual
    // Retry-After wait. Retaining its owner cannot renew that dispatch authority.
    let (_qualification_dir, claim, path) = crate::qualification::managed_claim_fixture().await;
    let qualified = client();
    let qualified_client = qualified.clone();
    let qualification = tokio::spawn(async move {
        claim
            .invoke(qualified_client.execute_response(route("/qualified")))
            .await
    });
    observed.wait_for("/qualified", 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(observed.count("/qualified"), 1);
    rusqlite::Connection::open(path)
        .unwrap()
        .execute("UPDATE cases SET window_end=0", [])
        .unwrap();
    let error = qualification.await.unwrap().unwrap_err();
    assert!(format!("{error:#}").contains("qualification"), "{error:#}");
    qualified.drain().await;
    // Qualification's one-second wait also passes the terminated owner's full
    // retry interval, so absence of its second dispatch is observable evidence.
    assert_eq!(observed.count("/qualified"), 1);
    assert_eq!(observed.count("/terminated"), 1);
    for (path, client, cancel, owner) in beyond_clock {
        assert_eq!(observed.count(path), 1);
        assert!(!owner.is_finished(), "{path} must remain pending");
        cancel.notify_one();
        let error = owner.await.unwrap().unwrap().unwrap_err();
        assert!(error.to_string().contains("fixture owner terminated"));
        client.drain().await;
        assert_eq!(observed.count(path), 1);
    }
    assert_eq!(observed.count("/independent"), 2);
    independent.drain().await;
    server.abort();
    let _ = server.await;
}
