//! Exercise the actual catalog schedulers, rather than bypassing their gates.
use super::*;
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse};
use std::{sync::Mutex, time::Duration};

#[derive(Clone)]
struct Origin {
    calls: Arc<Mutex<BTreeMap<String, usize>>>,
    rejected: Arc<tokio::sync::Barrier>,
    changed: Arc<tokio::sync::Notify>,
}
impl Origin {
    async fn seen(&self, path: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.calls.lock().unwrap().contains_key(path) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("healthy catalog queued behind Retry-After sleepers");
    }
}
async fn origin(
    State(state): State<Origin>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    let path = uri.path().to_owned();
    *state.calls.lock().unwrap().entry(path.clone()).or_default() += 1;
    state.changed.notify_waiters();
    if path.ends_with("/a") || path.ends_with("/b") {
        // Both configured slots are occupied before either rejected response
        // begins its indefinite server-directed wait.
        state.rejected.wait().await;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("retry-after", "18446744073709551615")],
            "unused",
        )
            .into_response();
    }
    if path == "/read" {
        return "healthy".into_response();
    }
    Json(serde_json::json!({"operations":[{"method":"GET","path":"/read"}]})).into_response()
}
fn config(mode: &str, socks: std::net::SocketAddr) -> String {
    let mut config = format!(
        "version=1\n[network]\nmode='tor'\nsocks_endpoint='{socks}'\n[startup]\ncatalog_concurrency=2\n[wallets.w]\nmode='static'\nprivate_key_env='KEY'\n"
    );
    for id in ["a", "b", "z"] {
        config.push_str(&format!(
            "[sources.{id}]\nspec='http://catalog.example.com/{mode}/{id}'\nbase_url='http://catalog.example.com'\n"
        ));
    }
    config.push_str("[servers.one]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='w'\nsources=['a','b','z']\n");
    config
}

#[tokio::test]
async fn retry_waits_release_catalog_slots_in_serving_inspection_and_warming() {
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .args([
            "--ignored",
            "--exact",
            "deployment::deferred::tor_catalog_capacity_tests::catalog_capacity_child",
            "--nocapture",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test]
#[ignore = "isolated process network policy; invoked by parent"]
async fn catalog_capacity_child() {
    let state = Origin {
        calls: Default::default(),
        rejected: Arc::new(tokio::sync::Barrier::new(2)),
        changed: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(origin).with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let socks = crate::test_socks::Socks::start(
        BTreeMap::from([("catalog.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let env = BTreeMap::from([
        ("KEY".into(), format!("{:064x}", 2)),
        ("TOKEN".into(), "fixture".into()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");

    // Ordinary serving publishes the queued healthy source while both rejected
    // catalogs retain their operation owners. Its tool is actually callable.
    std::fs::write(&path, config("serving", socks.address)).unwrap();
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    assert_eq!(deployment.pending_sources.len(), 3);
    let running = deployment.bind(&env).await.unwrap();
    let catalog = running.deferred.as_ref().unwrap().catalog.clone();
    let stop = CancellationToken::new();
    let serving = tokio::spawn(running.serve(stop.clone()));
    state.seen("/serving/z").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while catalog.read().views["one"].len() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let snapshot = catalog.read();
    assert!(
        snapshot.instructions["one"]
            .as_ref()
            .unwrap()
            .contains("PENDING")
    );
    assert!(snapshot.views["one"][0].tool.name.starts_with("z_"));
    assert_eq!(
        snapshot.views["one"][0]
            .invoke(&Default::default())
            .await
            .unwrap(),
        "healthy"
    );
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Strict inspection still waits for completeness, but its logical queue must
    // let the healthy source download. Warming uses the same attempt capacity.
    for mode in ["inspection", "warm"] {
        let text = config(mode, socks.address);
        std::fs::write(&path, &text).unwrap();
        let operation = if mode == "inspection" {
            let path = path.clone();
            tokio::spawn(async move { Deployment::load(&path).await.map(|_| ()) })
        } else {
            let cfg = toml::from_str(&text).unwrap();
            let path = path.clone();
            tokio::spawn(async move {
                startup::warm(
                    &cfg,
                    &path,
                    &BTreeSet::from(["a".into(), "b".into(), "z".into()]),
                    false,
                    Default::default(),
                )
                .await
                .map(|_| ())
            })
        };
        state.seen(&format!("/{mode}/z")).await;
        assert!(
            !operation.is_finished(),
            "strict work still requires delayed catalogs"
        );
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
    }
    for mode in ["serving", "inspection", "warm"] {
        for id in ["a", "b", "z"] {
            assert_eq!(
                state.calls.lock().unwrap()[&format!("/{mode}/{id}")],
                1,
                "server waits and cancellation grant no later dispatch"
            );
        }
    }
    server.abort();
}
