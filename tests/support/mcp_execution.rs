//! Dispatcher tests: explicit local bindings isolate MCP behavior from import policy.
use super::*;
use axum::{
    body::Bytes,
    extract::OriginalUri,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use rmcp::ServerHandler;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn direct_and_fallback_wire_routing_errors_and_unicode_limits() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0));
    let (c, md) = (captured.clone(), mode.clone());
    let (vendor, v) = listen(axum::Router::new().route(
        "/items/{id}",
        post(
            move |OriginalUri(uri): OriginalUri, headers: HeaderMap, body: Bytes| {
                let (c, md) = (c.clone(), md.clone());
                async move {
                    c.lock()
                        .unwrap()
                        .push((uri.to_string(), headers, body.to_vec()));
                    match md.load(Ordering::SeqCst) {
                        1 => (StatusCode::BAD_GATEWAY, "vendor failed").into_response(),
                        2 => (
                            StatusCode::PAYMENT_REQUIRED,
                            [("payment-required", "invalid-base64")],
                            "bad challenge",
                        )
                            .into_response(),
                        3 => {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            "late".into_response()
                        }
                        _ => "雪🙂é".into_response(),
                    }
                }
            },
        ),
    ))
    .await;
    let m = manager(None).await;
    let tools = crate::catalog::build_tools(&crate::catalog::Config::default(), &json!({"paths":{"/items/{id}":{"post":{
        "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
        {"name":"q","in":"query","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"flag","in":"query","schema":{"type":"boolean"}}],
        "requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"q":{"type":"string"}}}}}}
    }}}}), "wire").unwrap();
    let name = tools[0].name.clone();
    let bound = crate::catalog_state::bind(vec![(
        tools[0].clone(),
        payer().with_timeout(Duration::from_millis(150)),
        vendor,
    )]);
    m.catalog.publish(CatalogSnapshot {
        generation: 1,
        views: BTreeMap::from([("writer".into(), bound)]),
    });
    let s = server(&m, "writer");
    assert!(s.get_tool(&name).is_some());
    assert!(s.get_tool("x402_treazury_tool_call").is_some());
    assert!(s.get_tool("missing").is_none());
    assert!(server(&m, "hidden").get_tool(&name).is_none());
    assert!(
        server(&m, "hidden")
            .get_tool("x402_treazury_source_add")
            .is_none()
    );
    let reference = m.tool_reference("writer", &m.catalog.read().views["writer"][0]);
    for limit in [0, 2, 3, 4] {
        let mut s = s.clone();
        s.max_response_chars = Some(limit);
        let (base, task) = listen(crate::server::http_app(s, "test-token".into())).await;
        for fallback in [false, true] {
            let args = json!({"id":"a/b?雪","q":["x y","雪"],"flag":true,"q_body":"body🙂"});
            let result = if fallback {
                call(
                    &base,
                    "x402_treazury_tool_call",
                    json!({"tool_ref":reference,"arguments":args}),
                )
                .await
            } else {
                call(&base, &name, args).await
            };
            let expected = if limit < 3 {
                format!(
                    "{}\n[truncated by --max-response-chars]",
                    "雪🙂é".chars().take(limit).collect::<String>()
                )
            } else {
                "雪🙂é".into()
            };
            assert_eq!(result["content"][0]["text"], expected);
            assert_ne!(result["isError"], true);
        }
        let structured = call(&base, "x402_treazury_tools_search", json!({})).await;
        assert_ne!(structured["isError"], true, "{structured}");
        assert_eq!(
            serde_json::from_str::<Value>(structured["content"][0]["text"].as_str().unwrap())
                .unwrap(),
            structured["structuredContent"]
        );
        task.abort();
    }
    {
        let captures = captured.lock().unwrap();
        assert_eq!(captures.len(), 8);
        for (uri, headers, body) in captures.iter() {
            assert_eq!(uri.split('?').next().unwrap(), "/items/a%2Fb%3F%E9%9B%AA");
            let parsed = reqwest::Url::parse(&format!("http://localhost{uri}")).unwrap();
            let pairs: Vec<_> = parsed
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            assert_eq!(
                pairs,
                vec![
                    ("flag".into(), "true".into()),
                    ("q".into(), "x y".into()),
                    ("q".into(), "雪".into())
                ]
            );
            assert_eq!(headers["content-type"], "application/json");
            assert!(!headers.contains_key("payment-signature"));
            assert_eq!(
                serde_json::from_slice::<Value>(body).unwrap(),
                json!({"q":"body🙂"})
            );
        }
    }
    let (base, task) = listen(crate::server::http_app(s, "test-token".into())).await;
    for (tool, args) in [
        (name.as_str(), json!({})),
        ("missing", json!({})),
        ("x402_treazury_tool_call", json!({})),
        ("x402_treazury_source_add", json!({})),
    ] {
        assert_eq!(call(&base, tool, args).await["isError"], true);
    }
    assert_eq!(captured.lock().unwrap().len(), 8);
    for fail in [1, 2, 3] {
        mode.store(fail, Ordering::SeqCst);
        for fallback in [false, true] {
            let before = captured.lock().unwrap().len();
            let result = if fallback {
                call(
                    &base,
                    "x402_treazury_tool_call",
                    json!({"tool_ref":reference,"arguments":{"id":"error"}}),
                )
                .await
            } else {
                call(&base, &name, json!({"id":"error"})).await
            };
            assert_eq!(result["isError"], true, "{result}");
            assert_eq!(captured.lock().unwrap().len(), before + 1);
        }
    }
    task.abort();
    v.abort();
}

