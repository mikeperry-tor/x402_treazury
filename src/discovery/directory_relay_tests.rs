//! Directory JSON must stay parseable when its transport purchases relay delivery.
use super::*;

#[test]
fn directory_relay_preserves_json_fee_visibility_and_complete_origin_results() {
    if std::env::var_os("TREAZURY_DIRECTORY_RELAY_CHILD").is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(directory_relay_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery::tests::directory_relay_tests::directory_relay_preserves_json_fee_visibility_and_complete_origin_results",
            "--nocapture",
        ])
        .env("TREAZURY_DIRECTORY_RELAY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn directory_relay_fixture() {
    use crate::{
        network::{Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        http::{HeaderMap, Method, StatusCode, Uri},
        response::IntoResponse,
        routing::{any, post},
    };
    use base64::{Engine, engine::general_purpose::STANDARD};

    let origin_calls = Arc::new(Mutex::new(Vec::new()));
    let captured = origin_calls.clone();
    let origin =
        axum::Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap| {
            let captured = captured.clone();
            async move {
                assert_eq!(method, Method::GET);
                assert!(
                    !headers.contains_key("payment-signature"),
                    "directory origin must not receive a relay signature"
                );
                captured.lock().unwrap().push(uri.path().to_owned());
                match uri.path() {
                    "/api/v1/services"
                    | "/api/v1/services/relayed"
                    | "/api/v1/services/invalid"
                    | "/api/v1/services/mixed"
                    | "/openapi.json"
                    | "/llms.txt" => StatusCode::FORBIDDEN.into_response(),
                    "/registered-openapi.json" => axum::Json(json!({
                        "openapi":"3.0.3", "info":{"title":"Ordinary directory API","version":"1"},
                        "servers":[{"url":"https://api.example.com"}],
                        "paths":{"/api/v1/services":{"get":{"operationId":"registeredBrowse","responses":{"200":{"description":"Directory JSON"}}}}}
                    })).into_response(),
                    "/api/v1/services/positive" => {
                        axum::Json(json!({"service":{"name":"Origin listing"}})).into_response()
                    }
                    "/api/v1/services/empty" => axum::Json(json!({})).into_response(),
                    // Incomplete metadata is still a complete successful directory response.
                    "/api/v1/services/metadata" => {
                        axum::Json(json!({"service":{"description":"No API base URL supplied"}}))
                            .into_response()
                    }
                    "/api/openapi.json"
                    | "/.well-known/openapi.json"
                    | "/swagger.json"
                    | "/.well-known/x402"
                    | "/discovery/resources"
                    | "/llms-full.txt" => StatusCode::NOT_FOUND.into_response(),
                    _ => panic!("unexpected directory or speculative probe request: {uri}"),
                }
            }
        }));
    let (origin_address, origin_task) = crate::test_tls::serve(origin).await;
    let paid_calls = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let seen = paid_calls.clone();
    let relay = axum::Router::new().route("/curl", post(move |headers: HeaderMap, axum::Json(request): axum::Json<Value>| {
        let seen = seen.clone();
        async move {
            assert_eq!(request["method"], "GET");
            assert!(request.get("headers").is_none(), "no target credentials are forwarded");
            let url = request["url"].as_str().unwrap().to_owned();
            let signed = headers.contains_key("payment-signature");
            seen.lock().unwrap().push((url.clone(), signed));
            if !signed {
                let challenge = json!({"x402Version":2,"resource":{"url":"https://spec.example.com/curl","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                return (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))], "").into_response();
            }
            let payload: Value = serde_json::from_slice(&STANDARD.decode(headers["payment-signature"].as_bytes()).unwrap()).unwrap();
            assert_eq!(payload["payload"]["authorization"]["value"], "10000");
            let expected: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
            assert_eq!(crate::test_signatures::recover_exact(&payload), expected.address());
            let body = if url.ends_with("/invalid") {
                "complete but invalid directory JSON".to_owned()
            } else if url.ends_with("/mixed") {
                json!({"service":{"name":"Mixed listing","base_url":"https://api.example.com"}}).to_string()
            } else if url.ends_with("/openapi.json") {
                json!({"openapi":"3.0.0","info":{"title":"Category fixture","version":"1"},"paths":{}}).to_string()
            } else if url.ends_with("/llms.txt") {
                "# Provider usage\nExact usage instructions.".to_owned()
            } else if reqwest::Url::parse(&url).unwrap().path() == "/api/v1/services" {
                json!({"services":[{"name":"Relayed search result"}]}).to_string()
            } else {
                json!({"service":{"name":"Relayed listing"}}).to_string()
            };
            axum::Json(json!({"url":url,"method":"GET","statusCode":200,"headers":{"content-type":if url.ends_with("/llms.txt") {"text/plain"} else {"application/json"}},"body":body,"redirectChain":[],"error":null})).into_response()
        }
    }));
    let (relay_address, relay_task) = crate::test_tls::serve(relay).await;
    let socks = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), origin_address),
            ("spec.example.com".into(), relay_address),
        ]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let relay = Arc::new(crate::discovery_relay::SearchRelay::fixture(
        "https://spec.example.com/curl".into(),
        Default::default(),
    ));
    let manager = Manager::new_with_search(
        listeners(false),
        BTreeMap::from([("shared".into(), payer())]),
        Arc::new(CatalogState::default()),
        vec![],
        Some(relay.clone()),
    )
    .await
    .unwrap();
    *manager.fixture_directory.lock().unwrap() = Some("https://api.example.com/api/v1".into());

    let search = manager
        .invoke_directory("writer", directory::SEARCH, json!({}).as_object().unwrap())
        .await
        .unwrap();
    assert!(search.payment_may_have_occurred);
    assert!(
        !search.advisories.is_empty(),
        "relay fee must be visible separately from JSON"
    );
    let body: Value = serde_json::from_str(&search.text).unwrap();
    assert_eq!(body["services"][0]["name"], "Relayed search result");
    assert_eq!(paid_calls.lock().unwrap().len(), 2);

    for (slug, status) in [
        ("relayed", "missing_api_base_url"),
        ("invalid", "directory_response_not_json"),
    ] {
        let before = paid_calls.lock().unwrap().len();
        let value = manager
            .invoke_source_details("writer", json!({"x402_list_id":slug}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(value["url_discovery"]["status"], status, "{value}");
        assert!(
            !value["warnings"].as_array().unwrap().is_empty(),
            "paid directory lookup warning must survive JSON parsing: {value}"
        );
        if slug == "relayed" {
            assert_eq!(value["listing"]["service"]["name"], "Relayed listing");
        } else {
            assert_eq!(value["listing"], "complete but invalid directory JSON");
        }
        assert_eq!(
            paid_calls.lock().unwrap().len() - before,
            2,
            "complete invalid JSON must not purchase again"
        );
    }
    let before = paid_calls.lock().unwrap().len();
    for slug in ["positive", "empty", "metadata"] {
        let value = manager
            .invoke_source_details("writer", json!({"x402_list_id":slug}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(value["url_discovery"]["status"], "missing_api_base_url");
        assert!(value.get("warnings").is_none());
    }
    assert_eq!(
        paid_calls.lock().unwrap().len(),
        before,
        "complete ordinary directory results never authorize fallback"
    );
    for (url, signed) in paid_calls
        .lock()
        .unwrap()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|calls| (&calls[0], &calls[1]))
    {
        assert_eq!(url.0, signed.0);
        assert!(!url.1);
        assert!(signed.1);
    }
    for (path, count) in [
        ("/api/v1/services", 3),
        ("/api/v1/services/relayed", 3),
        ("/api/v1/services/invalid", 3),
        ("/api/v1/services/positive", 1),
        ("/api/v1/services/empty", 1),
        ("/api/v1/services/metadata", 1),
    ] {
        assert_eq!(
            origin_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.as_str() == path)
                .count(),
            count,
            "{path}"
        );
    }
    assert!(
        manager.inner.lock().unwrap().state.records.is_empty(),
        "directory discovery never registers or funds a source"
    );
    // The same public directory URL remains an ordinary API destination for raw calls.
    let before = paid_calls.lock().unwrap().len();
    assert!(
        manager
            .invoke_raw(
                "writer",
                json!({"method":"GET","url":"https://api.example.com/api/v1/services"})
                    .as_object()
                    .unwrap()
            )
            .await
            .is_err()
    );
    assert_eq!(
        paid_calls.lock().unwrap().len(),
        before,
        "default API fallback is off even at the built-in directory hostname"
    );

    for discovery in [false, true] {
        for metadata in [false, true] {
            let configured = Arc::new((*relay).clone().with_policy(
                crate::discovery_relay::Policy {
                    source_discovery: discovery,
                    provider_metadata: metadata,
                    api_requests: !discovery,
                    ..Default::default()
                },
            ));
            let listeners = listeners(false);
            let snapshot = CatalogSnapshot {
                views: listeners
                    .keys()
                    .map(|owner| (owner.clone(), vec![]))
                    .collect(),
                ..Default::default()
            };
            let m = Manager::new_with_search(
                listeners,
                BTreeMap::from([("shared".into(), payer())]),
                Arc::new(CatalogState::new(snapshot)),
                vec![],
                Some(configured),
            )
            .await
            .unwrap();
            *m.fixture_directory.lock().unwrap() = Some("https://api.example.com/api/v1".into());
            let before = paid_calls.lock().unwrap().len();
            let details = m
                .invoke_source_details(
                    "writer",
                    json!({"x402_list_id":"mixed"}).as_object().unwrap(),
                )
                .await;
            if discovery {
                let details = details.unwrap();
                assert_eq!(details["listing"]["service"]["name"], "Mixed listing");
                let results = details["url_discovery"]["results"].as_array().unwrap();
                let schema = results
                    .iter()
                    .find(|item| item["url"] == "https://api.example.com/openapi.json")
                    .unwrap();
                assert_eq!(
                    schema["status"],
                    if metadata {
                        "verified_openapi"
                    } else {
                        "blocked"
                    }
                );
                assert_eq!(
                    paid_calls.lock().unwrap().len() - before,
                    if metadata { 6 } else { 2 }
                );
            } else {
                assert!(
                    details.is_err(),
                    "blocked directory cannot invent a base URL"
                );
                assert_eq!(paid_calls.lock().unwrap().len(), before);
            }
            // Explicit documentation follows metadata policy independently of directory/API policy.
            let before = paid_calls.lock().unwrap().len();
            let document = m
                .invoke_source_details(
                    "writer",
                    json!({"documentation_url":"https://api.example.com/llms.txt"})
                        .as_object()
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                document["documentation"]["status"],
                if metadata { "ok" } else { "blocked" }
            );
            assert_eq!(
                paid_calls.lock().unwrap().len() - before,
                if metadata { 2 } else { 0 }
            );
            let before = paid_calls.lock().unwrap().len();
            let raw = m
                .invoke_raw(
                    "writer",
                    json!({"method":"GET","url":"https://api.example.com/api/v1/services"})
                        .as_object()
                        .unwrap(),
                )
                .await;
            assert_eq!(
                raw.is_ok(),
                !discovery,
                "raw directory hostname follows API category"
            );
            assert_eq!(
                paid_calls.lock().unwrap().len() - before,
                if discovery { 0 } else { 2 }
            );
            // Healthy origin results are usable even when their relay category is disabled.
            let healthy = m
                .invoke_source_details(
                    "writer",
                    json!({"x402_list_id":"positive"}).as_object().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(healthy["listing"]["service"]["name"], "Origin listing");
            assert!(m.inner.lock().unwrap().state.records.is_empty());
            // Registering a schema at the built-in directory path grants only ordinary API purpose.
            let before = paid_calls.lock().unwrap().len();
            m.invoke("writer", "x402_treazury_source_add", json!({
                "name":"ordinary_directory", "spec_url":"https://api.example.com/registered-openapi.json"
            })).await.unwrap();
            assert_eq!(
                paid_calls.lock().unwrap().len(),
                before,
                "healthy origin schema registers with metadata relay disabled too"
            );
            let tool_id = {
                let catalog = m.catalog.read();
                let published = &catalog.views["writer"];
                assert_eq!(published.len(), 1);
                assert_eq!(published[0].tool.path, "/api/v1/services");
                published[0].tool.name.clone()
            };
            let before = paid_calls.lock().unwrap().len();
            let registered = m
                .find_tool("writer", &tool_id)
                .unwrap()
                .invoke_output(&Default::default())
                .await;
            assert_eq!(
                registered.is_ok(),
                !discovery,
                "registered directory path follows API category"
            );
            let calls = paid_calls.lock().unwrap();
            assert_eq!(calls.len() - before, if discovery { 0 } else { 2 });
            assert!(
                calls[before..]
                    .iter()
                    .all(|(url, _)| url == "https://api.example.com/api/v1/services")
            );
            drop(calls);
            assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
        }
    }
    origin_task.abort();
    relay_task.abort();
}
