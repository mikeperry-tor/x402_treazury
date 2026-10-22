//! Real bound invocation preserves curated image, help, compatibility and network policy.
use super::*;

#[test]
fn curated_bound_image_and_help_use_accepted_routes_and_public_transports() {
    if std::env::var_os("TREAZURY_CURATED_NETWORK_CHILD").is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery::tests::curated_network_tests::curated_bound_image_and_help_use_accepted_routes_and_public_transports",
            "--nocapture",
        ])
        .env("TREAZURY_CURATED_NETWORK_CHILD", "1")
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn fixture() {
    use crate::{
        catalog::Config,
        discovery::curated::AcceptedProvider,
        network::{IsolationId, Mode, NetworkContext, NetworkPolicy, ScopeObservation},
        test_socks::{Fault, Socks},
    };
    use axum::{
        body::Bytes,
        http::{HeaderMap, Method, StatusCode, Uri},
        response::IntoResponse,
        routing::any,
    };
    use std::sync::atomic::Ordering;

    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP8sAAAAASUVORK5CYII=";
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = axum::Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
        let captured = captured.clone();
        async move {
            assert!(!headers.contains_key("payment-signature"));
            assert!(!headers.contains_key("x-payment"));
            assert!(!headers.contains_key("authorization"));
            captured.lock().unwrap().push((method.to_string(), uri.to_string(), headers.get("host").unwrap().to_str().unwrap().to_owned(), body.to_vec()));
            match uri.path() {
                "/openapi.json" => axum::Json(json!({"openapi":"3.0.3","servers":[{"url":"https://api.example.com"}],"paths":{"/read":{"get":{}}}})).into_response(),
                "/v1/image" => {
                    assert_eq!(method, Method::POST);
                    assert_eq!(uri.query(), Some("format=png"));
                    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({"prompt":"a map"}));
                    axum::Json(json!({"image_url":format!("data:image/png;base64,{PNG}"),"note":"Fixture attribution"})).into_response()
                },
                "/help" => {
                    assert_eq!(method, Method::GET);
                    assert!(body.is_empty());
                    ([("content-type", "text/plain")], "# Curated fixture help\nUse the selected tool schemas.").into_response()
                },
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }));
    // Requiring both compatibility settings proves they reach API and help clients.
    let (address, task) =
        crate::test_tls::serve_with(app, &[&rustls::version::TLS12], &[b"http/1.1"]).await;
    // The fixture CA covers spec.example.com, used here as the separate docs origin.
    let proxy = Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
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
    let observations = Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
    let observed = observations.clone();
    crate::network::install_http_scope_observer(move |observation| {
        observed.lock().unwrap().push(observation.clone());
        Ok(())
    })
    .unwrap();

    let mut p = policy(None);
    p.allowed_origins = vec![
        "https://api.example.com".into(),
        "https://spec.example.com".into(),
    ];
    let m = manager_with(p, listeners(false)).await;
    let provider = AcceptedProvider::new("authored", Config {
        spec: "accepted-schema.json".into(),
        prefix: Some("original".into()),
        base_url: Some("https://api.example.com".into()),
        include: vec!["/v1".into()],
        help_url: Some("https://spec.example.com/help".into()),
        allow_http1: true,
        allow_tls12: true,
        response_mappings: serde_json::from_value(json!({"POST /v1/image":{"images":[{"pointer":"/image_url","encoding":"data_uri"}]}})).unwrap(),
        ..Default::default()
    });
    let document = json!({"openapi":"3.0.3","servers":[{"url":"https://broken.example.com/wrong"}],"paths":{
        "/v1/image":{"post":{"description":"Generate fixture image","parameters":[{"name":"format","in":"query","schema":{"type":"string"}}],"requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"prompt":{"type":"string"}},"required":["prompt"]}}}}}}
    }}).to_string();
    let candidate: Candidate =
        serde_json::from_value(json!({"name":"custom","provider":provider})).unwrap();
    let id = Uuid::new_v4().to_string();
    m.commit(
        "writer".into(),
        false,
        Record {
            id: id.clone(),
            owner: "writer".into(),
            candidate,
            targets: vec![],
            revision: 1,
            hash: hash(document.as_bytes()),
            document,
            format: FORMAT,
            accepted_at: epoch(),
            updated_at: epoch(),
            removed: false,
            disabled: None,
        },
    )
    .await
    .unwrap();
    assert!(
        requests.lock().unwrap().is_empty(),
        "registration builds accepted bytes without API/help traffic"
    );
    assert_eq!(proxy.accepted.load(Ordering::SeqCst), 0);

    let snapshot = m.catalog.read();
    let image = snapshot.views["writer"]
        .iter()
        .find(|bound| bound.tool.name == "dyn_custom__image")
        .unwrap()
        .clone();
    let help = snapshot.views["writer"]
        .iter()
        .find(|bound| bound.tool.name == "dyn_custom__help")
        .unwrap()
        .clone();
    assert_eq!(image.source, Some((id, 1)));
    assert_eq!(image.base, "https://api.example.com");
    let output = image
        .invoke_output(
            json!({"prompt":"a map","format":"png"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(output.images.len(), 1);
    assert_eq!(output.images[0].mime_type, "image/png");
    assert_eq!(output.images[0].data, PNG);
    assert!(output.text.contains("Fixture attribution"));
    assert!(!output.text.contains(PNG));
    let output = help
        .invoke_output(json!({}).as_object().unwrap())
        .await
        .unwrap();
    assert!(output.images.is_empty());
    assert!(output.text.contains("# Curated fixture help"));
    help.invoke_output(json!({}).as_object().unwrap())
        .await
        .unwrap();
    {
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2, "successful help is cached");
        assert_eq!(seen[0].0, "POST");
        assert_eq!(seen[0].1, "/v1/image?format=png");
        assert_eq!(seen[0].2, "api.example.com");
        assert_eq!(seen[1].0, "GET");
        assert_eq!(seen[1].1, "/help");
        assert_eq!(seen[1].2, "spec.example.com");
    }
    // Public destination enforcement is checked again at runtime even if a bound
    // help route is replaced after registration; rejection precedes SOCKS traffic.
    let accepted = proxy.accepted.load(Ordering::SeqCst);
    for private in [
        "https://127.0.0.1/private",
        "https://10.0.0.1/private",
        "https://[::1]/private",
        "https://localhost/private",
        "https://host.internal/private",
    ] {
        let mut invalid = help.clone();
        invalid.tool.help_url = Some(private.into());
        invalid.help = Arc::new(OnceCell::new());
        assert!(
            invalid
                .invoke_output(json!({}).as_object().unwrap())
                .await
                .is_err(),
            "{private}"
        );
        assert_eq!(proxy.accepted.load(Ordering::SeqCst), accepted);
    }
    let mut private_api = image.clone();
    private_api.base = "https://127.0.0.1".into();
    assert!(
        private_api
            .invoke_output(
                json!({"prompt":"a map","format":"png"})
                    .as_object()
                    .unwrap()
            )
            .await
            .is_err()
    );
    assert_eq!(proxy.accepted.load(Ordering::SeqCst), accepted);
    assert_eq!(requests.lock().unwrap().len(), 2);

    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    let owner = crate::network::global()
        .credentials(&IsolationId::evm(&signer.address().to_string()).unwrap());
    {
        let seen = observations.lock().unwrap();
        let api = seen
            .iter()
            .find(|record| record.origin == "https://api.example.com")
            .unwrap();
        assert_eq!(api.canonical, owner);
        let help_credentials = crate::network::global()
            .credentials(&IsolationId::discovery("https://spec.example.com/help").unwrap());
        let socks = proxy.records.lock().unwrap();
        assert!(socks.iter().any(|record| record.host == "api.example.com"
            && (record.user.clone(), record.password.clone()) == api.scope));
        assert!(socks.iter().any(|record| record.host == "spec.example.com"
            && (record.user.clone(), record.password.clone()) == help_credentials));
        assert!(socks.iter().all(|record| record.address_type == 3));
    }
    // Identical URLs with different acquisition compatibility must not share a
    // cached error or in-flight fetch. The fixture requires TLS 1.2 and HTTP/1.
    let schema_url = "https://spec.example.com/openapi.json";
    let compatible = AcceptedProvider::new(
        "remote",
        Config {
            spec: schema_url.into(),
            allow_http1: true,
            allow_tls12: true,
            ..Default::default()
        },
    );
    let (strict, curated) = tokio::join!(
        m.document_with("writer", schema_url, None),
        m.document_with("reader", schema_url, Some(&compatible)),
    );
    assert!(strict.is_err());
    let bytes = curated.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["openapi"],
        "3.0.3"
    );
    let before = requests.lock().unwrap().len();
    let repeated = m
        .document_with("reader", schema_url, Some(&compatible))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&bytes, &repeated));
    assert_eq!(requests.lock().unwrap().len(), before);
    task.abort();
}