#[tokio::test]
async fn registered_source_mutation_preserves_captured_paid_invocations() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    for remove in [false, true] {
        for fallback in [false, true] {
            let m = manager(None).await;
            let _added = add(&m, "captured", "process", "process").await;
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let counts = Arc::new(AtomicUsize::new(0));
            let (e, r, c) = (entered.clone(), release.clone(), counts.clone());
            let (vendor, v) = listen(axum::Router::new().route("/read", axum::routing::get(move |headers: HeaderMap| {
                let (e,r,c)=(e.clone(),r.clone(),c.clone());
                async move {
                    c.fetch_add(1,Ordering::SeqCst);
                    if let Some(signature)=headers.get("payment-signature") {
                        let payload: Value=serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
                        let signer=crate::test_signatures::recover_exact(&payload);
                        assert_eq!(signer.to_string().to_lowercase(),"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
                        e.notify_one(); r.notified().await;
                        return "original route and payer".into_response();
                    }
                    let challenge=json!({"x402Version":2,"resource":{"url":"https://api.example.com/read","description":"read","mimeType":"text/plain"},"accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"5000","payTo":"0x0000000000000000000000000000000000000003","maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}}]});
                    (StatusCode::PAYMENT_REQUIRED,[("payment-required",STANDARD.encode(serde_json::to_vec(&challenge).unwrap()))]).into_response()
                }
            }))).await;
            let mut snapshot = (*m.catalog.read()).clone();
            let name = snapshot.views["writer"]
                .iter()
                .find(|b| b.tool.path == "/read")
                .unwrap()
                .tool
                .name
                .clone();
            for view in snapshot.views.values_mut() {
                for bound in view {
                    bound.base = vendor.clone();
                    bound.client = payer();
                }
            }
            m.catalog.publish(snapshot);
            let s = server(&m, "writer");
            let (base, task) =
                listen(crate::server::http_app(s.clone(), "test-token".into())).await;
            let reference = m.tool_reference(
                "writer",
                crate::catalog_state::find(&m.catalog.read(), "writer", &name).unwrap(),
            );
            let (url, n, pending_reference) = (base.clone(), name.clone(), reference.clone());
            let pending = tokio::spawn(async move {
                if fallback {
                    call(
                        &url,
                        "x402_treazury_tool_call",
                        json!({"tool_ref":pending_reference,"arguments":{}}),
                    )
                    .await
                } else {
                    call(&url, &n, json!({})).await
                }
            });
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .unwrap();
            // Publication changes do not replace bindings captured by in-flight calls.
            let mut snapshot = (*m.catalog.read()).clone();
            snapshot.generation += 1;
            let view = snapshot.views.get_mut("writer").unwrap();
            if remove {
                view.clear();
            } else {
                view.retain(|b| b.tool.path != "/read");
                for b in view {
                    b.source.as_mut().unwrap().1 += 1;
                }
            }
            m.catalog.publish(snapshot);
            assert!(s.get_tool(&name).is_none());
            assert_eq!(call(&base, &name, json!({})).await["isError"], true);
            assert_eq!(
                call(
                    &base,
                    "x402_treazury_tool_call",
                    json!({"tool_ref":reference,"arguments":{}})
                )
                .await["isError"],
                true
            );
            assert_eq!(counts.load(Ordering::SeqCst), 2);
            release.notify_one();
            let result = pending.await.unwrap();
            assert_ne!(result["isError"], true, "{result}");
            assert_eq!(result["content"][0]["text"], "original route and payer");
            assert_eq!(counts.load(Ordering::SeqCst), 2);
            task.abort();
            v.abort();
        }
    }
}
