//! One published generation and retained same-registration ownership before I/O.
use super::*;
use std::sync::{
    Condvar,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn wait(&self) {
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*open {
            open = self
                .changed
                .wait(open)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
    fn release(&self) {
        *self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        self.changed.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pending_addition_owns_registration_across_eviction_payer_change_and_cancellation() {
    if crate::test_log_capture::run_in_child(
        "discovery::tests::publication_read_tests::pending_addition_owns_registration_across_eviction_payer_change_and_cancellation",
    ) {
        return;
    }
    let m = manager(None).await;
    struct ReleaseCommit(Arc<tokio::sync::Semaphore>);
    impl Drop for ReleaseCommit {
        fn drop(&mut self) {
            self.0.add_permits(1);
        }
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let release = ReleaseCommit(gate.clone());
    let (entered, entry) = tokio::sync::oneshot::channel();
    *m.commit_barrier.lock().unwrap() = Some((entered, gate));
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let app = axum::Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { axum::Json(spec()) }
    });
    let (endpoint, server) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(endpoint);
    let specification = "https://api.example.com/held.json";
    let original_key = fetch_key(&m, specification);
    let args = json!({"spec_url":specification,"name":"held"});
    let first_owner = m.clone();
    let first_args = args.clone();
    let first = tokio::spawn(async move {
        first_owner
            .invoke("writer", "x402_treazury_providers_add_remote", first_args)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entry)
        .await
        .unwrap()
        .unwrap();
    // The canceled waiter leaves its accepted add alive through publication.
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    m.source_client("writer").unwrap().replace_payer(
        Payer::new(
            &format!("{:064x}", 2),
            SpendPolicy::dollars("0.01").unwrap(),
        )
        .unwrap(),
    );
    assert_ne!(original_key, fetch_key(&m, specification));
    for index in 0..4 {
        m.document("writer", &format!("https://api.example.com/churn{index}"))
            .await
            .unwrap();
    }
    assert!(!m.fetches.lock().unwrap().contains_key(&original_key));
    // Other identities can acquire, build and publish while the first is held.
    let other = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":"https://api.example.com/other.json","name":"other"}),
        )
        .await
        .unwrap();
    assert_eq!(other["name"], "other");
    assert!(
        m.existing_registration(
            "writer",
            import::RegistrationKey::Specification(specification)
        )
        .is_none()
    );
    // Poll the actual add body, so Pending proves it reached its own wait rather
    // than merely scheduling an owned task that has not run yet.
    let mut second = Box::pin(m.add_source_owned("writer", args));
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    let before_release = requests.load(Ordering::SeqCst);
    drop(release);
    let added = second.await.unwrap();
    server.abort();
    let _ = server.await;
    assert_eq!(before_release, 6);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        6,
        "the duplicate add must not reacquire after cache eviction or payer replacement"
    );
    assert_eq!(added["name"], "held");
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
    assert_eq!(m.catalog.read().generation, 2);
    assert!(
        m.additions
            .lock()
            .unwrap()
            .values()
            .all(|gate| gate.strong_count() == 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_duplicate_miss_waits_before_reacquisition_after_completed_cache_eviction() {
    if crate::test_log_capture::run_in_child(
        "discovery::tests::publication_read_tests::pending_duplicate_miss_waits_before_reacquisition_after_completed_cache_eviction",
    ) {
        return;
    }
    let m = manager(None).await;
    let old = register_spec(&m, "writer", "api-example-com", "first.json", spec())
        .await
        .unwrap();
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let held = gate.clone();
    let (entered, entry) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    assert!(
        import::BUILD_OBSERVER
            .set(Box::new(move |candidate| {
                if candidate.name == "api-example-com-2" {
                    if let Some(entered) = entered.lock().unwrap().take() {
                        let _ = entered.send(());
                    }
                    held.wait();
                }
            }))
            .is_ok()
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let app = axum::Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { axum::Json(spec()) }
    });
    let (endpoint, server) = listen(app).await;
    *m.fixture_endpoint.lock().unwrap() = Some(endpoint);
    let specification = "https://api.example.com/second.json?contract=A";
    let args = json!({"spec_url":specification});
    let first_owner = m.clone();
    let first_args = args.clone();
    let first = tokio::spawn(async move {
        first_owner
            .invoke("writer", "x402_treazury_providers_add_remote", first_args)
            .await
    });
    let held_commit = tokio::time::timeout(Duration::from_secs(5), entry).await;
    // If the owner failed before the callback, cleanup still drains it before asserts.
    let evicted_complete = if matches!(held_commit, Ok(Ok(()))) {
        m.fetches
            .lock()
            .unwrap()
            .remove(&fetch_key(&m, specification))
            .is_some_and(|cell| cell.get().is_some())
    } else {
        false
    };
    let previous = m.catalog.read();
    let (rechecking, recheck) = tokio::sync::oneshot::channel();
    *m.recheck_started.lock().unwrap() = Some(rechecking);
    let second_owner = m.clone();
    let second = tokio::spawn(async move {
        second_owner
            .invoke("writer", "x402_treazury_providers_add_remote", args)
            .await
    });
    let actual_miss = tokio::time::timeout(Duration::from_secs(5), recheck).await;
    let reader_owner = m.clone();
    let mut reader = tokio::task::spawn_blocking(move || {
        let known = reader_owner.existing_registration(
            "writer",
            import::RegistrationKey::Specification("https://api.example.com/first.json"),
        );
        let search = reader_owner.search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"api-example-com"}),
        );
        let captured = reader_owner.catalog.read();
        (known, search, captured)
    });
    let read = tokio::time::timeout(Duration::from_secs(5), &mut reader).await;
    let pre_release_requests = requests.load(Ordering::SeqCst);
    let duplicate_waiting = !second.is_finished();
    gate.release();
    let late_read = if read.is_err() {
        Some(reader.await)
    } else {
        None
    };
    let first = first.await;
    let second = second.await;
    server.abort();
    let _ = server.await;
    // All owner work is released/joined before checking any outcome.
    assert!(
        matches!(held_commit, Ok(Ok(()))),
        "actual final-name commit did not reach its gate"
    );
    assert!(
        evicted_complete,
        "fixture must remove completed optional retention while owner retains its document"
    );
    assert!(
        matches!(actual_miss, Ok(Ok(()))),
        "duplicate must actually enter the serialized miss recheck"
    );
    assert!(duplicate_waiting);
    assert_eq!(pre_release_requests, 1);
    assert!(
        late_read.is_none(),
        "ordinary immutable readers waited on the held mutation"
    );
    let (known, search, captured) = read.unwrap().unwrap();
    assert_eq!(known.unwrap(), old);
    assert_eq!(search.unwrap()["total"], 2);
    assert!(Arc::ptr_eq(&previous, &captured));
    let first = first.unwrap().unwrap();
    assert_eq!(first["name"], "api-example-com-2");
    assert_eq!(second.unwrap().unwrap(), first);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "duplicate must not reacquire a possibly-paid document"
    );
    assert_eq!(m.catalog.read().views["writer"].len(), 4);
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
}

