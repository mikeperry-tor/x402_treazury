use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn streamed_import_limits_partial_chunks_and_slow_download() {
    for mode in ["exact", "overflow", "interrupted", "slow"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/spec", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() <= 8192);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n").await.unwrap();
            match mode {
                "slow" => {
                    // Exceed the old total deadline with continuous progress.
                    for byte in b"efgh" {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        socket
                            .write_all(&[b'1', b'\r', b'\n', *byte, b'\r', b'\n'])
                            .await
                            .unwrap();
                    }
                    socket.write_all(b"0\r\n\r\n").await.unwrap();
                }
                "interrupted" => {
                    socket.write_all(b"4\r\ne").await.unwrap();
                }
                _ => {
                    socket.write_all(b"4\r\nefgh\r\n").await.unwrap();
                    if mode == "overflow" {
                        socket.write_all(b"1\r\ni\r\n").await.unwrap();
                    }
                    socket.write_all(b"0\r\n\r\n").await.unwrap();
                }
            }
        });
        let mut p = policy(None);
        p.max_spec_bytes = 8;
        p.read_timeout_seconds = Some(1);
        let result = tokio::time::timeout(Duration::from_secs(5), import::fixture_fetch(&p, &url))
            .await
            .expect("fixture completes");
        if mode == "exact" || mode == "slow" {
            assert_eq!(result.unwrap().body.as_slice(), b"abcdefgh");
        } else {
            let error = result.unwrap_err();
            match mode {
                "overflow" => {
                    let limit = error
                        .downcast_ref::<crate::limits::DownloadLimit>()
                        .expect("typed download rejection");
                    assert_eq!((limit.setting, limit.limit), ("max_spec_bytes", 8));
                }
                "interrupted" => assert!(
                    error.downcast_ref::<reqwest::Error>().is_some(),
                    "{error:#}"
                ),
                _ => unreachable!(),
            }
        }
        server.abort();
    }
}

#[test]
fn allowed_origins_apply_to_spec_base_and_explicit_override() {
    let mut p = policy(None);
    p.allowed_origins = vec![
        "https://api.example.com".into(),
        "https://spec.example.com:8443".into(),
    ];
    for allowed in [
        "https://api.example.com:443/spec",
        "https://spec.example.com:8443/spec",
    ] {
        assert!(import::endpoint(&p, allowed).is_ok());
    }
    for denied in [
        "https://api.example.com:8443/spec",
        "https://spec.example.com/spec",
        "https://api.example.com.attacker.com/spec",
        "https://other.example.com/spec",
    ] {
        assert!(import::endpoint(&p, denied).is_err());
    }
    let mut c: Candidate =
        serde_json::from_value(candidate("origins", "process", "server")).unwrap();
    c.spec_url = "https://spec.example.com:8443/spec".into();
    let mut doc = spec();
    assert!(import::build(&p, &c, &serde_json::to_vec(&doc).unwrap()).is_ok());
    c.base_url = Some("https://other.example.com".into());
    assert!(import::build(&p, &c, &serde_json::to_vec(&doc).unwrap()).is_err());
    c.base_url = None;
    doc["servers"] = json!([{"url":"https://other.example.com"}]);
    assert!(import::build(&p, &c, &serde_json::to_vec(&doc).unwrap()).is_err());
    c.spec_url = "https://other.example.com/spec".into();
    doc = spec();
    assert!(import::build(&p, &c, &serde_json::to_vec(&doc).unwrap()).is_err());
}

