//! One retained owner for equivalent in-flight background balance acquisitions.
//! No completed-result cache; advisory observations carry no age permission.
use super::*;
use futures_util::FutureExt;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

type SharedResult = Arc<Result<ChainView, Arc<anyhow::Error>>>;
#[derive(PartialEq, Eq)]
struct Key {
    query: ChainQuery,
    endpoints: Vec<String>,
    confirmations: u64,
}
struct Active {
    key: Key,
    result: watch::Receiver<Option<SharedResult>>,
}
static ACTIVE: Mutex<Vec<Active>> = Mutex::new(Vec::new());

struct RetainedOwner {
    channel: watch::Receiver<Option<SharedResult>>,
    permit: tokio::sync::SemaphorePermit<'static>,
}
impl Drop for RetainedOwner {
    fn drop(&mut self) {
        // Runtime teardown or task abort may bypass the async completion path.
        // Remove active ownership before the query-memory permit is released.
        ACTIVE
            .lock()
            .expect("Base acquisition owners")
            .retain(|owner| !owner.result.same_channel(&self.channel));
    }
}

#[derive(Debug)]
struct SharedFailure(Arc<anyhow::Error>);
impl std::fmt::Display for SharedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for SharedFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
fn copy_failure(error: Arc<anyhow::Error>) -> anyhow::Error {
    // Anyhow downcasts recognize context values, retaining typed scope boundaries.
    let endpoint = error.downcast_ref::<EndpointMismatch>().cloned();
    let conflict = error.downcast_ref::<EvidenceConflict>().cloned();
    let evidence = error.downcast_ref::<evidence::EvidenceFailure>().copied();
    let rpc = error.downcast_ref::<RpcFailure>().cloned();
    let stage = error.downcast_ref::<VerificationStage>().cloned();
    let mut result = anyhow::Error::new(SharedFailure(error));
    if let Some(value) = rpc {
        result = result.context(value);
    }
    if let Some(value) = stage {
        result = result.context(value);
    }
    if let Some(value) = endpoint {
        result = result.context(value);
    }
    if let Some(value) = conflict {
        result = result.context(value);
    }
    if let Some(value) = evidence {
        result = result.context(value);
    }
    result
}

