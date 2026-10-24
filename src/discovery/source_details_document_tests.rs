//! Complete explicit usage text, independent of speculative document-location probes.
use super::*;

#[tokio::test]
async fn documentation_selectors_and_policy_reject_before_io() {
    let m = manager_with(listeners(false)).await;
    for invalid in [
        json!({"documentation_url":null}),
        json!({"documentation_url":3}),
        json!({"documentation_url":"https://api.example.com/help","slug":"service"}),
        json!({"documentation_url":"https://api.example.com/help","base_url":"https://api.example.com"}),
        json!({"documentation_url":"https://api.example.com/help","wallet":"shared"}),
        json!({"documentation_url":"http://api.example.com/help"}),
        json!({"documentation_url":"https://127.0.0.1/help"}),
        json!({"documentation_url":"https://user:secret@api.example.com/help"}),
        json!({"documentation_url":"https://api.example.com/help#fragment"}),
    ] {
        assert!(
            m.invoke_source_details("writer", invalid.as_object().unwrap())
                .await
                .is_err(),
            "{invalid}"
        );
    }
    assert!(
        m.invoke_source_details(
            "hidden",
            json!({"documentation_url":"https://api.example.com/help"})
                .as_object()
                .unwrap()
        )
        .await
        .is_err()
    );
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[test]
fn explicit_documents_use_one_unsigned_get_with_complete_visible_outcomes() {
    if std::env::var_os("TREAZURY_EXPLICIT_DOCUMENT_CHILD").is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(document_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","discovery::tests::source_details_document_tests::explicit_documents_use_one_unsigned_get_with_complete_visible_outcomes","--nocapture"])
        .env("TREAZURY_EXPLICIT_DOCUMENT_CHILD","1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
}

async fn document_fixture() {
    use crate::{
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy, ScopeObservation},
        test_socks::{Fault, Socks},
    };
    use axum::{
        body::{Body, Bytes},
        http::{HeaderMap, Method, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::any,
    };
    use tokio_util::sync::CancellationToken;
    let observations = Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
    let observed = observations.clone();
    crate::network::install_http_scope_observer(move |scope| {
        observed.lock().unwrap().push(scope.clone());
        Ok(())
    })
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let held = CancellationToken::new();
    let release = CancellationToken::new();
    let (captured, entered, unblock) = (requests.clone(), held.clone(), release.clone());
    let app =
        axum::Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap| {
            let (captured, entered, unblock) = (captured.clone(), entered.clone(), unblock.clone());
            async move {
                assert_eq!(
                    method,
                    Method::GET,
                    "explicit documentation must never use HEAD"
                );
                for forbidden in ["payment-signature", "x-payment", "authorization"] {
                    assert!(!headers.contains_key(forbidden));
                }
                captured
                    .lock()
                    .unwrap()
                    .push(uri.path_and_query().unwrap().to_string());
                let mut response = match uri.path() {
                    "/plain" => (
                        [("content-type", "text/plain; charset=utf-8")],
                        "No heading required.\nExact usage text.",
                    )
                        .into_response(),
                    "/markdown" => (
                        [("content-type", "TeXt/MaRkDoWn; charset=utf-8")],
                        "\u{feff} \nUsage **text**.\n",
                    )
                        .into_response(),
                    "/x-markdown" => {
                        ([("content-type", "text/x-markdown")], "  plain words  ").into_response()
                    }
                    "/empty" => ([("content-type", "text/plain")], "").into_response(),
                    "/204" => StatusCode::NO_CONTENT.into_response(),
                    "/206" => (
                        StatusCode::PARTIAL_CONTENT,
                        [("content-type", "text/plain")],
                        "fragment",
                    )
                        .into_response(),
                    "/range" => (
                        [
                            ("content-type", "text/plain"),
                            ("content-range", "bytes 0-7/100"),
                        ],
                        "fragment",
                    )
                        .into_response(),
                    "/invalid-mime" => {
                        let mut response = Response::new(Body::from("readable text"));
                        response.headers_mut().insert(
                            "content-type",
                            axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
                        );
                        response
                    }
                    "/html-literal" => (
                        [("content-type", "text/plain")],
                        "<html_escape> is a literal example.",
                    )
                        .into_response(),
                    "/spaces" => ([("content-type", "text/plain")], "\u{feff}  \n").into_response(),
                    "/missing-mime" => {
                        let mut r = Response::new(Body::from("No content type."));
                        r.headers_mut().remove("content-type");
                        r
                    }
                    "/html" => {
                        ([("content-type", "text/html")], "plain-looking body").into_response()
                    }
                    "/html-root" => (
                        [("content-type", "text/plain")],
                        " \n\u{feff} \t<HtMl lang=en>hidden</html>",
                    )
                        .into_response(),
                    "/doctype" => (
                        [("content-type", "text/markdown")],
                        "\u{feff}\n<!DoCtYpE hTmL><html></html>",
                    )
                        .into_response(),
                    "/json" => ([("content-type", "application/json")], "{}").into_response(),
                    "/nul" => {
                        ([("content-type", "text/plain")], b"text\0hidden".to_vec()).into_response()
                    }
                    "/invalid-utf8" => {
                        ([("content-type", "text/plain")], vec![0xff]).into_response()
                    }
                    "/oversized" => (
                        [("content-type", "text/plain")],
                        "x".repeat(crate::limits::HELP_BYTES + 1),
                    )
                        .into_response(),
                    "/redirect" => (
                        StatusCode::FOUND,
                        [("location", "https://spec.example.com/never-follow")],
                    )
                        .into_response(),
                    "/402" => (
                        StatusCode::PAYMENT_REQUIRED,
                        [("payment-required", "not-a-payment-to-sign")],
                    )
                        .into_response(),
                    "/403" => StatusCode::FORBIDDEN.into_response(),
                    "/401" => StatusCode::UNAUTHORIZED.into_response(),
                    "/404" => StatusCode::NOT_FOUND.into_response(),
                    "/410" => StatusCode::GONE.into_response(),
                    "/500" => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    "/retained" => {
                        entered.cancel();
                        let stream = futures_util::stream::once(async move {
                            unblock.cancelled().await;
                            Ok::<_, std::io::Error>(Bytes::from_static(b"retained complete text"))
                        });
                        Response::new(Body::from_stream(stream))
                    }
                    "/slow" => {
                        let stream = futures_util::stream::unfold(0, |n| async move {
                            if n == 3 {
                                return None;
                            }
                            tokio::time::sleep(Duration::from_millis(550)).await;
                            Some((Ok::<_, std::io::Error>(Bytes::from_static(b"chunk")), n + 1))
                        });
                        Response::new(Body::from_stream(stream))
                    }
                    "/stall" => {
                        let stream = futures_util::stream::once(async {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            Ok::<_, std::io::Error>(Bytes::from_static(b"too late"))
                        });
                        Response::new(Body::from_stream(stream))
                    }
                    _ => panic!("unexpected documentation path"),
                };
                // The retained and streamed cases exercise absent MIME too.
                if uri.path() == "/missing-mime" {
                    response.headers_mut().remove("content-type");
                }
                response
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
            read_timeout_seconds: Some(1),
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
    for (path, status, text) in [
        (
            "/plain?exact=a%20b&x=1",
            "ok",
            Some("No heading required.\nExact usage text."),
        ),
        ("/markdown", "ok", Some("\u{feff} \nUsage **text**.\n")),
        ("/x-markdown", "ok", Some("  plain words  ")),
        ("/empty", "empty", Some("")),
        ("/204", "empty", Some("")),
        ("/206", "http_error", None),
        ("/range", "http_error", None),
        ("/invalid-mime", "non_text", None),
        (
            "/html-literal",
            "ok",
            Some("<html_escape> is a literal example."),
        ),
        ("/spaces", "ok", Some("\u{feff}  \n")),
        ("/missing-mime", "ok", Some("No content type.")),
        ("/html", "non_text", None),
        ("/html-root", "non_text", None),
        ("/doctype", "non_text", None),
        ("/json", "non_text", None),
        ("/nul", "non_text", None),
        ("/invalid-utf8", "invalid_utf8", None),
        ("/oversized", "too_large", None),
        ("/redirect", "redirect", None),
        ("/402", "payment_required", None),
        ("/403", "blocked", None),
        ("/401", "blocked", None),
        ("/404", "not_found", None),
        ("/410", "not_found", None),
        ("/500", "http_error", None),
    ] {
        let url = format!("https://api.example.com{path}");
        let before = requests.lock().unwrap().len();
        let value = m
            .invoke_source_details(
                "writer",
                json!({"documentation_url":url}).as_object().unwrap(),
            )
            .await
            .unwrap();
        let doc = &value["documentation"];
        assert_eq!(doc["url"], url);
        assert_eq!(doc["provenance"], "caller_supplied");
        assert_eq!(doc["status"], status, "{path}: {value}");
        assert_eq!(doc.get("text").and_then(Value::as_str), text);
        assert_eq!(
            requests.lock().unwrap().len(),
            before
                + if matches!(path, "/403" | "/500") {
                    3
                } else {
                    1
                },
            "one complete GET sequence, with configured retries only for eligible failures"
        );
        assert_eq!(requests.lock().unwrap()[before], path);
        if matches!(path, "/206" | "/range") {
            assert_eq!(doc["reason"], "documentation partial_response");
        }
        if path == "/missing-mime" {
            assert!(doc["content_type"].is_null());
        }
        if path == "/oversized" {
            assert_eq!(doc["limit_setting"], "max_help_bytes");
            assert_eq!(doc["limit_bytes"], crate::limits::HELP_BYTES);
        }
    }
    let result = m
        .invoke_source_details(
            "reader",
            json!({"documentation_url":"https://api.example.com/slow"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result["documentation"]["text"], "chunkchunkchunk");
    let cloned = m.clone();
    let stalled = tokio::spawn(async move {
        cloned
            .invoke_source_details(
                "writer",
                json!({"documentation_url":"https://api.example.com/stall"})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(220)).await;
        let healthy = m
            .invoke_source_details(
                "reader",
                json!({"documentation_url":"https://api.example.com/empty"})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(healthy["documentation"]["status"], "empty");
    }
    let result = tokio::time::timeout(Duration::from_secs(5), stalled)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["documentation"]["status"], "unreachable");
    assert_eq!(
        result["documentation"]["reason"],
        "documentation read_inactivity"
    );
    assert!(result["documentation"].get("text").is_none());
    let cloned = m.clone();
    let waiter = tokio::spawn(async move {
        cloned
            .invoke_source_details(
                "writer",
                json!({"documentation_url":"https://api.example.com/retained"})
                    .as_object()
                    .unwrap(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), held.cancelled())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let client = m.source_client("writer").unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), client.drain())
            .await
            .is_err(),
        "retained document body must remain owned"
    );
    release.cancel();
    tokio::time::timeout(Duration::from_secs(2), client.drain())
        .await
        .unwrap();
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == "/retained")
            .count(),
        1
    );
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    let unsigned = Manager::new(
        listeners(false),
        BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    let unsigned_result = unsigned
        .invoke_source_details(
            "writer",
            json!({"documentation_url":"https://api.example.com/plain"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        unsigned_result["documentation"]["status"], "ok",
        "origin text remains available without a payer identity"
    );
    assert!(
        unsigned_result["documentation"]
            .get("payment_may_have_occurred")
            .is_none()
    );
    assert!(unsigned.inner.lock().unwrap().state.records.is_empty());
    let mut credentials: Vec<_> = (1..=2)
        .map(|key| {
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            crate::network::global()
                .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap())
        })
        .collect();
    credentials.push(
        crate::network::global()
            .credentials(&IsolationId::discovery("https://api.example.com/plain").unwrap()),
    );
    let observations = observations.lock().unwrap();
    for observation in observations.iter() {
        assert!(credentials.contains(&observation.canonical));
    }
    let records = proxy.records.lock().unwrap();
    for record in records.iter() {
        assert_eq!(record.host, "api.example.com");
        assert_eq!(record.address_type, 3);
        assert!(
            observations
                .iter()
                .any(|scope| scope.scope == (record.user.clone(), record.password.clone()))
        );
    }
    for identity in credentials {
        assert!(observations.iter().any(|scope| scope.canonical == identity));
    }
    task.abort();
}

#[test]
fn paid_document_fallback_preserves_validation_negative_probes_and_retained_ownership() {
    if std::env::var_os("TREAZURY_PAID_DOCUMENT_CHILD").is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(paid_document_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "discovery::tests::source_details_document_tests::paid_document_fallback_preserves_validation_negative_probes_and_retained_ownership", "--nocapture"])
        .env("TREAZURY_PAID_DOCUMENT_CHILD", "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn paid_document_fixture() {
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
    use tokio_util::sync::CancellationToken;
    // Runs in its own process. Include retained work and relay tasks in capture.
    let logs = tempfile::NamedTempFile::new().unwrap();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_env_filter("warn,x402_treazury=debug")
            .with_writer(logs.reopen().unwrap())
            .finish(),
    )
    .unwrap();
    let origin_calls = Arc::new(Mutex::new(Vec::new()));
    let captured = origin_calls.clone();
    let origin =
        axum::Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap| {
            let captured = captured.clone();
            async move {
                assert_eq!(method, Method::GET);
                for forbidden in ["payment-signature", "x-payment", "authorization"] {
                    assert!(!headers.contains_key(forbidden));
                }
                captured.lock().unwrap().push(uri.path().to_owned());
                match uri.path() {
                    "/paid-help" | "/paid-invalid" | "/relayed-402" | "/retained"
                    | "/openapi.json" | "/llms.txt" => StatusCode::FORBIDDEN.into_response(),
                    "/.well-known/openapi.json" | "/gone" => StatusCode::GONE.into_response(),
                    "/.well-known/x402" => StatusCode::UNAUTHORIZED.into_response(),
                    "/swagger.json" => {
                        ([("content-type", "application/json")], "not JSON").into_response()
                    }
                    "/probe-large-length" => "12345678".into_response(),
                    "/probe-large-stream" => {
                        let chunks = futures_util::stream::iter([
                            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"1234")),
                            Ok(axum::body::Bytes::from_static(b"5678")),
                        ]);
                        axum::response::Response::new(axum::body::Body::from_stream(chunks))
                    }
                    "/partial" | "/discovery/resources" => {
                        (StatusCode::PARTIAL_CONTENT, "fragment").into_response()
                    }
                    "/range" => ([("content-range", "bytes 0-1/20")], "{}").into_response(),
                    "/402" => (
                        StatusCode::PAYMENT_REQUIRED,
                        [("payment-required", "must-not-sign-target")],
                    )
                        .into_response(),
                    "/llms-full.txt" => {
                        ([("content-type", "text/plain")], "# Origin help").into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }
        }));
    let (origin_address, origin_task) = crate::test_tls::serve(origin).await;
    let paid_calls = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let entered = CancellationToken::new();
    let release = CancellationToken::new();
    let (seen, held, unblock) = (paid_calls.clone(), entered.clone(), release.clone());
    let relay = axum::Router::new().route("/curl", post(move |headers: HeaderMap, axum::Json(request): axum::Json<Value>| {
        let (seen, held, unblock) = (seen.clone(), held.clone(), unblock.clone());
        async move {
            assert_eq!(request["method"], "GET");
            assert!(request.get("headers").is_none(), "no target credentials are forwarded");
            let url = request["url"].as_str().unwrap().to_owned();
            let signed = headers.contains_key("payment-signature");
            seen.lock().unwrap().push((url.clone(), signed));
            if !signed {
                let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/curl","mimeType":"application/json"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                return (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))], "").into_response();
            }
            let payload: Value = serde_json::from_slice(&STANDARD.decode(headers["payment-signature"].as_bytes()).unwrap()).unwrap();
            assert_eq!(payload["payload"]["authorization"]["value"], "10000");
            let path = reqwest::Url::parse(&url).unwrap().path().to_owned();
            if path == "/retained" { held.cancel(); unblock.cancelled().await; }
            let (mime, body) = match path.as_str() {
                "/paid-help" | "/retained" => ("text/plain", "Complete relayed help"),
                "/relayed-402" => ("text/plain", "unpaid target challenge"),
                "/paid-invalid" => ("text/plain", "<!doctype html><html>not usage text</html>"),
                "/openapi.json" => ("application/json", r#"{"openapi":"3.1.0","info":{"title":"Fixture","version":"1"},"paths":{}}"#),
                "/llms.txt" => ("text/plain", "# Relayed help"),
                _ => panic!("a terminal origin outcome must not buy documentation: {path}"),
            };
            axum::Json(json!({"url":url,"method":"GET","statusCode":if path == "/relayed-402" { 402 } else { 200 },"headers":{"content-type":mime},"body":body,"redirectChain":[],"error":null})).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_address = listener.local_addr().unwrap();
    let relay_task = tokio::spawn(async move {
        axum::serve(listener, relay).await.unwrap();
    });
    let socks = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), origin_address),
            ("127.0.0.1".into(), relay_address),
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
    let sequence = Arc::new(crate::discovery_relay::SearchRelay::fixture(
        format!("http://{relay_address}/curl"),
        Default::default(),
    ));
    let payer = PaidClient::new(
        Payer::new(
            &format!("{:064x}", 47),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    let manager = Manager::new_with_search(
        listeners(false),
        BTreeMap::from([("shared".into(), payer)]),
        Arc::new(CatalogState::default()),
        vec![],
        Some(sequence),
    )
    .await
    .unwrap();
    for (path, status, paid) in [
        ("/paid-help", "ok", true),
        ("/paid-invalid", "non_text", true),
        ("/relayed-402", "payment_required", true),
        ("/missing", "not_found", false),
        ("/gone", "not_found", false),
        ("/partial", "http_error", false),
        ("/range", "http_error", false),
        ("/402", "payment_required", false),
    ] {
        let before = paid_calls.lock().unwrap().len();
        let value = manager
            .invoke_source_details(
                "writer",
                json!({"documentation_url":format!("https://api.example.com{path}")})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(value["documentation"]["status"], status, "{path}: {value}");
        assert_eq!(
            paid_calls.lock().unwrap().len() - before,
            if paid { 2 } else { 0 }
        );
        assert_eq!(
            value["documentation"]["payment_may_have_occurred"]
                .as_bool()
                .unwrap_or(false),
            paid
        );
        if paid {
            assert!(
                value["documentation"]["warning"]
                    .as_str()
                    .unwrap()
                    .contains("may have charged")
            );
        }
        assert_eq!(
            origin_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.as_str() == path)
                .count(),
            if paid { 3 } else { 1 }
        );
    }
    let explicit_logs = std::fs::read_to_string(logs.path()).unwrap();
    assert!(explicit_logs.lines().any(|line| line.contains("WARN")
        && line.contains("request_kind=Document")
        && line.contains("http_status=404")
        && line.contains("document does not exist at this location")
        && line.contains("relay_fallback_eligible=false")));
    let before = paid_calls.lock().unwrap().len();
    let discovered = manager
        .invoke_source_details(
            "writer",
            json!({"base_url":"https://api.example.com"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    let results = discovered["url_discovery"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 8);
    assert!(
        results
            .iter()
            .all(|result| result.get("head_status").is_none())
    );
    let result = |path: &str| {
        results
            .iter()
            .find(|result| result["url"] == format!("https://api.example.com{path}"))
            .unwrap()
    };
    assert_eq!(result("/openapi.json")["status"], "verified_openapi");
    assert_eq!(result("/llms.txt")["status"], "documentation_found");
    assert_eq!(result("/api/openapi.json")["status"], "not_found");
    assert_eq!(result("/.well-known/openapi.json")["status"], "not_found");
    assert_eq!(result("/swagger.json")["status"], "invalid_document");
    assert_eq!(result("/discovery/resources")["status"], "invalid_document");
    assert_eq!(
        paid_calls.lock().unwrap().len() - before,
        4,
        "only the two blocked candidates purchase a document"
    );
    let all_logs = std::fs::read_to_string(logs.path()).unwrap();
    let probe_logs = &all_logs[explicit_logs.len()..];
    assert!(!probe_logs.contains("WARN"), "{probe_logs}");
    for status in [401, 403, 404, 410] {
        assert!(
            probe_logs.lines().any(|line| line.contains("DEBUG")
                && line.contains(&format!("http_status={status}"))
                && line.contains("Optional discovery probe")),
            "{probe_logs}"
        );
    }
    for kind in ["OpenApiProbe", "DirectoryProbe", "DocumentationProbe"] {
        assert!(
            probe_logs.contains(&format!("request_kind={kind}")),
            "{probe_logs}"
        );
    }
    let blocked = probe_logs
        .lines()
        .find(|line| line.contains("http_status=403") && line.contains("request_kind=OpenApiProbe"))
        .unwrap();
    for expected in [
        "service denied access",
        "route=Tor",
        "attempt=3",
        "max_attempts=3",
        "relay_fallback_eligible=true",
    ] {
        assert!(blocked.contains(expected), "{blocked}");
    }
    let document_id = blocked
        .split("document_id=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    assert!(
        probe_logs
            .lines()
            .any(|line| line.contains(document_id) && line.contains("Using configured paid relay")),
        "{probe_logs}"
    );
    for private in [
        "api.example.com",
        "/openapi.json",
        "/paid-help",
        "must-not-sign-target",
        "Complete relayed help",
    ] {
        assert!(!all_logs.contains(private), "leaked {private}");
    }
    // Both declared-length and streamed overflow use the same bounds, without
    // WARN noise or purchasing a relay response for a local size rejection.
    let before = paid_calls.lock().unwrap().len();
    let client = manager.document_client("writer").unwrap();
    for path in ["/probe-large-length", "/probe-large-stream"] {
        let url = format!("https://api.example.com{path}");
        let mut options = crate::discovery_relay::document::Options::new(
            &url,
            crate::network::IsolationId::discovery(&url).unwrap(),
            7,
            "max_help_bytes",
        );
        options.kind = crate::discovery_relay::document::RequestKind::DocumentationProbe;
        let error = crate::discovery_relay::document::read(&client, &options)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::limits::DownloadLimit>()
                .unwrap()
                .limit,
            7
        );
        assert!(
            error
                .to_string()
                .contains("content rejected, no partial content returned")
        );
    }
    assert_eq!(paid_calls.lock().unwrap().len(), before);
    let all_logs = std::fs::read_to_string(logs.path()).unwrap();
    let oversized_logs = &all_logs[explicit_logs.len()..];
    assert!(!oversized_logs.contains("WARN"), "{oversized_logs}");
    assert_eq!(
        oversized_logs.matches("limit_bytes=7").count(),
        2,
        "{oversized_logs}"
    );
    assert!(
        oversized_logs.contains("setting=\"max_help_bytes\""),
        "{oversized_logs}"
    );
    let owner = manager.clone();
    let waiter = tokio::spawn(async move {
        owner
            .invoke_source_details(
                "writer",
                json!({"documentation_url":"https://api.example.com/retained"})
                    .as_object()
                    .unwrap(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered.cancelled())
        .await
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let client = manager.source_client("writer").unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), client.drain())
            .await
            .is_err()
    );
    release.cancel();
    tokio::time::timeout(Duration::from_secs(2), client.drain())
        .await
        .unwrap();
    assert_eq!(
        paid_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(url, signed)| url.ends_with("/retained") && *signed)
            .count(),
        1
    );
    assert!(manager.inner.lock().unwrap().state.records.is_empty());
    assert!(
        socks
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|record| record.host == "api.example.com" || record.host == "127.0.0.1")
    );
    origin_task.abort();
    relay_task.abort();
}