#[derive(Clone, Default)]
struct LimitLogs(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LimitLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_limit_logs(run: impl FnOnce()) -> String {
    let logs = LimitLogs::default();
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, run);
    String::from_utf8(logs.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn resource_limit_warns_only_when_exceeded() {
    let logs = capture_limit_logs(|| {
        crate::limits::check(511, 512, "fixture_limit", "fixture_setting").unwrap();
        crate::limits::check(512, 512, "fixture_limit", "fixture_setting").unwrap();
    });
    assert!(logs.is_empty(), "{logs}");
    let logs = capture_limit_logs(|| {
        let error = crate::limits::check(513, 512, "fixture_limit", "fixture_setting").unwrap_err();
        assert!(error.to_string().starts_with("fixture_limit"));
        assert!(error.to_string().contains("512"));
        assert!(error.to_string().contains("fixture_setting"));
    });
    for expected in ["WARN", "fixture_limit", "fixture_setting", "512", "513"] {
        assert!(logs.contains(expected), "missing {expected}: {logs}");
    }
}

#[test]
fn generated_tool_names_accept_512_bytes_and_warn_above_boundary() {
    let p = policy(None);
    let c: Candidate = serde_json::from_value(candidate("name", "process", "server")).unwrap();
    // The source name is separated from the operation by two underscores.
    let prefix_len = "dyn_name__".len();
    for length in [512, 513] {
        let path = format!("/{}", "x".repeat(length - prefix_len));
        let mut doc = spec();
        doc["paths"] = json!({(path):{"get":{}}});
        let logs = capture_limit_logs(|| {
            let result = import::build(&p, &c, &serde_json::to_vec(&doc).unwrap());
            if length == 512 {
                assert_eq!(result.unwrap().tools[0].name.len(), 512);
            } else {
                assert!(
                    result
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("tool_name_too_long")
                );
            }
        });
        if length == 512 {
            assert!(logs.is_empty(), "{logs}");
        } else {
            for expected in ["WARN", "tool_name_too_long", "512", "513"] {
                assert!(logs.contains(expected), "missing {expected}: {logs}");
            }
            assert!(
                !logs.contains(&"x".repeat(64)),
                "private name in logs: {logs}"
            );
            assert!(
                !logs.contains("api.example.com"),
                "private URL in logs: {logs}"
            );
        }
    }
}

#[test]
fn generated_schema_limit_warns_without_logging_document_content() {
    let p = policy(None);
    let c: Candidate = serde_json::from_value(candidate("schema", "process", "server")).unwrap();
    let mut doc = spec();
    doc["paths"]["/read"]["get"]["parameters"][0]["schema"]["description"] =
        json!("PRIVATE_DOCUMENT_CONTENT".repeat(12_000));
    let logs = capture_limit_logs(|| {
        let result = import::build(&p, &c, &serde_json::to_vec(&doc).unwrap());
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("schema_size_limit")
        );
    });
    assert!(logs.contains("WARN"), "{logs}");
    assert!(logs.contains("schema_size_limit"), "{logs}");
    assert!(logs.contains("262144"), "{logs}");
    assert!(!logs.contains("PRIVATE_DOCUMENT_CONTENT"), "{logs}");
}

#[test]
fn operation_count_limit_warns_even_when_selection_excludes_operations() {
    let p = policy(None);
    let mut c: Candidate =
        serde_json::from_value(candidate("operations", "process", "server")).unwrap();
    c.selection.include = vec!["/read".into()];
    let mut doc = spec();
    for index in 0..10_000 {
        doc["paths"][format!("/excluded-{index}")] = json!({"get":{}});
    }
    let logs = capture_limit_logs(|| {
        let result = import::build(&p, &c, &serde_json::to_vec(&doc).unwrap());
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("operation_count_limit")
        );
    });
    assert!(logs.contains("WARN"), "{logs}");
    assert!(logs.contains("operation_count_limit"), "{logs}");
    assert!(logs.contains("10000"), "{logs}");
}

