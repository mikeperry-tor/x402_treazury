//! Explicit unsigned usage text, independent of speculative document-location probes.
use super::*;

#[tokio::test]
async fn documentation_selectors_and_policy_reject_before_io() {
    let mut p = policy(None);
    p.allowed_origins = vec!["https://api.example.com".into()];
    let m = manager_with(p, listeners(false)).await;
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
        json!({"documentation_url":"https://skill.example.com/help"}),
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
    let unsigned = Manager::new(
        policy(None),
        listeners(false),
        BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    let error = unsigned
        .invoke_source_details(
            "writer",
            json!({"documentation_url":"https://api.example.com/help"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("documentation identity unavailable")
    );
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
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy},
        test_socks::{Fault, Socks},
    };
    use axum::{
        body::{Body, Bytes},
        http::{HeaderMap, Method, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::any,
    };
    use tokio_util::sync::CancellationToken;
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
                    "/oversized" => {
                        ([("content-type", "text/plain")], "x".repeat(129)).into_response()
                    }
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
                    "/404" => StatusCode::NOT_FOUND.into_response(),
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
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let mut p = policy(None);
    p.allowed_origins = vec!["https://api.example.com".into()];
    p.max_help_bytes = 128;
    p.read_timeout_seconds = Some(1);
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
    let m = Manager::new(p, ls, payers, Arc::new(CatalogState::default()), vec![])
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
        ("/404", "not_found", None),
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
            before + 1,
            "must GET once without follow/traversal/retry"
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
            assert_eq!(doc["limit_bytes"], 128);
        }
        if path.starts_with("/plain") || path == "/html-root" {
            eprintln!("D02 actual fixture output: {value}");
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
    let result = tokio::time::timeout(Duration::from_secs(2), stalled)
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
    let credentials: Vec<_> = (1..=2)
        .map(|key| {
            let signer: alloy_signer_local::PrivateKeySigner =
                format!("{key:064x}").parse().unwrap();
            crate::network::global()
                .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap())
        })
        .collect();
    let records = proxy.records.lock().unwrap();
    for record in records.iter() {
        assert_eq!(record.host, "api.example.com");
        assert_eq!(record.address_type, 3);
        assert!(credentials.contains(&(record.user.clone(), record.password.clone())));
    }
    for identity in credentials {
        assert!(
            records
                .iter()
                .any(|r| (r.user.clone(), r.password.clone()) == identity)
        );
    }
    task.abort();
}
