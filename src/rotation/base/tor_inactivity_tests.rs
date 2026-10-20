//! Tor recovery distinguishes renewable response progress from true inactivity.
use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, Uri},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Fixture {
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    sibling_started: Arc<Semaphore>,
}
fn result() -> Value {
    json!({"jsonrpc":"2.0","id":1,"result":format!("0x{:064x}",17)})
}
async fn endpoint(
    State(f): State<Fixture>,
    uri: Uri,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    assert_eq!(payload["method"], "eth_call");
    assert_eq!(payload["params"][1], "latest");
    let path = uri.path().to_owned();
    let ordinal = {
        let mut requests = f.requests.lock().unwrap();
        requests.push((path.clone(), payload));
        requests.iter().filter(|(p, _)| p == &path).count()
    };
    if path == "/stall" && ordinal == 1 {
        let prefix = futures_util::stream::once(async {
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"{\"jsonrpc\":"))
        });
        let pending =
            futures_util::stream::pending::<Result<axum::body::Bytes, std::convert::Infallible>>();
        return axum::body::Body::from_stream(prefix.chain(pending)).into_response();
    }
    if matches!(path.as_str(), "/slow" | "/sibling") {
        let bytes = serde_json::to_vec(&result()).unwrap();
        let chunk_size = if path == "/slow" {
            bytes.len().div_ceil(4)
        } else {
            bytes.len().div_ceil(11)
        };
        let chunks = bytes
            .chunks(chunk_size)
            .map(axum::body::Bytes::copy_from_slice)
            .enumerate()
            .collect::<Vec<_>>();
        let sibling = path == "/sibling";
        let interval = if sibling { 200 } else { 400 };
        let stream = futures_util::stream::iter(chunks).then(move |(index, chunk)| {
            let started = f.sibling_started.clone();
            async move {
                if index > 0 {
                    tokio::time::sleep(Duration::from_millis(interval)).await;
                } else if sibling {
                    started.add_permits(1);
                }
                Ok::<_, std::convert::Infallible>(chunk)
            }
        });
        return axum::body::Body::from_stream(stream).into_response();
    }
    Json(result()).into_response()
}
fn rpc(path: &str) -> BaseRpc {
    BaseRpc::new(&format!("https://api.example.com{path}"), 12).unwrap()
}
fn selected(payer: &str) -> Arc<crate::network::HttpTransport> {
    crate::network::global()
        .http_participating(
            &crate::network::IsolationId::evm(payer).unwrap(),
            "https://api.example.com",
            None,
            false,
            crate::network::HttpPolicy::COMPATIBLE,
        )
        .unwrap()
}
#[tokio::test]
async fn tor_base_progress_and_true_inactivity() {
    let result = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "rotation::base::tor_inactivity_tests::tor_base_inactivity_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "private Tor inactivity policy; invoked by parent"]
async fn tor_base_inactivity_child() {
    use crate::network::{Mode, NetworkContext, NetworkPolicy};
    let f = Fixture {
        requests: Default::default(),
        sibling_started: Arc::new(Semaphore::new(0)),
    };
    let app = Router::new().fallback(endpoint).with_state(f.clone());
    let (address, server) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS13], &[b"h2"]).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            read_timeout_seconds: Some(1),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let payer = Address::repeat_byte(7).to_string();
    let original = selected(&payer);
    let started = tokio::time::Instant::now();
    assert_eq!(
        rpc("/slow").current_balance(&payer).await.unwrap(),
        U256::from(17)
    );
    assert!(started.elapsed() > Duration::from_secs(1));
    assert!(Arc::ptr_eq(&original, &selected(&payer)));
    assert_eq!(
        f.requests.lock().unwrap().len(),
        1,
        "progress cannot earn an isolation retry"
    );
    assert_eq!(proxy.records.lock().unwrap().len(), 1);

    let sibling = tokio::spawn({
        let payer = payer.clone();
        async move { rpc("/sibling").current_balance(&payer).await }
    });
    tokio::time::timeout(Duration::from_secs(5), f.sibling_started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let stalled = tokio::time::Instant::now();
    assert_eq!(
        rpc("/stall").current_balance(&payer).await.unwrap(),
        U256::from(17)
    );
    assert!(stalled.elapsed() >= Duration::from_secs(1));
    assert!(
        !sibling.is_finished(),
        "sibling progress must survive recovery on another physical scope"
    );
    let winner = selected(&payer);
    assert!(!Arc::ptr_eq(&original, &winner));
    {
        let reads = f.requests.lock().unwrap();
        let stalled_reads = reads
            .iter()
            .filter(|(p, _)| p == "/stall")
            .collect::<Vec<_>>();
        assert_eq!(
            stalled_reads.len(),
            2,
            "true inactivity recovers once before success"
        );
        assert_eq!(
            stalled_reads[0].1, stalled_reads[1].1,
            "retry preserves exact read payload"
        );
        assert_eq!(reads.iter().filter(|(p, _)| p == "/sibling").count(), 1);
    }
    {
        let records = proxy.records.lock().unwrap();
        assert_eq!(
            records.len(),
            2,
            "candidate has its own SOCKS connection while the original HTTP/2 stream progresses"
        );
        assert_ne!(records[0].password, records[1].password);
    }
    assert_eq!(sibling.await.unwrap().unwrap(), U256::from(17));
    assert!(
        Arc::ptr_eq(&winner, &selected(&payer)),
        "late original success cannot restore its scope"
    );
    server.abort();
    let _ = server.await;
}
