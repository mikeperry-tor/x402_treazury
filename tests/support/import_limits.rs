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
            assert_eq!(result.unwrap(), b"abcdefgh");
        } else {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains(match mode {
                    "overflow" => "spec_too_large",
                    "interrupted" => "source_read_failed",
                    _ => unreachable!(),
                }),
                "{mode}: {error}"
            );
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
async fn eager_endpoint_quota_warns_without_publishing() {
    let accepted = manager(None).await;
    add(&accepted, "accepted", "", "").await;
    let mut p = policy(None);
    p.max_tools_per_server = 1;
    let eager = manager_with(p, listeners(false)).await;
    let logs = capture_limit_logs(|| {
        let state = accepted.inner.lock().unwrap();
        let base = eager.inner.lock().unwrap();
        let result = eager.snapshot(&state.state, &base.base, 1);
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("server_tool_quota_exceeded")
        );
    });
    assert!(logs.contains("WARN"), "{logs}");
    assert!(logs.contains("max_tools_per_server"), "{logs}");
    assert!(logs.contains("actual=2"), "{logs}");
    assert!(eager.catalog.read().views.values().all(Vec::is_empty));
    assert!(eager.inner.lock().unwrap().state.records.is_empty());
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
async fn saturated_import_and_mutation_capacity_warns_before_io() {
    for global in [false, true] {
        let m = manager(None).await;
        let permits = if global {
            m.global_import.clone()
        } else {
            m.owner_import["writer"].clone()
        };
        let _held = permits
            .try_acquire_many_owned(if global { 4 } else { 1 })
            .unwrap();
        let (result, logs) = capture_future_logs(m.document(
            "writer",
            "https://api.example.com/PRIVATE_SPEC",
            false,
        ))
        .await;
        let reason = if global {
            "imports_busy"
        } else {
            "owner_import_busy"
        };
        assert!(result.unwrap_err().to_string().contains(reason));
        assert!(logs.contains("WARN") && logs.contains(reason), "{logs}");
        assert!(
            !logs.contains("PRIVATE_") && !logs.contains("api.example.com"),
            "{logs}"
        );
        assert!(m.inner.lock().unwrap().state.records.is_empty());
    }
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
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains(if stall {
                "source_read_failed"
            } else {
                "source_http_302"
            }),
            "{error}"
        );
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
    m.inner
        .lock()
        .unwrap()
        .state
        .records
        .get_mut(added["source_id"].as_str().unwrap())
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
