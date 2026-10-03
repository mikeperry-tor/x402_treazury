//! Authenticated parallel agent mutations, with overlap forced just before commit.
use super::*;

fn grants() -> BTreeMap<String, ListenerConfig> {
    let mut ls = listeners(true);
    ls.get_mut("reader").unwrap().source_management = ls["writer"].source_management.clone();
    ls.get_mut("reader").unwrap().tags = vec!["read".into()];
    for id in ["writer", "reader"] {
        ls.get_mut(id)
            .unwrap()
            .source_management
            .as_mut()
            .unwrap()
            .max_owned_sources = 2;
    }
    ls
}
fn args(name: &str, key: &str) -> Value {
    json!({"candidate":candidate(name,"persistent","process"),"idempotency_key":key})
}
fn parsed(response: &Value) -> Value {
    assert_ne!(response["isError"], true, "{response}");
    serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap()
}
async fn wave(
    m: &Arc<Manager>,
    endpoints: &BTreeMap<String, String>,
    requests: Vec<(&str, Value)>,
) -> Vec<(String, Value, Value)> {
    let barrier = Arc::new(tokio::sync::Barrier::new(requests.len() + 1));
    *m.commit_barrier.lock().unwrap() = Some(barrier.clone());
    let mut calls = tokio::task::JoinSet::new();
    // Preview beforehand: this wave targets commit contention, independently of
    // the separately bounded importer. Identical retries reuse identical args.
    let mut previews = BTreeMap::new();
    for (owner, mut args) in requests {
        let key = (owner.to_owned(), args["candidate"].to_string());
        if !previews.contains_key(&key) {
            let preview = m
                .invoke(
                    owner,
                    "treazure_source_preview",
                    json!({"candidate":args["candidate"]}),
                )
                .await
                .unwrap();
            previews.insert(key.clone(), preview["preview_id"].clone());
        }
        args["preview_id"] = previews[&key].clone();
        let owner = owner.to_owned();
        let url = endpoints[&owner].clone();
        calls.spawn(async move {
            let response = rpc(
                &url,
                &format!("{owner}-token"),
                "tools/call",
                json!({"name":"treazure_source_add","arguments":args}),
            )
            .await;
            (owner, args, response)
        });
    }
    tokio::time::timeout(Duration::from_secs(8), barrier.wait())
        .await
        .unwrap();
    let mut results = vec![];
    tokio::time::timeout(Duration::from_secs(8), async {
        while let Some(result) = calls.join_next().await {
            results.push(result.unwrap());
        }
    })
    .await
    .unwrap();
    *m.commit_barrier.lock().unwrap() = None;
    results
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_additions_are_atomic_idempotent_owner_scoped_and_durable_under_contention() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("sources.sqlite");
    let mut p = policy(Some(file));
    p.max_sources = 3;
    let m = manager_with(p.clone(), grants()).await;
    seed(&m).await;
    let mut endpoints = BTreeMap::new();
    let mut tasks = vec![];
    let shutdown = tokio_util::sync::CancellationToken::new();
    for owner in ["writer", "reader", "hidden"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        endpoints.insert(
            owner.to_owned(),
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let app = crate::server::http_app(server(&m, owner), format!("{owner}-token"));
        let stop = shutdown.clone();
        tasks.push(tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .unwrap();
        }));
    }
    let original = m.catalog.read();
    let observer_stop = tokio_util::sync::CancellationToken::new();
    let observer = tokio::spawn({
        let m = m.clone();
        let stop = observer_stop.clone();
        async move {
            let mut samples = 0;
            while !stop.is_cancelled() {
                let s = m.catalog.read();
                assert_eq!(
                    s.views["writer"].len(),
                    2 * s.views["reader"].len(),
                    "partial cross-listener publication"
                );
                assert!(s.views["hidden"].is_empty());
                assert!(s.views["reader"].len() <= 3);
                let names: std::collections::BTreeSet<_> =
                    s.views["writer"].iter().map(|b| &b.tool.name).collect();
                assert_eq!(names.len(), s.views["writer"].len());
                samples += 1;
                tokio::task::yield_now().await;
            }
            samples
        }
    });
    // All twelve requests pass the initial replay check before any can commit.
    let duplicate = args("same", "same-key");
    let responses = wave(
        &m,
        &endpoints,
        (0..12).map(|_| ("writer", duplicate.clone())).collect(),
    )
    .await;
    let first_args = responses[0].1.clone();
    let first = parsed(&responses[0].2);
    for (_, _, response) in responses {
        assert_eq!(parsed(&response), first);
    }
    assert_eq!(m.catalog.read().generation, 1);
    // A second owner's identical key/name is independent, even across shared targets.
    let second = parsed(
        &rpc(
            &endpoints["reader"],
            "reader-token",
            "tools/call",
            json!({"name":"treazure_source_add","arguments":duplicate}),
        )
        .await,
    );
    assert_ne!(second["source_id"], first["source_id"]);
    let mut accepted = vec![
        ("writer".to_owned(), first_args, first),
        ("reader".to_owned(), duplicate.clone(), second),
    ];
    let requests = (0..12)
        .map(|i| {
            (
                if i % 2 == 0 { "writer" } else { "reader" },
                args(&format!("candidate{i}"), &format!("key{i}")),
            )
        })
        .collect();
    let responses = wave(&m, &endpoints, requests).await;
    let mut failed = 0;
    for (owner, args, response) in responses {
        if response["isError"] == true {
            assert!(response.to_string().contains("quota"), "{response}");
            failed += 1;
        } else {
            accepted.push((owner, args, parsed(&response)));
        }
    }
    assert_eq!(failed, 11);
    assert_eq!(accepted.len(), 3);
    assert_eq!(m.catalog.read().generation, 3);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 3);
    // Wrong endpoint authority cannot take ownership or add a hidden source.
    for (owner, _, receipt) in &accepted {
        let other = if owner == "writer" {
            "reader"
        } else {
            "writer"
        };
        let response = rpc(&endpoints[other], &format!("{other}-token"), "tools/call", json!({"name":"treazure_source_remove","arguments":{"source_id":receipt["source_id"],"expected_revision":1,"idempotency_key":"foreign-remove"}})).await;
        assert_eq!(response["isError"], true);
    }
    let denied = rpc(
        &endpoints["hidden"],
        "hidden-token",
        "tools/call",
        json!({"name":"treazure_source_add","arguments":args("forbidden","forbidden")}),
    )
    .await;
    assert_eq!(denied["isError"], true);
    assert_eq!(m.catalog.read().generation, 3);
    assert!(original.views.values().all(Vec::is_empty));
    observer_stop.cancel();
    assert!(observer.await.unwrap() > 0);
    // Aborting the accept loop does not join its pooled HTTP connection tasks.
    // Drain them before reopening the registry, as production shutdown does.
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        for task in tasks {
            task.await.unwrap();
        }
    })
    .await
    .expect("management listeners did not release their connections");
    // Listener completion can precede destruction of HTTP service task state.
    // Retain our owner until all others release theirs, then drop it synchronously
    // so registry reopening cannot race the final manager destructor.
    tokio::time::timeout(Duration::from_secs(5), async {
        while Arc::strong_count(&m) != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("HTTP service tasks still own the manager after listener shutdown");
    drop(m);
    let reopened = manager_with(p, grants()).await;
    assert_eq!(reopened.catalog.read().views["writer"].len(), 6);
    assert_eq!(reopened.catalog.read().views["reader"].len(), 3);
    assert_eq!(reopened.inner.lock().unwrap().state.records.len(), 3);
    for (owner, args, receipt) in accepted {
        assert_eq!(
            reopened
                .invoke(&owner, "treazure_source_add", args)
                .await
                .unwrap(),
            receipt
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_additions_enforce_listener_and_owner_quotas_without_partial_sources() {
    for tool_limit in [true, false] {
        let mut p = policy(None);
        p.max_tools_per_server = if tool_limit { 4 } else { 200 };
        let mut ls = listeners(false);
        ls.get_mut("writer")
            .unwrap()
            .source_management
            .as_mut()
            .unwrap()
            .max_owned_sources = if tool_limit { 8 } else { 2 };
        let m = manager_with(p, ls).await;
        seed(&m).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(9));
        *m.commit_barrier.lock().unwrap() = Some(barrier.clone());
        let mut calls = tokio::task::JoinSet::new();
        for i in 0..8 {
            let c = candidate(&format!("tool{i}"), "process", "process");
            let preview = m
                .invoke("writer", "treazure_source_preview", json!({"candidate":c}))
                .await
                .unwrap();
            let m = m.clone();
            calls.spawn(async move { m.invoke("writer", "treazure_source_add", json!({"candidate":c,"preview_id":preview["preview_id"],"idempotency_key":format!("tool{i}")})).await });
        }
        tokio::time::timeout(Duration::from_secs(8), barrier.wait())
            .await
            .unwrap();
        let mut successes = 0;
        while let Some(r) = calls.join_next().await {
            match r.unwrap() {
                Ok(_) => successes += 1,
                Err(e) => assert!(
                    e.to_string().contains(if tool_limit {
                        "server_tool_quota_exceeded"
                    } else {
                        "owner_source_quota_exceeded"
                    }),
                    "{e}"
                ),
            }
        }
        assert_eq!(successes, 2);
        let snapshot = m.catalog.read();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.views["writer"].len(), 4);
        assert_eq!(snapshot.views["reader"].len(), 4);
        assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_agent_imports_coalesce_and_busy_rejections_can_retry() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let (count, notify, permits) = (hits.clone(), entered.clone(), release.clone());
    let (vendor, vendor_task) = listen(axum::Router::new().route(
        "/spec",
        axum::routing::get(move || {
            let (count, notify, permits) = (count.clone(), notify.clone(), permits.clone());
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                notify.notify_one();
                let permit = permits.acquire().await.unwrap();
                permit.forget();
                axum::Json(spec())
            }
        }),
    ))
    .await;
    let m = manager_with(policy(None), grants()).await;
    *m.fixture_endpoint.lock().unwrap() = Some(format!("{vendor}/spec"));
    let mut endpoints = BTreeMap::new();
    let mut servers = vec![];
    for owner in ["writer", "reader"] {
        let (url, task) = listen(crate::server::http_app(
            server(&m, owner),
            format!("{owner}-token"),
        ))
        .await;
        endpoints.insert(owner, url);
        servers.push(task);
    }
    let process_args = |name: &str| json!({"candidate":candidate(name,"process","process"),"idempotency_key":name});
    let mut calls = vec![];
    for owner in ["writer", "reader"] {
        let url = endpoints[owner].clone();
        let args = process_args(owner);
        calls.push(tokio::spawn(async move {
            rpc(
                &url,
                &format!("{owner}-token"),
                "tools/call",
                json!({"name":"treazure_source_add","arguments":args}),
            )
            .await
        }));
        if owner == "writer" {
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .unwrap();
        }
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while m.owner_import["reader"].available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let retry = process_args("retry");
    for owner in ["writer", "reader"] {
        let response = rpc(
            &endpoints[owner],
            &format!("{owner}-token"),
            "tools/call",
            json!({"name":"treazure_source_add","arguments":retry}),
        )
        .await;
        assert_eq!(response["isError"], true);
        assert!(
            response.to_string().contains("owner_import_busy"),
            "{response}"
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(m.catalog.read().generation, 0);
    assert!(m.inner.lock().unwrap().state.records.is_empty());
    release.add_permits(1);
    for call in calls {
        parsed(
            &tokio::time::timeout(Duration::from_secs(5), call)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(m.catalog.read().generation, 2);
    let response = rpc(
        &endpoints["writer"],
        "writer-token",
        "tools/call",
        json!({"name":"treazure_source_add","arguments":retry}),
    )
    .await;
    parsed(&response);
    assert_eq!(m.catalog.read().generation, 3);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    for server in servers {
        server.abort();
        let _ = server.await;
    }
    vendor_task.abort();
    let _ = vendor_task.await;
}
