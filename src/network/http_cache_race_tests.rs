//! Force real synchronous miss constructors through the existing private observer.
use super::*;
use std::sync::{
    Condvar,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

type Captured = std::thread::Result<Result<Arc<HttpTransport>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Constructed(usize),
    Finished(usize),
}

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn wait(&self) -> Result<()> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| anyhow::anyhow!("fixture gate poisoned"))?;
        while !*open {
            open = self
                .changed
                .wait(open)
                .map_err(|_| anyhow::anyhow!("fixture gate poisoned"))?;
        }
        Ok(())
    }
    fn release(&self) {
        // Cleanup cannot panic and strand an observer if an earlier fixture failed.
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *open = true;
        self.changed.notify_all();
    }
}

// The scope joins spawned threads after unwinding its closure. Release callback
// gates first, including when OS thread creation itself panics before worker entry.
struct ReleaseOnDrop<'a>(&'a Gate);
impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn acquired(joined: std::thread::Result<Captured>) -> Result<Arc<HttpTransport>> {
    let captured = joined.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    captured.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

async fn constructor_race(fail_first: bool) {
    let observations = Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
    let observed = observations.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let next = count.clone();
    let gate = Arc::new(Gate::default());
    let held = gate.clone();
    let (events, receive) = mpsc::channel();
    let observed_events = events.clone();
    install_http_scope_observer(move |observation| {
        let ordinal = next.fetch_add(1, Ordering::SeqCst);
        observed
            .lock()
            .map_err(|_| anyhow::anyhow!("fixture observations poisoned"))?
            .push(observation.clone());
        observed_events
            .send(Event::Constructed(ordinal))
            .map_err(|_| anyhow::anyhow!("fixture coordinator closed"))?;
        if ordinal == 0 || !fail_first {
            held.wait()?;
        }
        ensure!(
            !(fail_first && ordinal == 0),
            "fixture first scope observation failed"
        );
        Ok(())
    })
    .unwrap();

    let app = axum::Router::new().fallback(|| async { "cache race fixture" });
    let (address, server) = crate::test_tls::serve(app).await;
    let socks = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let network = NetworkContext::new(NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some(socks.address.into()),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let identity = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
    let runtime = tokio::runtime::Handle::current();
    let acquire = || {
        network.http_participating(
            &identity,
            "https://api.example.com/private/path?private=query",
            None,
            false,
            HttpPolicy::COMPATIBLE,
        )
    };
    let worker = |id| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _runtime = runtime.enter();
            acquire()
        }));
        let _ = events.send(Event::Finished(id));
        result
    };
    let (first_event, first_unlocked, second_event, before_release, a, b) =
        std::thread::scope(|scope| {
            let _release = ReleaseOnDrop(&gate);
            let a = scope.spawn(|| worker(0));
            let first = receive.recv().ok();
            // A is the only constructor: this cannot fail because B briefly holds
            // the lookup lock. The old implementation fails here without a deadlock.
            let unlocked = first == Some(Event::Constructed(0)) && network.http.try_lock().is_ok();
            let b = unlocked.then(|| scope.spawn(|| worker(1)));
            let second = if b.is_some() {
                receive.recv().ok()
            } else {
                None
            };
            let finished = if fail_first && second == Some(Event::Constructed(1)) {
                receive.recv().ok()
            } else {
                second
            };
            let cache = if b.is_some()
                && finished
                    == Some(if fail_first {
                        Event::Finished(1)
                    } else {
                        Event::Constructed(1)
                    }) {
                network.http.try_lock().ok().map(|cache| cache.len())
            } else {
                None
            };
            // Capture every outcome, then release and join BOTH workers before any
            // assertion, returned error or caught constructor panic is reported.
            gate.release();
            let a = a.join();
            let b = b.map(|worker| worker.join());
            (first, unlocked, finished, cache, a, b)
        });
    let a = acquired(a);
    let b = b.map(acquired);
    assert_eq!(first_event, Some(Event::Constructed(0)));
    assert!(
        first_unlocked,
        "constructor A held the global HTTP cache mutex"
    );
    assert_eq!(
        second_event,
        Some(if fail_first {
            Event::Finished(1)
        } else {
            Event::Constructed(1)
        })
    );
    assert_eq!(before_release, Some(usize::from(fail_first)));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let b = b.expect("second constructor was started").unwrap();
    if fail_first {
        assert!(format!("{:#}", a.unwrap_err()).contains("fixture first scope observation failed"));
    } else {
        assert!(
            Arc::ptr_eq(&a.unwrap(), &b),
            "same-key callers must receive one canonical winner"
        );
    }
    assert_eq!(network.http_cache_len(), 1);
    assert!(Arc::ptr_eq(&acquire().unwrap(), &b));
    assert_eq!(b.key.runtime, Some(runtime.id()));
    let winner_scope = network.credentials(&b.scope);
    {
        let saved = observations.lock().unwrap();
        assert_eq!(saved.len(), 2);
        assert_ne!(saved[0].scope, saved[1].scope);
        assert!(saved.iter().all(|observation| observation.canonical
            == network.credentials(&identity)
            && observation.origin == "https://api.example.com"));
        assert!(
            saved
                .iter()
                .any(|observation| observation.scope == winner_scope)
        );
        if fail_first {
            assert_eq!(saved[1].scope, winner_scope);
        }
    }
    tokio::task::yield_now().await;
    assert_eq!(
        socks.accepted.load(Ordering::SeqCst),
        0,
        "construction and failed observation must precede all outbound I/O"
    );
    assert!(socks.records.lock().unwrap().is_empty());
    let response = b
        .client
        .get("https://api.example.com/private/path?private=query")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(response.text().await.unwrap(), "cache race fixture");
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].host, "api.example.com");
        assert_eq!(records[0].address_type, 3);
        assert_eq!(
            (records[0].user.clone(), records[0].password.clone()),
            winner_scope
        );
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "winner reuse/disclosure creates no extra construction"
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn same_key_actual_constructors_release_cache_and_return_one_winner() {
    if crate::test_log_capture::run_in_child(
        "network::http_cache_race_tests::same_key_actual_constructors_release_cache_and_return_one_winner",
    ) {
        return;
    }
    constructor_race(false).await;
}

#[tokio::test]
async fn failed_observer_still_fails_its_caller_after_another_constructor_publishes() {
    if crate::test_log_capture::run_in_child(
        "network::http_cache_race_tests::failed_observer_still_fails_its_caller_after_another_constructor_publishes",
    ) {
        return;
    }
    constructor_race(true).await;
}
