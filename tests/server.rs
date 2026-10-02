use axum::{Router, routing::get};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use x402_treazure::{
    catalog::{Config, build_tools},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};

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
async fn authenticated_stateless_http_initializes_lists_and_calls() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (vendor, vendor_task) = serve(
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
    let server = Server::new(tools, client, vendor, cfg.instructions_text, None);
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
    let initialized: Value = response.json().await.unwrap();
    assert_eq!(initialized["result"]["instructions"], "Read help first");
    let listed: Value = request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for (name, expected) in [
        ("test_hello", "hello"),
        ("test_help", "vendor documentation"),
        ("test_help", "vendor documentation"),
    ] {
        let result: Value = request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":{}}})).send().await.unwrap().json().await.unwrap();
        assert_eq!(result["result"]["content"][0]["text"], expected, "{result}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let error: Value = request(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"missing","arguments":{}}})).send().await.unwrap().json().await.unwrap();
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
    assert_eq!(tool.input_schema["required"], json!(["id", "q_body"]));
    assert_eq!(
        tool.input_schema["properties"]["q_body"]["exclusiveMinimum"],
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
            json!({"id":"a/b?x","q":"query","q_body":2})
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
