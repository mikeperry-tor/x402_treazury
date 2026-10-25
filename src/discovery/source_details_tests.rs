use super::*;
use crate::discovery::source_details;
use rmcp::ServerHandler;

#[tokio::test]
async fn input_and_listener_authority_precede_io() {
    let m = manager_with(listeners(false)).await;
    for demand in [false, true] {
        let mut s = server(&m, "hidden");
        s.discover_on_demand = demand;
        assert!(s.get_tool(source_details::NAME).is_none());
        assert!(
            s.invoke_output(
                source_details::NAME,
                json!({"base_url":"https://api.example.com"})
                    .as_object()
                    .unwrap()
            )
            .await
            .is_err()
        );
        assert!(
            server(&m, "writer")
                .get_tool(source_details::NAME)
                .is_some()
        );
    }
    for args in [
        json!({}),
        json!({"slug":"vendor"}),
        json!({"x402_list_id":null}),
        json!({"x402_list_id":""}),
        json!({"x402_list_id":".."}),
        json!({"x402_list_id":"vendor","base_url":"https://api.example.com"}),
        json!({"base_url":"https://api.example.com","wallet":"other"}),
        json!({"base_url":"https://api.example.com?secret=1"}),
        json!({"base_url":"http://api.example.com"}),
        json!({"base_url":"https://127.0.0.1"}),
        json!({"base_url":"https://user:secret@api.example.com"}),
    ] {
        assert!(
            m.invoke_source_details("writer", args.as_object().unwrap())
                .await
                .is_err(),
            "{args}"
        );
    }
}

