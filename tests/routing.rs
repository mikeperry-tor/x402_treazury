use axum::{Router, extract::Request};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use x402_treazury::{
    catalog::{Config, build_tools},
    payment::{PaidClient, Payer, SpendPolicy},
    server::Server,
};

fn path_tools() -> Vec<x402_treazury::catalog::ToolSpec> {
    build_tools(
        &Config::default(),
        &json!({"paths":{"/users/{id}/settings":{"put":{
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}]
        }}}}),
        "route",
    )
    .unwrap()
}

#[test]
fn assembled_template_cannot_form_normalized_dot_segments() {
    for path in [
        "/users/.{suffix}/settings",
        "/users/%2e{suffix}/settings",
        "/users/..{suffix}/settings",
        "/users/%2e%2e{suffix}/settings",
    ] {
        let tools = build_tools(&Config::default(), &json!({"operations":[{"method":"PUT","path":path,"params":[{"name":"suffix","in":"path"}]}]}), "route").unwrap();
        assert!(
            tools[0]
                .route(
                    "https://example.com",
                    json!({"suffix":""}).as_object().unwrap()
                )
                .is_err()
        );
        assert!(
            tools[0]
                .route(
                    "https://example.com",
                    json!({"suffix":"txt"}).as_object().unwrap()
                )
                .is_ok()
        );
    }
}

#[tokio::test]
async fn inherited_parameters_route_and_prevent_incomplete_pricing_probes() {
    let root = json!({"openapi":"3.1.0","components":{"parameters":{
        "id":{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
        "query":{"name":"q","in":"query","required":true,"schema":{"type":"string"}}
    }},"paths":{
        "/users/{id}":{"parameters":[{"$ref":"#/components/parameters/id"},{"$ref":"#/components/parameters/query"}],
            "get":{"parameters":[{"name":"q","in":"query","required":false,"schema":{"type":"integer"}}]},"delete":{}},
        "/search":{"parameters":[{"$ref":"#/components/parameters/query"}],"get":{}}
    }});
    let cfg = Config::default();
    let tools = build_tools(&cfg, &root, "route").unwrap();
    let get = tools
        .iter()
        .find(|t| t.path == "/users/{id}" && t.method == "GET")
        .unwrap();
    assert_eq!(get.input_schema["required"], json!(["id"]));
    assert_eq!(get.input_schema["properties"]["q"]["type"], "integer");
    let route = get
        .route(
            "https://example.com",
            json!({"id":"alice","q":7}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(route.url, "https://example.com/users/alice");
    assert_eq!(route.query["q"], 7);
    let delete = tools.iter().find(|t| t.method == "DELETE").unwrap();
    assert_eq!(delete.input_schema["required"], json!(["id", "q"]));
    let discovery = x402_treazury::pricing::PricingCache::default()
        .discover_observed(&cfg, &root, &tools, "http://127.0.0.1:1")
        .await
        .unwrap();
    assert_eq!(discovery.evidence.skipped_arguments, 1);
    assert_eq!(discovery.evidence.eligible, 0);
    assert!(discovery.prices.is_empty());
}

#[tokio::test]
async fn nullable_body_fields_preserve_presence_and_wire_names() {
    let root = json!({"openapi":"3.1.0","paths":{"/profile":{"patch":{
        "parameters":[{"name":"nickname","in":"query","schema":{"type":"string"}}],
        "requestBody":{"content":{"application/json":{"schema":{"type":"object",
            "properties":{"nickname":{"type":["string","null"]},"nested":{"type":"object"}},
            "required":["nickname"]
        }}}}
    }}}});
    let tools = build_tools(&Config::default(), &root, "route").unwrap();
    let tool = &tools[0];
    assert_eq!(tool.input_schema["required"], json!(["nickname_body"]));
    assert_eq!(
        tool.input_schema["properties"]["nickname_body"]["type"],
        json!(["string", "null"])
    );
    let args = json!({"nickname":null,"nickname_body":null,"nested":{"value":null,"items":[null]},"extra":null});
    let routed = tool
        .route("https://example.com", args.as_object().unwrap())
        .unwrap();
    assert!(routed.query.is_empty());
    let expected = json!({"nickname":null,"nested":{"value":null,"items":[null]},"extra":null});
    assert_eq!(routed.body, Some(expected.clone()));
    assert_eq!(
        tool.route("https://example.com", &Default::default())
            .unwrap()
            .body,
        Some(json!({}))
    );
    assert!(
        path_tools()[0]
            .route(
                "https://example.com",
                json!({"id":null}).as_object().unwrap()
            )
            .is_err()
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(
                |axum::Json(value): axum::Json<serde_json::Value>| async move { axum::Json(value) },
            ),
        )
        .await
        .unwrap()
    });
    let result = PaidClient::unsigned()
        .execute(tool.route(&base, args.as_object().unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        expected
    );
    task.abort();
}

#[tokio::test]
async fn dot_arguments_are_rejected_before_provider_io() {
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app = Router::new().fallback(move |request: Request| {
        seen.fetch_add(1, Ordering::SeqCst);
        async move { request.uri().path().to_owned() }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/gateway", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tools = path_tools();
    let name = tools[0].name.clone();
    let payer = Payer::new(
        &format!("{:064x}", 1),
        SpendPolicy::dollars("0.01").unwrap(),
    )
    .unwrap();
    let server = Server::new(tools, PaidClient::new(payer), base, None);
    for id in [".", ".."] {
        let error = server
            .invoke(&name, json!({"id":id}).as_object().unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("dot path parameter"));
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "neither unsigned nor signed requests may escape"
    );
    for (id, encoded) in [
        ("alice", "alice"),
        ("a.b", "a.b"),
        ("...", "..."),
        ("%2e%2e", "%252e%252e"),
        ("../admin", "..%2Fadmin"),
        ("a?b#c", "a%3Fb%23c"),
    ] {
        let result = server
            .invoke(&name, json!({"id":id}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(result, format!("/gateway/users/{encoded}/settings"));
    }
    assert_eq!(requests.load(Ordering::SeqCst), 6);
    task.abort();
}
