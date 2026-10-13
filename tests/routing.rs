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
    let server = Server::new(tools, PaidClient::new(payer), base, None, None);
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