#[tokio::test]
async fn directory_listing_survives_unavailable_url_discovery() {
    use axum::{extract::Path, response::IntoResponse, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let (base, task) = listen(axum::Router::new().route(
        "/services/{slug}",
        get(move |Path(slug): Path<String>| {
            seen.fetch_add(1, Ordering::SeqCst);
            async move {
                match slug.as_str() {
                    "missing" => axum::Json(json!({"data":{"service":{"name":"Retained", "endpoints":[{"path":"/read"}]}},"meta":{"original":true}})).into_response(),
                    "rejected" => axum::Json(json!({"service":{"base_url":"https://127.0.0.1/private"},"original":true})).into_response(),
                    "malformed" => "Original non-JSON directory response".into_response(),
                    _ => panic!("unexpected directory request"),
                }
            }
        }),
    )).await;
    // A free directory response can be received without a documentation wallet.
    // The optional probe identity must not hide the already received listing.
    let m = Manager::new(
        listeners(false),
        BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    *m.fixture_directory.lock().unwrap() = Some(base);
    for (slug, status, listing) in [
        (
            "missing",
            "missing_api_base_url",
            json!({"data":{"service":{"name":"Retained", "endpoints":[{"path":"/read"}]}},"meta":{"original":true}}),
        ),
        (
            "rejected",
            "api_base_url_rejected",
            json!({"service":{"base_url":"https://127.0.0.1/private"},"original":true}),
        ),
        (
            "malformed",
            "directory_response_not_json",
            json!("Original non-JSON directory response"),
        ),
    ] {
        let result = m
            .invoke_source_details("writer", json!({"x402_list_id":slug}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(result["listing"], listing);
        assert_eq!(result["url_discovery"]["status"], status);
        assert_eq!(result["url_discovery"]["results"], json!([]));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    task.abort();
}

#[test]
fn unsigned_probes_validate_content_and_use_wallet_tor_isolation() {
    if std::env::var_os("TREAZURY_SOURCE_DETAILS_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "discovery::tests::source_details_tests::unsigned_probes_validate_content_and_use_wallet_tor_isolation", "--nocapture"])
        .env("TREAZURY_SOURCE_DETAILS_CHILD", "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn fixture() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        http::{HeaderMap, Method, StatusCode, Uri},
        response::IntoResponse,
        routing::any,
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = axum::Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap| {
        let captured = captured.clone();
        async move {
            assert!(!headers.contains_key("payment-signature"));
            assert!(!headers.contains_key("x-payment"));
            assert!(!headers.contains_key("authorization"));
            captured.lock().unwrap().push((method.to_string(), uri.path().to_owned()));
            if uri.path() == "/directory/services/vendor" {
                return axum::Json(json!({"data":{"base_url":"https://api.example.com/v1","website_url":"https://127.0.0.1/private"}})).into_response();
            }
            let path = uri.path();
            if path == "/.well-known/openapi.json" {
                return (StatusCode::FOUND, [("location", "https://127.0.0.1/private")]).into_response();
            }
            if path == "/llms-full.txt" || (method == Method::GET && path == "/v1/swagger.json") {
                return (StatusCode::PAYMENT_REQUIRED, [("payment-required", "deliberately-not-a-payment-to-sign")]).into_response();
            }
            match path {
                "/v1/openapi.json" => axum::Json(json!({"openapi":"3.1.0","info":{"title":"Fixture","version":"1"},"servers":[{"url":"https://api.example.com/v1"}],"paths":{"/read":{"get":{}}}})).into_response(),
                "/openapi.json" => ([("content-type", "text/html")], "<!doctype html><html>fallback</html>").into_response(),
                "/llms.txt" => ([("content-type", "text/plain")], "# Fixture API\nSee the provider docs.").into_response(),
                "/v1/llms-full.txt" => ([("content-type", "text/plain")], "x".repeat(crate::limits::HELP_BYTES + 1)).into_response(),
                "/.well-known/x402" => axum::Json(json!({"resources":[]})).into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }));
    let (address, task) = crate::test_tls::serve(app).await;
    let proxy = Socks::start(
        BTreeMap::from([("api.example.com".into(), address)]),
        Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut ls = listeners(false);
    ls.get_mut("reader").unwrap().wallet = Some("split".into());
    let payers = [(1, "shared"), (2, "split")]
        .into_iter()
        .map(|(key, name)| {
            (
                name.into(),
                PaidClient::new(
                    Payer::new(
                        &format!("{key:064x}"),
                        SpendPolicy::dollars("0.001").unwrap(),
                    )
                    .unwrap(),
                ),
            )
        })
        .collect();
    let m = Manager::new(ls, payers, Arc::new(CatalogState::default()), vec![])
        .await
        .unwrap();
    *m.fixture_directory.lock().unwrap() = Some("https://api.example.com/directory".into());
    for (owner, args) in [
        ("writer", json!({"base_url":"https://api.example.com/v1"})),
        ("reader", json!({"x402_list_id":"vendor"})),
    ] {
        let output = server(&m, owner)
            .invoke(source_details::NAME, args.as_object().unwrap())
            .await
            .unwrap();
        let doc: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(doc["url_discovery"]["status"], "completed");
        if args.get("x402_list_id").is_some() {
            assert_eq!(
                doc["listing"]["data"]["base_url"],
                "https://api.example.com/v1"
            );
        } else {
            assert!(doc["listing"].is_null());
        }
        assert_eq!(
            doc["url_discovery"]["website_status"],
            if args.get("x402_list_id").is_some() {
                "rejected_by_url_or_origin_policy"
            } else {
                "not_provided"
            }
        );
        let results = doc["url_discovery"]["results"].as_array().unwrap();
        assert_eq!(results.len(), 16);
        let result = |path: &str| {
            results
                .iter()
                .find(|r| r["url"] == format!("https://api.example.com{path}"))
                .unwrap()
        };
        assert_eq!(result("/v1/openapi.json")["status"], "verified_openapi");
        assert_eq!(result("/v1/openapi.json")["get_status"], 200);
        assert_eq!(result("/openapi.json")["status"], "invalid_document");
        assert_eq!(result("/llms.txt")["status"], "documentation_found");
        assert_eq!(result("/llms.txt")["get_status"], 200);
        assert!(
            results
                .iter()
                .all(|result| result.get("head_status").is_none())
        );
        assert_eq!(result("/llms-full.txt")["status"], "payment_required");
        assert_eq!(result("/v1/swagger.json")["status"], "payment_required");
        assert_eq!(result("/v1/llms-full.txt")["status"], "too_large");
        assert!(
            result("/v1/llms-full.txt")["reason"]
                .as_str()
                .unwrap()
                .contains(&format!("max_help_bytes={}", crate::limits::HELP_BYTES))
        );
        assert_eq!(
            result("/.well-known/x402")["status"],
            "directory_json_unverified"
        );
        assert_eq!(result("/.well-known/openapi.json")["status"], "redirect");
    }
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|(_, p)| p == "/directory/services/vendor")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(
                |(m, p)| m == "GET" && (p == "/llms-full.txt" || p == "/.well-known/openapi.json")
            )
            .count(),
        4
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(m, p)| m == "GET" && p == "/v1/swagger.json")
            .count(),
        2
    );
    assert_eq!(requests.iter().filter(|(m, _)| m == "HEAD").count(), 0);
    assert_eq!(
        requests.len(),
        33,
        "one GET per candidate plus the directory listing"
    );
    let records = proxy.records.lock().unwrap();
    let expected: Vec<_> = (1..=2)
        .map(|key| {
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            let identity = IsolationId::evm(&signer.address().to_string()).unwrap();
            let selected = crate::network::global()
                .http_participating(
                    &identity,
                    "https://api.example.com",
                    Some(Duration::from_secs(60)),
                    true,
                    Default::default(),
                )
                .unwrap();
            crate::network::global().credentials(&selected.scope)
        })
        .collect();
    for record in records.iter() {
        assert_eq!(record.host, "api.example.com");
        assert_eq!(record.address_type, 3);
        assert!(expected.contains(&(record.user.clone(), record.password.clone())));
    }
    for credentials in expected {
        assert!(
            records
                .iter()
                .any(|r| (r.user.clone(), r.password.clone()) == credentials)
        );
    }
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    task.abort();
}

#[test]
fn details_schema_and_root_candidates_preserve_public_contract() {
    let schema = source_details::schema(true);
    assert!(schema["properties"].get("slug").is_none());
    assert!(schema["properties"].get("x402_list_id").is_some());
    assert_ne!(source_details::NAME, "x402_treazury_source_details");
}