#[tokio::test]
async fn source_metadata_and_cursor_use_the_same_captured_generation() {
    let m = manager(None).await;
    register_spec(&m, "writer", "first", "first.json", spec())
        .await
        .unwrap();
    let old = m.catalog.read();
    let initial = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"first","limit":1}),
        )
        .unwrap();
    let cursor = initial["next_cursor"].clone();
    register_spec(&m, "writer", "second", "second.json", spec())
        .await
        .unwrap();
    let current = m.catalog.read();
    assert!(!Arc::ptr_eq(&old, &current));
    assert_eq!(old.registrations.owners["writer"].len(), 1);
    assert_eq!(old.views["writer"].len(), 2);
    assert_eq!(current.registrations.owners["writer"].len(), 2);
    assert_eq!(current.views["writer"].len(), 4);
    assert!(
        m.search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"first","limit":1,"cursor":cursor})
        )
        .unwrap_err()
        .to_string()
        .contains("stale_cursor")
    );
    assert_eq!(
        m.search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"second"})
        )
        .unwrap()["catalog_generation"],
        current.generation
    );
    assert!(
        m.search_tools(
            "reader",
            "x402_treazury_tools_search",
            json!({"source_name":"second"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn static_base_never_retains_copied_dynamic_metadata() {
    let m = manager(None).await;
    register_spec(&m, "writer", "first", "first.json", spec())
        .await
        .unwrap();
    let mut base = m.static_snapshot().await.unwrap();
    base.registrations = m.catalog.read().registrations.clone();
    m.replace_static(base.generation, base).await.unwrap();
    assert!(
        m.static_snapshot()
            .await
            .unwrap()
            .registrations
            .owners
            .is_empty()
    );
    assert_eq!(m.catalog.read().registrations.owners["writer"].len(), 1);
    let mut base = m.static_snapshot().await.unwrap();
    base.registrations = m.catalog.read().registrations.clone();
    let clean = Manager::new(
        listeners(false),
        BTreeMap::from([("shared".into(), payer())]),
        Arc::new(CatalogState::new(base)),
        vec![],
    )
    .await
    .unwrap();
    assert!(
        clean
            .static_snapshot()
            .await
            .unwrap()
            .registrations
            .owners
            .is_empty()
    );
    assert!(clean.catalog.read().registrations.owners.is_empty());
}

#[tokio::test]
async fn failed_persistence_keeps_registration_metadata_and_tools_on_previous_arc() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(Some(dir.path().join("registry.sqlite"))).await;
    let first = register_spec(&m, "writer", "first", "first.json", spec())
        .await
        .unwrap();
    let previous = m.catalog.read();
    m.inner
        .lock()
        .unwrap()
        .stores
        .get("writer")
        .unwrap()
        .reject_writes();
    assert!(
        register_spec(&m, "writer", "second", "second.json", spec())
            .await
            .is_err()
    );
    assert!(Arc::ptr_eq(&previous, &m.catalog.read()));
    assert_eq!(
        m.existing_registration(
            "writer",
            import::RegistrationKey::Specification("https://api.example.com/first.json")
        )
        .unwrap(),
        first
    );
    assert!(
        m.existing_registration(
            "writer",
            import::RegistrationKey::Specification("https://api.example.com/second.json")
        )
        .is_none()
    );
    assert!(
        m.search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"second"})
        )
        .is_err()
    );
}
