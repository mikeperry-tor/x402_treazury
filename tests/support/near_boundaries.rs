use super::*;

#[test]
fn asset_and_quote_boundaries_fail_closed() {
    let tokens: Value = serde_json::from_str(include_str!("../fixtures/near/tokens.json")).unwrap();
    for index in 0..2 {
        let mut missing = tokens.clone();
        missing.as_array_mut().unwrap().remove(index);
        assert!(validate_assets(&missing).is_err());
        let mut duplicate = tokens.clone();
        duplicate
            .as_array_mut()
            .unwrap()
            .push(tokens[index].clone());
        assert!(validate_assets(&duplicate).is_err());
        let mut decimals = tokens.clone();
        decimals[index]["decimals"] = json!(99);
        assert!(validate_assets(&decimals).is_err());
    }
    let (request, response, mut limits) = fixture(false);
    limits.max_input = 110000;
    assert!(validate_quote(request.clone(), response.clone(), &limits, 2_000_000_700).is_ok());
    assert!(validate_quote(request.clone(), response.clone(), &limits, 2_000_000_701).is_err());
    limits.max_input = 109999;
    assert!(validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000).is_err());
    limits.max_input = u64::MAX;
    limits.max_fee = u64::MAX;
    assert!(validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000).is_err());
    let (_, _, limits) = fixture(false);
    for text in [
        "5.250001",
        "18446744073709551615.999999999999999999",
        "5.0000000000000000001",
        "NaN",
        "-1",
    ] {
        let mut bad = response.clone();
        bad["quote"]["amountInUsd"] = json!(text);
        assert!(
            validate_quote(request.clone(), bad, &limits, 2_000_000_000).is_err(),
            "{text}"
        );
    }
    for (fee, pass) in [(500u64, true), (501, false), (u64::MAX, false)] {
        let mut changed = response.clone();
        changed["quoteRequest"]["appFees"] = json!([{"recipient":"5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd","fee":fee}]);
        assert_eq!(
            validate_quote(request.clone(), changed, &limits, 2_000_000_000).is_ok(),
            pass
        );
    }
}

#[tokio::test]
async fn status_binds_quote_and_credentials_stay_on_the_original_request() {
    use axum::{
        Json, Router,
        extract::State,
        http::HeaderMap,
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};
    let (request, response, limits) = fixture(false);
    let quote = validate_quote(request.clone(), response.clone(), &limits, 2_000_000_000).unwrap();
    let state = Arc::new(Mutex::new((
        json!({"status":"SUCCESS","quoteResponse":response}),
        vec![],
    )));
    type Responses = Arc<Mutex<(Value, Vec<HeaderMap>)>>;
    let handler = |State(state): State<Responses>, headers: HeaderMap| async move {
        let mut state = state.lock().unwrap();
        state.1.push(headers);
        Json(state.0.clone())
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v0/status", get(handler))
        .route("/v0/quote", post(handler))
        .route("/v0/tokens", get(handler))
        .with_state(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for authenticated in [false, true] {
        let client = NearClient::at(
            &url,
            authenticated.then_some("fixture-key"),
            authenticated.then_some("fixture-session"),
        )
        .unwrap();
        assert!(matches!(
            client.status(&quote).await.unwrap(),
            SwapStatus::Success
        ));
        state.lock().unwrap().0 = response.clone();
        client
            .quote(request.clone(), &limits, 2_000_000_000)
            .await
            .unwrap();
        state.lock().unwrap().0 =
            serde_json::from_str(include_str!("../fixtures/near/tokens.json")).unwrap();
        client.assets().await.unwrap();
        let headers = std::mem::take(&mut state.lock().unwrap().1);
        assert_eq!(headers.len(), 3);
        for header in headers {
            assert_eq!(
                header.get("x-api-key").map(|v| v.to_str().unwrap()),
                authenticated.then_some("fixture-key")
            );
            assert_eq!(
                header.get("authorization").map(|v| v.to_str().unwrap()),
                authenticated.then_some("Bearer fixture-session")
            );
        }
        state.lock().unwrap().0 = json!({"status":"SUCCESS","quoteResponse":response});
    }
    let client = NearClient::at(&url, None, None).unwrap();
    for pointer in [
        "/quoteResponse/quote/depositAddress",
        "/quoteResponse/quoteRequest/recipient",
        "/quoteResponse/quoteRequest/refundTo",
        "/quoteResponse/quoteRequest/amount",
    ] {
        let mut bad = json!({"status":"SUCCESS","quoteResponse":response});
        *bad.pointer_mut(pointer).unwrap() = Value::Null;
        state.lock().unwrap().0 = bad;
        assert!(client.status(&quote).await.is_err(), "{pointer}");
    }
    state.lock().unwrap().0 = json!({"status":"FUTURE_STATUS","quoteResponse":response});
    assert!(matches!(
        client.status(&quote).await.unwrap(),
        SwapStatus::Unknown
    ));
    server.abort();
}

#[tokio::test]
async fn response_reader_checks_exact_limit_partial_body_and_deadline() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for case in ["limit", "oversize", "malformed", "partial", "timeout"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.unwrap() > 0);
            if case == "timeout" {
                tokio::time::sleep(Duration::from_millis(200)).await;
                return;
            }
            let body = match case {
                "limit" => format!("null{}", " ".repeat(1_999_996)),
                "oversize" => format!("null{}", " ".repeat(1_999_997)),
                "malformed" => "not json".into(),
                _ => "{}".into(),
            };
            let length = body.len() + usize::from(case == "partial") * 100;
            if socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .is_ok()
            {
                let _ = socket.write_all(body.as_bytes()).await;
            }
        });
        let client = NearClient::at(&url, None, None).unwrap();
        let http = crate::network::discovery(
            &url,
            Duration::from_millis(if case == "timeout" { 50 } else { 2000 }),
        )
        .unwrap();
        let result = client.response(http.get(&url)).await;
        if case == "limit" {
            assert_eq!(result.unwrap(), Value::Null);
        } else {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(match case {
                    "oversize" => "near_response_too_large",
                    "malformed" => "invalid NEAR JSON",
                    "partial" => "near_response_failed",
                    _ => "near_unavailable",
                }),
                "{case}: {error}"
            );
        }
        server.await.unwrap();
    }
}