#[tokio::test]
async fn oversized_management_request_warns_without_logging_arguments() {
    use tracing::instrument::WithSubscriber;
    let m = manager(None).await;
    let logs = LimitLogs::default();
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let result = m
        .invoke(
            "writer",
            "x402_treazury_source_add",
            json!({
                "spec_url":"https://api.example.com/openapi.json",
                "name":"PRIVATE_ARGUMENT".repeat(5_000)
            }),
        )
        .with_subscriber(subscriber)
        .await;
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("management_request_too_large")
    );
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("WARN"), "{logs}");
    assert!(logs.contains("management_request_too_large"), "{logs}");
    assert!(!logs.contains("PRIVATE_ARGUMENT"), "{logs}");
    assert!(!logs.contains("api.example.com"), "{logs}");
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[test]
fn import_policy_rejections_warn_without_private_content() {
    let p = policy(None);
    let c: Candidate = serde_json::from_value(candidate("policy", "process", "server")).unwrap();
    let mut cases = vec![(b"PRIVATE_INVALID_JSON".to_vec(), "invalid OpenAPI JSON")];
    for (target, field, value, reason) in [
        (
            "components",
            "$ref",
            json!("https://private.example/PRIVATE_REFERENCE"),
            "external_reference_unsupported",
        ),
        (
            "components",
            "$ref",
            json!("#/PRIVATE_MISSING"),
            "unresolved_reference",
        ),
        (
            "components",
            "$ref",
            json!("#/components"),
            "cyclic_reference_unsupported",
        ),
        (
            "path",
            "servers",
            json!([{ "url": "https://private.example/PRIVATE_SERVER" }]),
            "per-path servers unsupported",
        ),
        (
            "operation",
            "servers",
            json!([{ "url": "https://private.example/PRIVATE_SERVER" }]),
            "per-operation servers unsupported",
        ),
    ] {
        let mut doc = spec();
        match target {
            "components" => doc["components"] = json!({field: value}),
            "path" => doc["paths"]["/read"][field] = value,
            _ => doc["paths"]["/read"]["get"][field] = value,
        }
        cases.push((serde_json::to_vec(&doc).unwrap(), reason));
    }
    for (bytes, reason) in cases {
        let logs = capture_limit_logs(|| {
            let error = import::build(&p, &c, &bytes).unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        });
        assert!(logs.contains("WARN") && logs.contains(reason), "{logs}");
        assert!(
            !logs.contains("PRIVATE_") && !logs.contains("private.example"),
            "{logs}"
        );
    }
    let logs = capture_limit_logs(|| {
        assert!(import::endpoint(&p, "https://127.0.0.1/PRIVATE_DESTINATION").is_err());
    });
    assert!(
        logs.contains("WARN") && logs.contains("public_destination_rejected"),
        "{logs}"
    );
    assert!(
        !logs.contains("PRIVATE_") && !logs.contains("127.0.0.1"),
        "{logs}"
    );
}

async fn capture_future_logs<T>(future: impl std::future::Future<Output = T>) -> (T, String) {
    use tracing::instrument::WithSubscriber;
    let logs = LimitLogs::default();
    let sink = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let result = future.with_subscriber(subscriber).await;
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    (result, text)
}

#[tokio::test]
async fn saturated_mutation_capacity_warns_before_publication() {
    let m = manager(None).await;
    seed(&m).await;
    let _held = m.mutations.clone().try_acquire_many_owned(16).unwrap();
    let (result, logs) = capture_future_logs(m.invoke(
        "writer",
        "x402_treazury_source_add",
        json!({"spec_url":"https://api.example.com/openapi.json"}),
    ))
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("mutation_queue_full")
    );
    assert!(
        logs.contains("WARN") && logs.contains("mutation_queue_full"),
        "{logs}"
    );
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    assert!(m.catalog.read().views.values().all(Vec::is_empty));
}

#[tokio::test]
async fn import_redirect_and_read_timeout_warn_without_response_prose() {
    for stall in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/PRIVATE_SPEC", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            if stall {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_secs(10)).await;
            } else {
                socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://private.example/PRIVATE_REDIRECT\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            }
        });
        let mut p = policy(None);
        p.read_timeout_seconds = Some(1);
        let (result, logs) = capture_future_logs(import::fixture_fetch(&p, &url)).await;
        let error = result.unwrap_err();
        if stall {
            assert!(
                error
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_timeout),
                "{error:#}"
            );
        } else {
            assert_eq!(
                crate::discovery_relay::document::SharedFailure::capture(&error, "fixture")
                    .status(),
                Some(302)
            );
        }
        assert!(logs.contains("WARN"), "{logs}");
        assert!(
            logs.contains(if stall {
                "transport timeout"
            } else {
                "HTTP response rejected"
            }),
            "{logs}"
        );
        assert!(
            !logs.contains("PRIVATE_") && !logs.contains("private.example"),
            "{logs}"
        );
        task.abort();
    }
}