impl BaseRpc {
    pub(crate) async fn shared_balance_view(
        &self,
        query: ChainQuery,
        store: crate::rotation::store::StoreHandle,
    ) -> Result<ChainView> {
        // Historical authorization acquisition has its own retained maintenance owner.
        if query.wallets.len() != 1 || !query.pending.is_empty() {
            return self.view(query).await;
        }
        let wallet = &query.wallets[0].0;
        let capture = query
            .captures
            .get(wallet)
            .context(evidence::EvidenceFailure::ScopeMismatch)?;
        ensure!(
            capture.wallet == *wallet && !capture.namespace.is_nil(),
            evidence::EvidenceFailure::ScopeMismatch
        );
        let key = Key {
            query: query.clone(),
            endpoints: std::iter::once(self)
                .chain(&self.fallbacks)
                .map(Self::endpoint_id)
                .collect(),
            confirmations: self.confirmations,
        };
        let mut receiver = {
            let mut active = ACTIVE.lock().expect("Base acquisition owners");
            if let Some(owner) = active.iter().find(|owner| owner.key == key) {
                owner.result.clone()
            } else {
                // Reuse the existing query-memory permit; never evict active work.
                let permit = resources::query()?;
                let (sender, receiver) = watch::channel(None);
                active.push(Active {
                    key,
                    result: receiver.clone(),
                });
                let rpc = self.clone();
                let owner = RetainedOwner {
                    channel: receiver.clone(),
                    permit,
                };
                tokio::spawn(async move {
                    let _store_owner = store;
                    let result =
                        std::panic::AssertUnwindSafe(rpc.view_with_owner(query, &owner.permit))
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|_| {
                                Err(anyhow::anyhow!("Base acquisition owner failed"))
                            });
                    sender.send_replace(Some(Arc::new(result.map_err(Arc::new))));
                    drop(owner);
                });
                receiver
            }
        };
        let result = loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                break result;
            }
            receiver
                .changed()
                .await
                .context("Base acquisition owner closed")?;
        };
        match result.as_ref() {
            Ok(view) => Ok(view.clone()),
            Err(error) => Err(copy_failure(error.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[tokio::test]
    async fn exact_capture_joins_cancelled_waiter_but_changed_capture_is_independent() {
        let held = Arc::new(AtomicBool::new(true));
        let chains = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let timestamp = now().unwrap();
        let app = Router::new().route("/", post({
            let held = held.clone(); let chains = chains.clone(); let release = release.clone();
            move |Json(value): Json<serde_json::Value>| {
                let held = held.clone(); let chains = chains.clone(); let release = release.clone();
                async move {
                    let result = match value["method"].as_str().unwrap() {
                        "eth_chainId" => {
                            chains.fetch_add(1, Ordering::SeqCst);
                            let notified = release.notified();
                            if held.load(Ordering::SeqCst) { notified.await; }
                            json!("0x2105")
                        },
                        "eth_getBlockByNumber" => {
                            let tag = value["params"][0].as_str().unwrap();
                            let height = if tag == "latest" {100} else {u64::from_str_radix(&tag[2..],16).unwrap()};
                            json!({"number":format!("0x{height:x}"),"hash":format!("0x{height:064x}"),"timestamp":format!("0x{timestamp:x}")})
                        },
                        "eth_call" => json!(format!("0x{:064x}", 5_000_000)),
                        method => panic!("unexpected {method}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":value["id"],"result":result}))
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "shared",
            "5",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        let query = store
            .scoped_chain_query(&pool, vec![job.wallet_id.clone()], vec![])
            .unwrap();
        let (store, owner) = crate::rotation::store::StoreHandle::spawn(store);
        for malformed in 0..3 {
            let mut invalid = query.clone();
            match malformed {
                0 => invalid.captures.clear(),
                1 => invalid.captures.get_mut(&job.wallet_id).unwrap().wallet = "other".into(),
                _ => {
                    invalid.captures.get_mut(&job.wallet_id).unwrap().namespace = uuid::Uuid::nil()
                }
            }
            let error = BaseRpc::new(&url, 12)
                .unwrap()
                .shared_balance_view(invalid, store.clone())
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.downcast_ref::<evidence::EvidenceFailure>(),
                Some(&evidence::EvidenceFailure::ScopeMismatch)
            );
        }
        assert_eq!(chains.load(Ordering::SeqCst), 0);
        let launch = |query: ChainQuery| {
            let store = store.clone();
            let rpc = BaseRpc::new(&url, 12).unwrap();
            tokio::spawn(async move { rpc.shared_balance_view(query, store).await })
        };
        let first = launch(query.clone());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while chains.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        first.abort();
        let _ = first.await;
        let joined = launch(query.clone());
        let mut changed = query;
        changed.captures.get_mut(&job.wallet_id).unwrap().generation += 1;
        let independent = launch(changed);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while chains.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // The two owners remain blocked; a cancelled first waiter did not spawn a third.
        tokio::task::yield_now().await;
        assert_eq!(chains.load(Ordering::SeqCst), 2);
        held.store(false, Ordering::SeqCst);
        release.notify_waiters();
        let joined = joined.await.unwrap().unwrap();
        let independent = independent.await.unwrap().unwrap();

        assert_eq!(joined.observations, independent.observations);
        drop(store);
        owner.await.unwrap();
        server.abort();
    }

    #[test]
    fn owner_drop_removes_only_its_active_entry_without_polling() {
        let make_owner = || {
            let (_, channel) = watch::channel(None);
            let owner = RetainedOwner {
                channel: channel.clone(),
                permit: resources::query().unwrap(),
            };
            ACTIVE.lock().unwrap().push(Active {
                key: Key {
                    query: ChainQuery {
                        wallets: vec![],
                        pending: vec![],
                        anchor: None,
                        captures: Default::default(),
                    },
                    endpoints: vec![],
                    confirmations: 1,
                },
                result: channel.clone(),
            });
            (owner, channel)
        };
        let (first, first_channel) = make_owner();
        let (second, second_channel) = make_owner();
        drop(first);
        {
            let active = ACTIVE.lock().unwrap();
            assert!(
                !active
                    .iter()
                    .any(|entry| entry.result.same_channel(&first_channel))
            );
            assert!(
                active
                    .iter()
                    .any(|entry| entry.result.same_channel(&second_channel))
            );
        }
        drop(second);
        assert!(
            !ACTIVE
                .lock()
                .unwrap()
                .iter()
                .any(|entry| entry.result.same_channel(&second_channel))
        );
    }

    #[test]
    fn shared_failures_preserve_typed_invalidation_context() {
        let failure = anyhow::Error::new(RpcFailure::malformed("eth_call"))
            .context(VerificationStage("confirmed balance"));
        let diagnostic = safe_diagnostic(&failure);
        let copy = copy_failure(Arc::new(failure));
        assert!(copy.downcast_ref::<RpcFailure>().is_some());
        assert_eq!(
            copy.downcast_ref::<VerificationStage>().unwrap().0,
            "confirmed balance"
        );
        assert_eq!(safe_diagnostic(&copy), diagnostic);
        let mismatch = EndpointMismatch {
            endpoint: "a".repeat(64),
        };
        let copy = copy_failure(Arc::new(
            anyhow::Error::new(mismatch).context("fixture stage"),
        ));
        assert_eq!(
            copy.downcast_ref::<EndpointMismatch>().unwrap().endpoint,
            "a".repeat(64)
        );
        let conflict = EvidenceConflict {
            wallet: "wallet".into(),
            endpoint: "b".repeat(64),
            anchor: Anchor {
                height: 1,
                hash: "hash".into(),
            },
            latest_only: false,
            financial_checkpoint: true,
        };
        let copy = copy_failure(Arc::new(conflict.into()));
        assert!(
            copy.downcast_ref::<EvidenceConflict>()
                .unwrap()
                .financial_checkpoint
        );
    }
}
