//! Isolated process-wide bound across real authenticated MCP listeners and payers.
use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use x402_treazury::{
    catalog::{Config, build_tools},
    mcp_wire::McpResponse,
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_capacity_survives_disconnect_signing_and_body_reads_across_listeners() {
    let unsigned = Arc::new(AtomicUsize::new(0));
    let signed = Arc::new(AtomicUsize::new(0));
    let reading = Arc::new(AtomicUsize::new(0));
    let headers = Arc::new(Semaphore::new(0));
    let bodies = Arc::new(Semaphore::new(0));
    let vendor = Router::new().route("/{mode}", get({
        let (unsigned, signed, reading, headers, bodies) = (unsigned.clone(), signed.clone(), reading.clone(), headers.clone(), bodies.clone());
        move |path: axum::extract::Path<String>, request: HeaderMap| {
            let (unsigned, signed, reading, headers, bodies) = (unsigned.clone(), signed.clone(), reading.clone(), headers.clone(), bodies.clone());
            async move {
                if request.contains_key("payment-signature") { signed.fetch_add(1, Ordering::SeqCst); }
                else {
                    unsigned.fetch_add(1, Ordering::SeqCst);
                    headers.acquire().await.unwrap().forget();
                    if path.0 == "paid" {
                        let challenge = json!({"x402Version":2,"resource":{"url":"http://localhost/paid"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":x402_treazury::payment::USDC,"amount":"1","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                        return (StatusCode::PAYMENT_REQUIRED, [("payment-required", STANDARD.encode(challenge.to_string()))], "challenge").into_response();
                    }
                }
                reading.fetch_add(1, Ordering::SeqCst);
                Body::from_stream(futures_util::stream::once(async move {
                    bodies.acquire().await.unwrap().forget();
                    Ok::<_, std::io::Error>(Bytes::from_static(b"done"))
                })).into_response()
            }
        }
    }));
    let (origin, vendor) = serve(vendor).await;
    let mut listeners = Vec::new();
    let mut owners = Vec::new();
    let mut clients = Vec::new();
    let mut urls = Vec::new();
    for mode in ["free", "paid"] {
        let client = if mode == "free" {
            PaidClient::unsigned()
        } else {
            PaidClient::new(
                Payer::new(&format!("{:064x}", 1), SpendPolicy::dollars("1").unwrap()).unwrap(),
            )
        };
        let tools = build_tools(
            &Config::default(),
            &json!({"paths":{format!("/{mode}"):{"get":{"operationId":"read"}}}}),
            "test",
        )
        .unwrap();
        let name = tools[0].name.clone();
        let server = Server::new(tools, client.clone(), origin.clone(), None);
        owners.push(server.work.clone());
        clients.push(client);
        let (url, task) = serve(http_app(server, "fixture-token".into())).await;
        urls.push((url, name));
        listeners.push(task);
    }
    let http = x402_treazury::network::discovery(&urls[0].0, Duration::from_secs(30)).unwrap();
    let request = |n: usize| {
        let (url, name) = &urls[n % 2];
        http.post(format!("{url}/mcp")).bearer_auth("fixture-token")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":n,"method":"tools/call","params":{"name":name,"arguments":{}}}))
    };
    let mut waiters = Vec::new();
    for n in 0..128 {
        let request = request(n);
        waiters.push(tokio::spawn(async move {
            request
                .send()
                .await
                .unwrap()
                .mcp_json::<Value>()
                .await
                .unwrap()
        }));
    }
    until(|| unsigned.load(Ordering::SeqCst) == 128).await;
    for waiter in waiters.iter().take(4) {
        waiter.abort();
    }
    for phase in 0..2 {
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            request(128 + phase)
                .send()
                .await
                .unwrap()
                .mcp_json::<Value>()
                .await
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            response
                .to_string()
                .contains("managed payment queue capacity unavailable"),
            "{response}"
        );
        assert_eq!(unsigned.load(Ordering::SeqCst), 128);
        // The same bound covers direct library calls sharing this process.
        let error = clients[0]
            .execute(x402_treazury::catalog::RoutedRequest {
                headers: Default::default(),
                method: "GET".into(),
                url: format!("{origin}/free"),
                query: Default::default(),
                body: None,
            })
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("managed payment queue capacity unavailable")
        );
        if phase == 0 {
            headers.add_permits(128);
            until(|| reading.load(Ordering::SeqCst) == 128).await;
            assert_eq!(signed.load(Ordering::SeqCst), 64);
        }
    }
    bodies.add_permits(128);
    for (n, waiter) in waiters.into_iter().enumerate() {
        match waiter.await {
            Err(e) => assert!(n < 4 && e.is_cancelled()),
            Ok(value) => assert_eq!(value["result"]["isError"], false, "{value}"),
        }
    }
    // Draining tracks cancelled HTTP waiters too; only completion releases capacity.
    for owner in &owners {
        owner.drain().await;
    }
    headers.add_permits(1);
    bodies.add_permits(1);
    assert_eq!(
        clients[0]
            .execute(x402_treazury::catalog::RoutedRequest {
                headers: Default::default(),
                method: "GET".into(),
                url: format!("{origin}/free"),
                query: Default::default(),
                body: None,
            })
            .await
            .unwrap(),
        "done"
    );
    for client in clients {
        client.drain().await;
    }
    assert_eq!(unsigned.load(Ordering::SeqCst), 129);
    assert_eq!(signed.load(Ordering::SeqCst), 64);
    for task in listeners {
        task.abort();
    }
    vendor.abort();
}
