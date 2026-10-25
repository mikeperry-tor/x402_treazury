use super::*;
use crate::discovery::policy::RemoteDirectory;

#[test]
fn grouped_output_preserves_native_objects_and_payment_after_later_errors() {
    use crate::output::{InlineImage, ToolOutput};
    let native =
        json!({"items":[{"url":"same"},{"url":"same"}],"partial":true,"unknown":{"count":null}});
    let mut paid = ToolOutput::text(native.to_string());
    paid.payment_may_have_occurred = true;
    paid.advisories.push("original relay evidence".into());
    let mut invalid = ToolOutput::text("not JSON with credential-bearing prose".into());
    invalid.payment_may_have_occurred = true;
    invalid.images.push(InlineImage {
        data: String::new(),
        mime_type: "image/png".into(),
    });
    let result = super::super::remote::aggregate(vec![
        (RemoteDirectory::Coinbase, Ok(paid)),
        (RemoteDirectory::CoinbaseCurated, Ok(invalid)),
        (
            RemoteDirectory::X402List,
            Err(anyhow::anyhow!("private upstream prose")),
        ),
    ]);
    let parsed: Value = serde_json::from_str(&result.text).unwrap();
    assert_eq!(parsed["coinbase"], native);
    assert!(result.payment_may_have_occurred);
    assert_eq!(
        parsed["coinbase_curated"]["local_error"]["payment_may_have_occurred"],
        true
    );
    assert!(parsed["x402-list"]["local_error"].is_object());
    assert!(!result.text.contains("prose"));
    assert!(
        result
            .advisories
            .iter()
            .any(|s| s == "original relay evidence")
    );
    assert!(
        result
            .advisories
            .iter()
            .any(|s| s.contains("x402-list.com"))
    );
}

#[test]
fn grouped_workflows_finish_siblings_and_outlive_cancelled_waiters() {
    if std::env::var_os("TREAZURY_FANOUT_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","discovery::tests::remote_tests::grouped_workflows_finish_siblings_and_outlive_cancelled_waiters","--nocapture"])
        .env("TREAZURY_FANOUT_CHILD","1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn fixture() {
    use crate::{
        network::{Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{body::Body, extract::Query, response::IntoResponse, routing::get};
    use futures_util::StreamExt;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let released = Arc::new(tokio::sync::Semaphore::new(0));
    let arrived = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let capture_started = started.clone();
    let captured = seen.clone();
    let coinbase = axum::Router::new().route("/search",get(move |Query(q): Query<BTreeMap<String,String>>| {
        let (captured, started) = (captured.clone(), capture_started.clone());
        async move {
            captured.lock().unwrap().push(("coinbase".to_owned(),q.clone()));
            started.add_permits(1);
            if matches!(q.get("query").map(String::as_str),Some("failure" | "allfail")) { return "private non JSON response".into_response(); }
            if q.get("query").map(String::as_str)==Some("stall") {
                let chunks = futures_util::stream::iter(["{", "\"resources\":", "[]", ",\"unknown\":true", "}"]).then(|chunk| async move {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok::<_,std::io::Error>(chunk)
                });
                return ([("content-type","application/json")],Body::from_stream(chunks)).into_response();
            }
            axum::Json(json!({"resources":[{"url":"duplicate"},{"url":"duplicate"}],"unknown":q["curatedOnly"],"partialResults":true})).into_response()
        }
    }));
    let (coin_address, coin_task) = crate::test_tls::serve(coinbase).await;
    let (capture, arrive, release) = (seen.clone(), arrived.clone(), released.clone());
    let list = axum::Router::new().route(
        "/best",
        get(move |Query(q): Query<BTreeMap<String, String>>| {
            let (capture, arrive, release) = (capture.clone(), arrive.clone(), release.clone());
            async move {
                capture
                    .lock()
                    .unwrap()
                    .push(("x402-list".to_owned(), q.clone()));
                arrive.add_permits(1);
                if q.get("q").map(String::as_str) == Some("cancel") {
                    release.acquire().await.unwrap().forget();
                }
                let query = q.get("q").cloned().unwrap_or_default();
                let chunks = futures_util::stream::iter([
                    "{",
                    "\"data\":",
                    "[]",
                    ",\"unknown\":true,\"partial\":true",
                    "}",
                ])
                .then(move |chunk| {
                    let query = query.clone();
                    async move {
                        if query == "failure" {
                            tokio::time::sleep(Duration::from_millis(400)).await;
                        }
                        if query == "stall" {
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                        }
                        Ok::<_, std::io::Error>(if query == "allfail" { "invalid" } else { chunk })
                    }
                });
                (
                    [("content-type", "application/json")],
                    Body::from_stream(chunks),
                )
            }
        }),
    );
    let (list_address, list_task) = crate::test_tls::serve(list).await;
    let socks = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), coin_address),
            ("spec.example.com".into(), list_address),
        ]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            tor_isolation_retries: Some(0),
            read_timeout_seconds: Some(1),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut config = listeners(false);
    config
        .get_mut("writer")
        .unwrap()
        .remote_provider_directories = vec![
        RemoteDirectory::Coinbase,
        RemoteDirectory::CoinbaseCurated,
        RemoteDirectory::X402List,
        RemoteDirectory::Coinbase,
    ];
    let m = manager_with(config).await;
    *m.fixture_coinbase.lock().unwrap() = Some("https://api.example.com/search".into());
    *m.fixture_directory.lock().unwrap() = Some("https://spec.example.com".into());
    for q in ["success", "failure", "stall", "allfail"] {
        let result = m
            .invoke_search_remote(
                "writer",
                json!({"q":q,"limit":2,"max_price_usd":0.1})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&result.text).unwrap();
        if matches!(q, "stall" | "allfail") {
            assert!(value["x402-list"]["local_error"].is_object());
        } else {
            assert_eq!(
                value["x402-list"],
                json!({"data":[],"unknown":true,"partial":true})
            );
        }
        for key in ["coinbase", "coinbase_curated"] {
            if matches!(q, "failure" | "allfail") {
                assert!(value[key]["local_error"].is_object());
            } else if q == "stall" {
                assert_eq!(value[key], json!({"resources":[],"unknown":true}));
            } else {
                assert_eq!(value[key]["resources"].as_array().unwrap().len(), 2);
                assert_eq!(
                    value[key]["unknown"],
                    if key == "coinbase" { "false" } else { "true" }
                );
            }
        }
    }
    let before = seen.lock().unwrap().len();
    let pending = {
        let m = m.clone();
        tokio::spawn(async move {
            m.invoke_search_remote("writer", json!({"q":"cancel"}).as_object().unwrap())
                .await
        })
    };
    // Drain permits from completed list workflows, then wait for the new one.
    arrived.acquire_many(5).await.unwrap().forget();
    started.acquire_many(10).await.unwrap().forget();
    pending.abort();
    assert_eq!(
        seen.lock().unwrap().len(),
        before + 3,
        "every configured workflow starts once"
    );
    released.add_permits(1);
    m.source_client("writer").unwrap().drain().await;
    let calls = seen.lock().unwrap();
    assert_eq!(
        calls.len(),
        15,
        "no replay or duplicate configured directory"
    );
    for (_, query) in calls.iter().filter(|(key, _)| key == "x402-list") {
        assert_eq!(query["prefer"], "balanced");
        assert_eq!(query["require_verified"], "false");
        assert_eq!(query["network"], "BSE");
    }
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    coin_task.abort();
    list_task.abort();
}
