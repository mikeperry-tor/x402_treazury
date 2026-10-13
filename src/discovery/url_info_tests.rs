use super::*;
use crate::discovery::url_info;
use rmcp::ServerHandler;

#[tokio::test]
async fn input_and_listener_authority_precede_io() {
    let mut policy = policy(None);
    policy.allowed_origins = vec!["https://api.example.com".into()];
    let m = manager_with(policy, listeners(false)).await;
    for demand in [false, true] {
        let mut s = server(&m, "hidden");
        s.discover_on_demand = demand;
        assert!(s.get_tool(url_info::NAME).is_none());
        assert!(
            s.invoke_output(
                url_info::NAME,
                json!({"base_url":"https://api.example.com"})
                    .as_object()
                    .unwrap()
            )
            .await
            .is_err()
        );
        assert!(server(&m, "writer").get_tool(url_info::NAME).is_some());
    }
    for args in [
        json!({}),
        json!({"slug":null}),
        json!({"slug":""}),
        json!({"slug":".."}),
        json!({"slug":"vendor","base_url":"https://api.example.com"}),
        json!({"base_url":"https://api.example.com","wallet":"other"}),
        json!({"base_url":"https://api.example.com?secret=1"}),
        json!({"base_url":"http://api.example.com"}),
        json!({"base_url":"https://127.0.0.1"}),
        json!({"base_url":"https://user:secret@api.example.com"}),
        json!({"base_url":"https://other.example.com"}),
    ] {
        assert!(
            m.invoke_url_info("writer", args.as_object().unwrap())
                .await
                .is_err(),
            "{args}"
        );
    }
}

#[test]
fn unsigned_probes_validate_content_and_use_wallet_tor_isolation() {
    if std::env::var_os("TREAZURY_URL_INFO_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "discovery::tests::url_info_tests::unsigned_probes_validate_content_and_use_wallet_tor_isolation", "--nocapture"])
        .env("TREAZURY_URL_INFO_CHILD", "1").output().unwrap();
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
            if method == Method::HEAD {
                return match path {
                    "/v1/openapi.json" => StatusCode::METHOD_NOT_ALLOWED,
                    "/llms.txt" => StatusCode::NOT_IMPLEMENTED,
                    "/openapi.json" | "/v1/llms-full.txt" | "/.well-known/x402" | "/v1/swagger.json" => StatusCode::OK,
                    _ => StatusCode::NOT_FOUND,
                }.into_response();
            }
            match path {
                "/v1/openapi.json" => axum::Json(json!({"openapi":"3.1.0","info":{"title":"Fixture","version":"1"},"servers":[{"url":"https://api.example.com/v1"}],"paths":{"/read":{"get":{}}}})).into_response(),
                "/openapi.json" => ([("content-type", "text/html")], "<!doctype html><html>fallback</html>").into_response(),
                "/llms.txt" => ([("content-type", "text/plain")], "# Fixture API\nSee the provider docs.").into_response(),
                "/v1/llms-full.txt" => ([("content-type", "text/plain")], "x".repeat(1024)).into_response(),
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
    let mut p = policy(None);
    p.allowed_origins = vec!["https://api.example.com".into()];
    p.max_help_bytes = 64;
    let m = Manager::new(p, ls, payers, Arc::new(CatalogState::default()), vec![])
        .await
        .unwrap();
    *m.fixture_directory.lock().unwrap() = Some("https://api.example.com/directory".into());
    for (owner, args) in [
        ("writer", json!({"base_url":"https://api.example.com/v1"})),
        ("reader", json!({"slug":"vendor"})),
    ] {
        let output = server(&m, owner)
            .invoke(url_info::NAME, args.as_object().unwrap())
            .await
            .unwrap();
        let doc: Value = serde_json::from_str(&output).unwrap();
        let results = doc["results"].as_array().unwrap();
        assert_eq!(results.len(), 16);
        let result = |path: &str| {
            results
                .iter()
                .find(|r| r["url"] == format!("https://api.example.com{path}"))
                .unwrap()
        };
        assert_eq!(result("/v1/openapi.json")["status"], "verified_openapi");
        assert_eq!(result("/v1/openapi.json")["head_status"], 405);
        assert_eq!(result("/openapi.json")["status"], "invalid_document");
        assert_eq!(result("/llms.txt")["status"], "documentation_found");
        assert_eq!(result("/llms.txt")["head_status"], 501);
        assert_eq!(result("/llms-full.txt")["status"], "payment_required");
        assert_eq!(result("/v1/swagger.json")["status"], "payment_required");
        assert_eq!(result("/v1/llms-full.txt")["status"], "too_large");
        assert!(
            result("/v1/llms-full.txt")["reason"]
                .as_str()
                .unwrap()
                .contains("max_help_bytes=64")
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
    assert!(!requests.iter().any(|(m,p)| m == "GET" && (p == "/llms-full.txt" || p == "/.well-known/openapi.json")));
    assert_eq!(
        requests
            .iter()
            .filter(|(m, p)| m == "GET" && p == "/v1/swagger.json")
            .count(),
        2
    );
    assert_eq!(requests.iter().filter(|(m, _)| m == "HEAD").count(), 32);
    let records = proxy.records.lock().unwrap();
    let expected: Vec<_> = (1..=2)
        .map(|key| {
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            crate::network::global()
                .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap())
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