#[tokio::test]
async fn reserved_source_delimiter_warns_for_add_inspection_and_restore() {
    let m = manager(None).await;
    let (result, logs) = capture_future_logs(m.invoke(
        "writer",
        "x402_treazury_source_add",
        json!({"spec_url":"https://private.example.com/openapi.json","name":"PRIVATE__LABEL"}),
    ))
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("source_name_contains_reserved_double_underscore")
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(
        logs.contains("WARN") && logs.contains("source_name_contains_reserved_double_underscore"),
        "{logs}"
    );
    assert!(
        !logs.contains("PRIVATE") && !logs.contains("private.example"),
        "{logs}"
    );
    let c: Candidate =
        serde_json::from_value(candidate("PRIVATE__LABEL", "process", "server")).unwrap();
    let logs = capture_limit_logs(|| {
        assert!(import::saved_tools(&c, &serde_json::to_vec(&spec()).unwrap()).is_err());
    });
    assert!(
        logs.contains("WARN") && logs.contains("source_name_contains_reserved_double_underscore"),
        "{logs}"
    );
    let added = add(&m, "valid", "", "").await;
    let id = registered_id(&m, &added);
    m.inner
        .lock()
        .unwrap()
        .state
        .records
        .get_mut(&id)
        .unwrap()
        .candidate
        .name = "PRIVATE__LABEL".into();
    let logs = capture_limit_logs(|| m.restore().unwrap());
    assert!(
        logs.contains("WARN") && logs.contains("source_name_contains_reserved_double_underscore"),
        "{logs}"
    );
    assert!(!logs.contains("PRIVATE"), "{logs}");
    assert!(m.catalog.read().views["writer"].is_empty());
}
#[tokio::test]
async fn advertised_inventory_warning_is_first_page_only_and_observational() {
    use rmcp::{ServerHandler, model::PaginatedRequestParams, service::RequestContext};
    use tracing::instrument::WithSubscriber;
    let m = manager(None).await;
    add(&m, "accepted", "", "").await;
    let template = m.catalog.read().views["writer"][0].clone();
    for (owner, on_demand, api_count, expected_warnings) in [
        ("hidden", false, 250, 0),
        ("hidden", false, 251, 2),
        ("writer", false, 243, 0),
        ("writer", false, 244, 2),
        ("writer", true, 513, 0),
    ] {
        let mut snapshot = (*m.catalog.read()).clone();
        snapshot.generation += 1;
        snapshot.views.insert(
            owner.into(),
            (0..api_count)
                .map(|index| {
                    let mut bound = template.clone();
                    bound.tool.name = format!("fixture_{index}");
                    bound.tool.description = "PRIVATE_DESCRIPTION https://private.invalid".into();
                    if index % 2 == 0 {
                        bound.source = None;
                    }
                    bound
                })
                .collect(),
        );
        m.catalog.publish(snapshot);
        let mut endpoint = server(&m, owner);
        endpoint.discover_on_demand = on_demand;
        // A peer supplies the request context; listing itself is exercised directly,
        // with no client transport, wallet lookup or provider work.
        let (transport, _client) = tokio::io::duplex(1024);
        let running = rmcp::service::serve_directly(endpoint.clone(), transport, None);
        let list = |cursor: Option<String>| {
            endpoint.list_tools(
                cursor.map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor))),
                RequestContext::new(rmcp::model::RequestId::Number(1), running.peer().clone()),
            )
        };
        let logs = LimitLogs::default();
        let sink = logs.clone();
        let subscriber = tracing::Dispatch::new(
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(move || sink.clone())
                .finish(),
        );
        let first = list(None)
            .with_subscriber(subscriber.clone())
            .await
            .unwrap();
        let count = if on_demand {
            7
        } else {
            api_count + if owner == "writer" { 7 } else { 0 }
        };
        assert_eq!(first.tools.len(), count.min(25));
        let after_first = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            after_first
                .matches("Large advertised tool inventory")
                .count(),
            usize::from(expected_warnings > 0)
        );
        let prefix = format!(
            "{}:{}:{}:",
            m.catalog.instance(),
            owner,
            m.catalog.read().generation
        );
        for cursor in [
            format!("{prefix}0"),
            format!("{prefix}25"),
            "invalid".into(),
        ] {
            let result = list(Some(cursor.clone()))
                .with_subscriber(subscriber.clone())
                .await;
            if cursor == "invalid" || count < 25 && cursor.ends_with(":25") {
                assert!(result.is_err());
            } else {
                assert!(result.is_ok());
            }
        }
        assert_eq!(
            String::from_utf8(logs.0.lock().unwrap().clone()).unwrap(),
            after_first
        );
        list(None).with_subscriber(subscriber).await.unwrap();
        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert_eq!(
            logs.matches("Large advertised tool inventory").count(),
            expected_warnings
        );
        if expected_warnings > 0 {
            for expected in [
                "WARN",
                owner,
                &format!("advertised_tools={count}"),
                "discover_on_demand = true",
            ] {
                assert!(logs.contains(expected), "{logs}");
            }
        }
        assert!(!logs.contains("PRIVATE_DESCRIPTION"));
        assert!(!logs.contains("private.invalid"));
        assert_eq!(m.catalog.read().views[owner].len(), api_count);
        running.cancel().await.unwrap();
    }
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
}
