#[path = "support/tls.rs"]
mod tls;
use axum::{Router, extract::Request};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use x402_treazury::network;
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

#[test]
fn inherited_parameters_preserve_required_arguments_when_routing() {
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
}

#[tokio::test]
async fn nullable_body_fields_preserve_presence_and_wire_names() {
    tls::install_trust();
    let root = json!({"openapi":"3.1.0","paths":{"/profile":{"patch":{
        "parameters":[{"name":"nickname","in":"query","schema":{"type":"string"}}],
        "requestBody":{"content":{"application/json":{"schema":{"type":"object",
            "properties":{"nickname":{"type":["string","null"]},"nested":{"type":"object"}},
            "required":["nickname"]
        }}}}
    }}}});
    let tools = build_tools(&Config::default(), &root, "route").unwrap();
    let tool = &tools[0];
    assert!(tool.input_schema.get("required").is_none());
    assert_eq!(
        tool.input_schema["properties"]["body"]["required"],
        json!(["nickname"])
    );
    assert_eq!(
        tool.input_schema["properties"]["body"]["properties"]["nickname"]["type"],
        json!(["string", "null"])
    );
    let args = json!({"nickname":null,"body":{"nickname":null,"nested":{"value":null,"items":[null]},"extra":null}});
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
        None
    );
    assert!(
        path_tools()[0]
            .route(
                "https://example.com",
                json!({"id":null}).as_object().unwrap()
            )
            .is_err()
    );
    tls::install_trust();
    let (address, task) = tls::serve(Router::new().fallback(
        |axum::Json(value): axum::Json<serde_json::Value>| async move { axum::Json(value) },
    ))
    .await;
    let base = format!("https://{address}");
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
    tls::install_trust();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let app = Router::new().fallback(move |request: Request| {
        seen.fetch_add(1, Ordering::SeqCst);
        async move { request.uri().path().to_owned() }
    });
    tls::install_trust();
    let (address, task) = tls::serve(app).await;
    let base = format!("https://{address}/gateway");
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

#[tokio::test]
async fn whole_json_media_aliases_have_exact_wire_bytes_and_route_failures_have_no_io() {
    use axum::body::to_bytes;
    use std::sync::Mutex;
    tls::install_trust();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let seen = captured.clone();
    let (address, task) = tls::serve(Router::new().fallback(move |request: Request| {
        let seen = seen.clone();
        async move {
            let method = request.method().to_string();
            let uri = request
                .uri()
                .path_and_query()
                .expect("provider request path and query")
                .as_str()
                .to_owned();
            let media = request.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned();
            let header = request.headers()["x-mode"].to_str().unwrap().to_owned();
            let bytes = to_bytes(request.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec();
            seen.lock()
                .unwrap()
                .push((method, uri, media, header, bytes));
            "ok"
        }
    }))
    .await;
    let document = json!({"openapi":"3.1.0","paths":{
        "/items/{id}":{"patch":{"parameters":[
            {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
            {"name":"id","in":"query","schema":{"type":"integer"}},
            {"name":"X-Mode","in":"header","schema":{"type":"string"}}
        ],"requestBody":{"required":true,"content":{"application/json-patch+json; profile=\"https://example.com/profile\"":{"schema":{"type":"array","items":{"type":"object"}}}}}}},
        "/unsupported":{"post":{"parameters":[{"name":"session","in":"cookie"}]}}
    }});
    let built =
        x402_treazury::catalog::build_catalog(&Config::default(), &document, "wire").unwrap();
    assert_eq!(built.issues.len(), 1);
    assert_eq!(built.tools.len(), 1);
    let tools = serde_json::from_value(serde_json::to_value(built.tools).unwrap()).unwrap();
    let server = Server::new(
        tools,
        PaidClient::unsigned(),
        format!("https://{address}"),
        None,
    );
    let name = server.catalog.read().views["default"][0].tool.name.clone();
    for args in [
        json!({"id":"a/b","X-Mode":"fast"}),
        json!({"id":"a/b","X-Mode":"fast","body":[],"extra":"forbidden"}),
        json!({"id":"a/b","id_query":{"nested":1},"X-Mode":"fast","body":[]}),
        json!({"id":{"nested":1},"X-Mode":"fast","body":[]}),
        json!({"id":["a","b"],"X-Mode":"fast","body":[]}),
        json!({"id":"a/b","id_query":[[1]],"X-Mode":"fast","body":[]}),
    ] {
        assert!(
            server
                .invoke(&name, args.as_object().unwrap())
                .await
                .is_err()
        );
    }
    assert!(
        server
            .invoke("wire_unsupported", &Default::default())
            .await
            .is_err()
    );
    assert!(
        captured.lock().unwrap().is_empty(),
        "route/unsupported failures must precede all unsigned or paid I/O"
    );
    let body = json!([{"op":"remove","path":"/old"}]);
    let result = server
        .invoke(
            &name,
            json!({"id":"a/b","id_query":7,"X-Mode":"fast","body":body})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result, "ok");
    let records = captured.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0],
        (
            "PATCH".into(),
            "/items/a%2Fb?id=7".into(),
            "application/json-patch+json; profile=\"https://example.com/profile\"".into(),
            "fast".into(),
            serde_json::to_vec(&body).unwrap()
        )
    );
    drop(records);
    task.abort();
}
