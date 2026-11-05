#[path = "support/tls.rs"]
mod tls;
use x402_treazury::network;
#[path = "support/log_capture.rs"]
mod log_capture;
use axum::{Router, routing::get};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use x402_treazury::mcp_wire::McpResponse;
use x402_treazury::{
    catalog::{Config, build_tools},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};

async fn serve_provider(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    tls::install_trust();
    let (address, task) = tls::serve(app).await;
    (format!("https://{address}"), task)
}
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
    )
}

#[tokio::test]
async fn direct_and_discovered_api_errors_keep_status_without_diagnostic_chains() {
    tls::install_trust();
    use axum::{
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    let signed = Arc::new(AtomicUsize::new(0));
    let unsigned = Arc::new(AtomicUsize::new(0));
    let (s, u) = (signed.clone(), unsigned.clone());
    let (vendor, vendor_task) = serve_provider(Router::new().route("/status/{code}", get(move |Path(code): Path<u16>, headers: HeaderMap| {
        let (s,u) = (s.clone(),u.clone());
        async move {
            if headers.contains_key("payment-signature") {
                s.fetch_add(1, Ordering::SeqCst);
                return (StatusCode::from_u16(code).unwrap(), "insufficient_funds PRIVATE SDK trace {} null").into_response();
            }
            u.fetch_add(1, Ordering::SeqCst);
            let challenge = json!({"x402Version":2,"resource":{"url":"https://provider.example/status"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":x402_treazury::payment::USDC,"amount":"1000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
            (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
        }
    }))).await;
    let tools = build_tools(&Config::default(), &json!({"paths":{"/status/{code}":{"get":{"parameters":[{"name":"code","in":"path","required":true,"schema":{"type":"integer"}}]}}}}), "fixture").unwrap();
    for discovered in [false, true] {
        let client = PaidClient::new(
            Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
        );
        let mut server = Server::new(tools.clone(), client, vendor.clone(), None);
        server.discover_on_demand = discovered;
        let name = tools[0].name.clone();
        let (base, task) = serve(http_app(server, "token".into())).await;
        let local = reqwest::Client::new();
        for (code, kind) in [
            (402, "payment rejected"),
            (403, "access denied"),
            (404, "not found"),
            (429, "rate limited"),
            (503, "provider unavailable"),
        ] {
            let params = if discovered {
                json!({"name":"x402_treazury_tool_call","arguments":{"tool_id":name,"arguments":{"code":code}}})
            } else {
                json!({"name":name,"arguments":{"code":code}})
            };
            let result: Value = local
                .post(format!("{base}/mcp"))
                .bearer_auth("token")
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":code,"method":"tools/call","params":params}))
                .send()
                .await
                .unwrap()
                .mcp_json()
                .await
                .unwrap();
            assert_eq!(result["result"]["isError"], true, "{result}");
            assert_eq!(
                result["result"]["content"].as_array().unwrap().len(),
                1,
                "opaque static SDK offers have no observation"
            );
            let status = StatusCode::from_u16(code).unwrap();
            assert_eq!(
                result["result"]["content"][0]["text"],
                format!("Provider API HTTP {status}: {kind}. Do not retry automatically.")
            );
        }
        task.abort();
    }
    assert_eq!(unsigned.load(Ordering::SeqCst), 10);
    assert_eq!(signed.load(Ordering::SeqCst), 10);
    vendor_task.abort();
}

#[tokio::test]
async fn direct_and_on_demand_calls_route_declared_headers_and_own_distinct_keys() {
    tls::install_trust();
    use axum::http::HeaderMap;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let (base, task) = serve_provider(Router::new().route(
        "/read",
        get(move |headers: HeaderMap| {
            captured.lock().unwrap().push((
                headers["x-quote-id"].to_str().unwrap().to_owned(),
                headers["idempotency-key"].to_str().unwrap().to_owned(),
            ));
            async { "complete result" }
        }),
    ))
    .await;
    let tools = build_tools(
        &Config::default(),
        &json!({"paths":{"/read":{"get":{
            "parameters":[
                {"name":"X-Quote-Id","in":"header","required":true,"schema":{"type":"string"}},
                {"name":"Idempotency-Key","in":"header","required":true,"schema":{"type":"string"}}
            ]
        }}}}),
        "fixture",
    )
    .unwrap();
    let mut server = Server::new(tools, PaidClient::unsigned(), base, None);
    let args = json!({"X-Quote-Id":"quote-from-agent"});
    assert_eq!(
        server
            .invoke("fixture_read", args.as_object().unwrap())
            .await
            .unwrap(),
        "complete result"
    );
    server.discover_on_demand = true;
    let search: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_tools_search",
                json!({"query":"fixture_read"}).as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let properties = &search["items"][0]["input_schema"]["properties"];
    assert!(properties.get("X-Quote-Id").is_some(), "{search}");
    assert!(properties.get("Idempotency-Key").is_none());
    assert_eq!(
        server
            .invoke(
                "x402_treazury_tool_call",
                json!({"tool_id":"fixture_read","arguments":args})
                    .as_object()
                    .unwrap()
            )
            .await
            .unwrap(),
        "complete result"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter()
            .all(|(quote, key)| quote == "quote-from-agent" && uuid::Uuid::parse_str(key).is_ok())
    );
    assert_ne!(seen[0].1, seen[1].1);
    task.abort();
}

#[tokio::test]
async fn authenticated_mcp_paid_call_signs_once_and_delivers_the_provider_result() {
    tls::install_trust();
    use axum::http::{HeaderMap, StatusCode};
    use base64::{Engine, engine::general_purpose::STANDARD};
    tls::install_trust();
    let listener = tls::listener().await;
    let vendor = format!(
        "https://{}",
        axum::serve::Listener::local_addr(&listener).unwrap()
    );
    let challenge = STANDARD.encode(
        json!({
            "x402Version": 2,
            "resource": {"url": format!("{vendor}/read"), "mimeType": "text/plain"},
            "accepts": [{"scheme": "exact", "network": "eip155:8453", "amount": "1000",
                "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                "payTo": "0x0000000000000000000000000000000000000002",
                "maxTimeoutSeconds": 60, "extra": {"name": "USD Coin", "version": "2"}}]
        })
        .to_string(),
    );
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let (initial, retry) = (unsigned.clone(), signed.clone());
    let vendor_task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/read",
                get(move |headers: HeaderMap| {
                    let (challenge, initial, retry) =
                        (challenge.clone(), initial.clone(), retry.clone());
                    async move {
                        if headers.contains_key("payment-signature") {
                            retry.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::OK,
                                [("content-type", "text/plain".to_owned())],
                                "paid result",
                            )
                        } else {
                            initial.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::PAYMENT_REQUIRED,
                                [("payment-required", challenge)],
                                "payment required",
                            )
                        }
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });
    let tools = build_tools(
        &Config::default(),
        &json!({"paths":{"/read":{"get":{}}}}),
        "test",
    )
    .unwrap();
    let payer = Payer::new(
        &format!("{:064x}", 1),
        SpendPolicy::dollars("0.20").unwrap(),
    )
    .unwrap();
    let server = Server::new(tools, PaidClient::new(payer), vendor, None);
    let (mcp, task) = serve(http_app(server, "test-token".into())).await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let request = || {
        http.post(format!("{mcp}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"test_read","arguments":{}}}))
    };
    for (authorization, message) in [
        (None, "missing Authorization header"),
        (Some("Basic never-log-this"), "invalid Authorization header"),
        (Some("Bearer wrong-secret"), "incorrect bearer token value"),
    ] {
        let mut request = request();
        if let Some(value) = authorization {
            request = request.header("Authorization", value);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
        let body = response.text().await.unwrap();
        assert!(body.contains(message), "{body}");
        assert!(!body.contains("never-log-this") && !body.contains("wrong-secret"));
    }
    assert_eq!(unsigned.load(Ordering::SeqCst), 0);
    assert_eq!(signed.load(Ordering::SeqCst), 0);
    let result: Value = request()
        .header("authorization", "bEaReR test-token")
        .send()
        .await
        .unwrap()
        .mcp_json()
        .await
        .unwrap();
    assert_ne!(result["result"]["isError"], true, "{result}");
    assert_eq!(result["result"]["content"][0]["text"], "paid result");
    assert_eq!(unsigned.load(Ordering::SeqCst), 1);
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    task.abort();
    vendor_task.abort();
}

#[tokio::test]
async fn disabled_cover_does_not_shadow_an_ordinary_api_tool() {
    tls::install_trust();
    use rmcp::ServerHandler;
    let (vendor, task) =
        serve_provider(Router::new().route("/cover_status", get(|| async { "ordinary API" })))
            .await;
    let tools = build_tools(
        &Config::default(),
        &json!({"paths":{"/cover_status":{"get":{}}}}),
        "x402_treazury",
    )
    .unwrap();
    assert_eq!(tools[0].name, "x402_treazury_cover_status");
    let server = Server::new(tools, PaidClient::unsigned(), vendor, None);
    assert!(server.get_tool("x402_treazury_cover_status").is_some());
    assert_eq!(
        server
            .invoke("x402_treazury_cover_status", &Default::default())
            .await
            .unwrap(),
        "ordinary API"
    );
    task.abort();
}
#[tokio::test]
async fn authenticated_stateless_http_initializes_lists_and_calls() {
    tls::install_trust();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (vendor, vendor_task) = serve_provider(
        Router::new()
            .route("/hello", get(|| async { "hello" }))
            .route(
                "/llms.txt",
                get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "vendor documentation"
                    }
                }),
            ),
    )
    .await;
    let cfg = Config {
        help_url: Some(format!("{vendor}/llms.txt")),
        instructions_text: Some("Read help first".into()),
        ..Default::default()
    };
    let root = json!({"paths":{"/hello":{"get":{"description":"Say hello"}}}});
    let tools = build_tools(&cfg, &root, "test").unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let server = Server::new(tools, client, vendor, cfg.instructions_text);
    let catalog = server.catalog.clone();
    let (base, task) = serve(http_app(server, "test-token".into())).await;
    let endpoint = format!("{base}/mcp");
    for token in [None, Some("wrong")] {
        let mut request = http.post(&endpoint);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.json(&json!({})).send().await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    let request = |body: Value| {
        http.post(&endpoint)
            .bearer_auth("test-token")
            .header("accept", "application/json, text/event-stream")
            .json(&body)
    };
    let response = request(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("mcp-session-id").is_none());
    let initialized: Value = response.mcp_json().await.unwrap();
    let instructions = initialized["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("Read help first\n\n"));
    assert_eq!(
        instructions
            .matches("Prices are estimates or sampled payment offers")
            .count(),
        1
    );
    assert!(instructions.ends_with("Unknown does not mean free."));
    assert_ne!(
        initialized["result"]["capabilities"]["tools"]["listChanged"],
        true
    );
    let malformed = http
        .post(&endpoint)
        .bearer_auth("test-token")
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .body("{broken")
        .send()
        .await
        .unwrap();
    assert!(malformed.status().is_client_error());
    let listed: Value = request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .mcp_json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(
        listed["result"]["_meta"]["x402_treazury/catalog"]["instructions"],
        "Read help first"
    );
    let mut updated = (*catalog.read()).clone();
    updated.generation += 1;
    updated
        .instructions
        .insert("default".into(), Some("Source late: PENDING".into()));
    updated
        .instructions
        .insert("other".into(), Some("Private other listener".into()));
    catalog.publish(updated);
    let refreshed: Value = request(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .mcp_json()
        .await
        .unwrap();
    let status = &refreshed["result"]["_meta"]["x402_treazury/catalog"];
    assert_eq!(status["instructions"], "Source late: PENDING");
    assert_eq!(status["generation"], 1);
    assert_eq!(status["publication_stopped"], false);
    assert!(!refreshed.to_string().contains("Private other listener"));
    assert_eq!(refreshed["result"]["tools"], listed["result"]["tools"]);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for (name, expected) in [
        ("test_hello", "hello"),
        ("test_help", "vendor documentation"),
        ("test_help", "vendor documentation"),
    ] {
        let result: Value = request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":{}}})).send().await.unwrap().mcp_json().await.unwrap();
        assert_eq!(result["result"]["content"][0]["text"], expected, "{result}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let error: Value = request(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"missing","arguments":{}}})).send().await.unwrap().mcp_json().await.unwrap();
    assert_eq!(error["result"]["isError"], true);
    task.abort();
    vendor_task.abort();
}

#[test]
fn schemas_filters_and_routes_handle_boundaries_collisions_and_refs() {
    let cfg = Config {
        include: vec!["/v1/web".into()],
        ..Default::default()
    };
    let root = json!({"components":{"schemas":{"Body":{"type":"object","properties":{"q":{"type":"number","minimum":0,"exclusiveMinimum":true}},"required":["q"]}}},
        "paths":{"/v1/webhook":{"get":{}},"/v1/web/{id}":{"post":{"parameters":[
            {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
            {"name":"q","in":"query","schema":{"type":"string"}},
            {"name":"authorization","in":"header","schema":{"type":"string"}}],
            "requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Body"}}}}}}}});
    let tools = build_tools(&cfg, &root, "t").unwrap();
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(tool.input_schema["required"], json!(["id"]));
    assert_eq!(
        tool.input_schema["properties"]["body"]["required"],
        json!(["q"])
    );
    assert_eq!(
        tool.input_schema["properties"]["body"]["properties"]["q"]["exclusiveMinimum"],
        0
    );
    assert!(
        tool.input_schema["properties"]
            .get("authorization")
            .is_none()
    );
    let route = tool
        .route(
            "https://example.test/gateway",
            json!({"id":"a/b?x","q":"query","body":{"q":2}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(route.url, "https://example.test/gateway/v1/web/a%2Fb%3Fx");
    assert_eq!(route.query, BTreeMap::from([("q".into(), json!("query"))]));
    assert_eq!(route.body, Some(json!({"q":2})));
    assert!(
        tool.route("https://example.test", &Default::default())
            .is_err()
    );
}

#[tokio::test]
async fn catalog_replacement_preserves_inflight_routes_and_updates_all_views() {
    tls::install_trust();
    use x402_treazury::catalog_state::{self, CatalogSnapshot};
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (e, r) = (entered.clone(), release.clone());
    let (url, task) = serve_provider(
        Router::new()
            .route(
                "/old",
                get(move || {
                    let (e, r) = (e.clone(), r.clone());
                    async move {
                        e.notify_one();
                        r.notified().await;
                        "old"
                    }
                }),
            )
            .route("/new", get(|| async { "new" })),
    )
    .await;
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let mut tool = build_tools(
        &Config::default(),
        &json!({"paths":{"/old":{"get":{}}}}),
        "t",
    )
    .unwrap()
    .remove(0);
    let server = Server::new(vec![tool.clone()], client.clone(), url.clone(), None);
    let old = server.clone();
    let name = tool.name.clone();
    let pending =
        tokio::spawn(async move { old.invoke(&name, &serde_json::Map::new()).await.unwrap() });
    entered.notified().await;
    tool.path = "/new".into();
    let tools = catalog_state::bind(vec![(tool.clone(), client, url)]);
    server.catalog.publish(CatalogSnapshot {
        registrations: Default::default(),
        instructions: Default::default(),
        generation: 1,
        views: BTreeMap::from([("default".into(), tools.clone()), ("other".into(), tools)]),
    });
    assert_eq!(
        server
            .invoke(&tool.name, &serde_json::Map::new())
            .await
            .unwrap(),
        "new"
    );
    assert_eq!(server.catalog.read().views["other"][0].tool.path, "/new");
    release.notify_one();
    assert_eq!(pending.await.unwrap(), "old");
    task.abort();
}

#[tokio::test]
async fn untagged_help_failures_warn_once_without_exposing_content() {
    tls::install_trust();
    if log_capture::run_in_child("untagged_help_failures_warn_once_without_exposing_content") {
        return;
    }
    use axum::http::{HeaderMap, StatusCode};
    use tracing::instrument::WithSubscriber;

    for (status, body, expected_warning) in [
        (
            StatusCode::OK,
            b"private-help-body\xff".to_vec(),
            Some("help_invalid_utf8"),
        ),
        (
            StatusCode::BAD_REQUEST,
            b"private-help-body".to_vec(),
            Some("Could not retrieve document"),
        ),
        (StatusCode::OK, b"useful help".to_vec(), None),
    ] {
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let (base, task) = serve_provider(Router::new().route(
            "/help",
            get(move |headers: HeaderMap| {
                observed.fetch_add(1, Ordering::SeqCst);
                assert!(!headers.contains_key("payment-signature"));
                let body = body.clone();
                async move { (status, body) }
            }),
        ))
        .await;
        let cfg = Config {
            help_url: Some(format!("{base}/help?token=private-help-token")),
            ..Default::default()
        };
        assert!(cfg.reliability_tags.is_empty());
        let tools = build_tools(&cfg, &json!({"paths":{"/unused":{"get":{}}}}), "fixture").unwrap();
        let server = Server::new(tools, PaidClient::unsigned(), base.clone(), None);
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let result = server
            .invoke("fixture_help", &Default::default())
            .with_subscriber(subscriber)
            .await;
        let logs = std::fs::read_to_string(log.path()).unwrap();
        let warnings: Vec<_> = logs.lines().filter(|line| line.contains("WARN")).collect();
        if let Some(expected) = expected_warning {
            assert!(result.is_err());
            assert_eq!(warnings.len(), 1, "{logs}");
            assert!(warnings[0].contains(expected), "{logs}");
            assert!(
                warnings[0].contains("authorization_may_charge=false"),
                "{logs}"
            );
        } else {
            assert_eq!(result.unwrap(), "useful help");
            assert!(warnings.is_empty(), "{logs}");
        }
        for private in ["private-help-body", "private-help-token", base.as_str()] {
            assert!(!logs.contains(private), "{logs}");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn help_failures_retry_concurrent_calls_coalesce_and_new_urls_get_new_content() {
    tls::install_trust();
    use axum::{
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
    };
    use x402_treazury::catalog_state::{self, CatalogSnapshot};
    let hits = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (h, e, r) = (hits.clone(), entered.clone(), release.clone());
    let (url, task) = serve_provider(
        Router::new()
            .route(
                "/help",
                get(move |headers: HeaderMap| {
                    let (h, e, r) = (h.clone(), e.clone(), r.clone());
                    async move {
                        assert!(!headers.contains_key("payment-signature"));
                        if h.fetch_add(1, Ordering::SeqCst) == 0 {
                            return (StatusCode::BAD_GATEWAY, "retry").into_response();
                        }
                        e.notify_one();
                        r.notified().await;
                        "old documentation".into_response()
                    }
                }),
            )
            .route("/new-help", get(|| async { "new documentation" })),
    )
    .await;
    let cfg = Config {
        help_url: Some(format!("{url}/help")),
        ..Default::default()
    };
    let tools = build_tools(&cfg, &json!({"paths":{"/unused":{"get":{}}}}), "help").unwrap();
    let client = PaidClient::new(
        Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let server = Server::new(tools.clone(), client.clone(), url.clone(), None);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(
        server
            .invoke("help_help", &Default::default())
            .await
            .is_err()
    );
    let mut callers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let s = server.clone();
        callers.spawn(async move { s.invoke("help_help", &Default::default()).await.unwrap() });
    }
    entered.notified().await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    release.notify_one();
    while let Some(result) = callers.join_next().await {
        assert_eq!(result.unwrap(), "old documentation");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let mut tool = tools.iter().find(|t| t.help_url.is_some()).unwrap().clone();
    tool.help_url = Some(format!("{url}/new-help"));
    server.catalog.publish(CatalogSnapshot {
        registrations: Default::default(),
        instructions: Default::default(),
        generation: 1,
        views: BTreeMap::from([(
            "default".into(),
            catalog_state::bind(vec![(tool, client, url)]),
        )]),
    });
    assert_eq!(
        server
            .invoke("help_help", &Default::default())
            .await
            .unwrap(),
        "new documentation"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    task.abort();
}

#[test]
fn catalog_rejects_ambiguous_names_and_preserves_nested_constraints_and_overrides() {
    let body = json!({"type":"object","properties":{"q":{"type":"string"}}});
    let collision = json!({"parameters":[{"name":"q","in":"query","schema":{"type":"string"}},{"name":"q_body","in":"query","schema":{"type":"string"}}],"requestBody":{"content":{"application/json":{"schema":body}}}});
    let disambiguated = build_tools(
        &Config::default(),
        &json!({"paths":{"/a-b":{"get":{}},"/a_b":{"get":{}}}}),
        "t",
    )
    .unwrap();
    assert_eq!(
        disambiguated
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["t_a_b_get", "t_a_b_get_2"]
    );
    assert_eq!(
        disambiguated
            .iter()
            .map(|t| t.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/a-b", "/a_b"]
    );
    assert!(
        build_tools(
            &Config::default(),
            &json!({"paths":{"/a-b":{"get":{}},"/a_b":{"get":{}},"/a_b_get_2":{"get":{}}}}),
            "t"
        )
        .is_err()
    );
    let collision_tools = build_tools(
        &Config::default(),
        &json!({"paths":{"/x":{"post":collision}}}),
        "t",
    )
    .unwrap();
    let routed = collision_tools[0]
        .route(
            "https://example.com",
            json!({"q":"query","q_body":"second query","body":{"q":"body"}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(routed.query["q"], "query");
    assert_eq!(routed.query["q_body"], "second query");
    assert_eq!(routed.body, Some(json!({"q":"body"})));
    let nested = json!({"type":"object","properties":{"boolean_bound":{"type":"number","minimum":3,"exclusiveMinimum":true},"numeric_bound":{"type":"number","exclusiveMaximum":9}}});
    let body = json!({"type":"object","properties":{"items":{"type":"array","items":nested}}});
    let operation = json!({"description":"雪🙂 vendor instructions kept whole", "parameters":[{"name":"Authorization","in":"header","schema":{"type":"string"}}],"requestBody":{"content":{"application/json":{"schema":body}}}});
    let doc = json!({"paths":{"/x":{"post":operation}}});
    let tools = build_tools(&Config::default(), &doc, "t").unwrap();
    let props = &tools[0].input_schema["properties"]["body"]["properties"];
    assert!(props.get("Authorization").is_none());
    assert_eq!(
        props["items"]["items"]["properties"]["boolean_bound"]["exclusiveMinimum"],
        3
    );
    assert_eq!(
        props["items"]["items"]["properties"]["numeric_bound"]["exclusiveMaximum"],
        9
    );
    assert!(
        tools[0]
            .description
            .contains("雪🙂 vendor instructions kept whole")
    );
    let cfg = Config {
        overrides: json!({"t_x":{"description":"Authored instructions"}}),
        ..Default::default()
    };
    let tools = x402_treazury::catalog::build_tools(&cfg, &doc, "t").unwrap();
    assert_eq!(tools[0].description, "Authored instructions");
    let route = tools[0]
        .route(
            "https://example.com",
            json!({"body":{"items":[{"boolean_bound":4,"numeric_bound":8}]}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        route.body.unwrap(),
        json!({"items":[{"boolean_bound":4,"numeric_bound":8}]})
    );
}

#[tokio::test]
async fn standalone_non_loopback_listener_is_rejected_before_provider_io() {
    tls::install_trust();
    let directory = tempfile::tempdir().unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
        .current_dir(directory.path())
        .args([
            "serve",
            "--transport",
            "http",
            "--host",
            "0.0.0.0",
            "--provider",
            "missing.toml",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("loopback"), "{diagnostic}");
    assert!(!diagnostic.contains("missing.toml"), "{diagnostic}");
}
