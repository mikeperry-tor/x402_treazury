//! Completed SDK results stay with the existing owner until local publication.
use super::*;
use crate::rotation::transaction::{PreparedTransaction, TransactionFacts};

pub(super) const MAINTENANCE_SECONDS: u64 = 60;

pub(super) struct CompletedPreparation {
    operation: String,
    expected_revision: i64,
    raw: Zeroizing<Vec<u8>>,
    snapshot: Option<Zeroizing<Vec<u8>>>,
    // Invalid completed bytes are retained without broadcast-ready facts.
    facts: Option<TransactionFacts>,
}
impl Treasury {
    pub(super) async fn retain_calculation(
        &mut self,
        operation: String,
        raw: Zeroizing<Vec<u8>>,
        facts: Result<TransactionFacts>,
    ) -> Result<PreparedTransaction> {
        let (facts, invalid) = match facts {
            Ok(facts) => (Some(facts), None),
            Err(error) => (None, Some(error)),
        };
        self.completed = Some(CompletedPreparation {
            operation,
            expected_revision: self.revision,
            raw,
            snapshot: None,
            facts,
        });
        let prepared = match self.publish_completed().await {
            Ok(value) => value,
            Err(error) => {
                Self::warn_publication_pending();
                return Err(error);
            }
        };
        match invalid {
            Some(error) => {
                Err(error.context("completed transaction retained without submission authority"))
            }
            None => Ok(prepared),
        }
    }

    /// Only this idempotent local boundary repeats; SDK calculation never does.
    pub(super) async fn publish_completed(&mut self) -> Result<PreparedTransaction> {
        let completed = self
            .completed
            .as_mut()
            .context("completed result missing")?;
        if completed.snapshot.is_none() {
            completed.snapshot =
                Some(snapshot(self.client.as_ref().context("treasury unavailable")?).await?);
        }
        let id = completed.operation.clone();
        let expected = completed.expected_revision;
        let raw = completed.raw.clone();
        let bytes = completed
            .snapshot
            .as_ref()
            .expect("completed snapshot")
            .clone();
        let facts = completed.facts.clone();
        let validated = facts.is_some();
        let (revision, prepared) = self
            .store
            .call(move |store| {
                let revision = store.prepare_with_facts(&id, expected, &bytes, &raw, facts)?;
                Ok((revision, PreparedTransaction::load(store, &id)?))
            })
            .await?;
        self.revision = revision;
        self.completed = None;
        self.healthy = validated;
        Ok(prepared)
    }

    pub(super) fn warn_publication_pending() {
        tracing::warn!(
            category = "completed_transaction_publication_pending",
            "completed transaction retained by treasury owner; local storage needs attention; SDK mutation paused"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    async fn fixture(valid: bool) -> (tempfile::TempDir, Treasury, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut treasury = Treasury::create(
            dir.path().join("state"),
            dir.path().join("key"),
            2_000_000,
            Some(Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        treasury
            .store
            .call(|s| s.accept_preparation("completed", None, 60, None))
            .await
            .unwrap();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_completed BEFORE INSERT ON outgoing BEGIN SELECT RAISE(ABORT,'fixture publication failure'); END;").unwrap();
        treasury.healthy = false;
        let facts = TransactionFacts {
            txid: "original".into(),
            expiry_height: 100,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: None,
        };
        assert!(
            treasury
                .retain_calculation(
                    "completed".into(),
                    Zeroizing::new(b"exact completed bytes".to_vec()),
                    if valid {
                        Ok(facts)
                    } else {
                        Err(anyhow::anyhow!("fixture validation failure"))
                    }
                )
                .await
                .is_err()
        );
        assert!(treasury.completed.is_some());
        (dir, treasury, db)
    }
    #[tokio::test]
    async fn completed_result_retries_only_identical_local_bytes_after_failure_or_lost_ack() {
        let (_dir, mut treasury, db) = fixture(true).await;
        assert!(!treasury.healthy);
        assert!(treasury.derive_address().await.is_err());
        let original = treasury.completed.as_ref().unwrap().raw.clone();
        let revision = treasury.revision;
        let error = treasury
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("completed publication still owned")
        );
        assert_eq!(treasury.revision, revision);
        assert_eq!(
            treasury.completed.as_ref().unwrap().raw.as_slice(),
            original.as_slice()
        );
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        let completion = treasury.completed.as_ref().unwrap();
        let (id, revision, bytes, raw, facts) = (
            completion.operation.clone(),
            completion.expected_revision,
            completion.snapshot.clone().unwrap(),
            completion.raw.clone(),
            completion.facts.clone(),
        );
        // The write commits but its result is lost. Current permission may pause;
        // neither event discards the accepted completion or mints another send.
        treasury
            .store
            .call(move |s| {
                s.prepare_with_facts(&id, revision, &bytes, &raw, facts)?;
                s.deny_new_funding();
                Err::<(), anyhow::Error>(anyhow::anyhow!("fixture lost acknowledgement"))
            })
            .await
            .unwrap_err();
        let saved = treasury.publish_completed().await.unwrap();
        assert_eq!(saved.bytes(), b"exact completed bytes");
        assert_eq!(treasury.revision, 2);
        assert!(treasury.completed.is_none() && treasury.healthy);
        assert!(
            treasury
                .store
                .call(|s| s.request_broadcast("completed", 1, 1, false))
                .await
                .is_err()
        );
        treasury.close().await.unwrap();
    }
    #[tokio::test]
    async fn invalid_completed_bytes_are_durable_without_broadcast_authority() {
        let (_dir, mut treasury, db) = fixture(false).await;
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        let prepared = treasury.publish_completed().await.unwrap();
        assert_eq!(prepared.bytes(), b"exact completed bytes");
        assert!(prepared.facts().is_none());
        assert!(treasury.completed.is_none() && !treasury.healthy);
        assert!(
            treasury
                .store
                .call(|s| s.request_broadcast("completed", 1, 1, false))
                .await
                .is_err()
        );
        treasury.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_close_waiter_does_not_discard_completed_result() {
        let (dir, treasury, db) = fixture(true).await;
        let store = treasury.store.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let blocked_store = store.clone();
        let blocker = tokio::spawn(async move {
            blocked_store
                .call(move |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.blocking_recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap();
        });
        entered_rx.await.unwrap();
        let mut close = Box::pin(treasury.close());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut close)
                .await
                .is_err()
        );
        drop(close);
        db.execute_batch("DROP TRIGGER reject_completed;").unwrap();
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if store
                    .call(|s| s.operation_pending("completed"))
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            store
                .call(|s| s.prepared_bytes("completed"))
                .await
                .unwrap()
                .as_slice(),
            b"exact completed bytes"
        );
        let id = store.call(|s| Ok(s.id().to_owned())).await.unwrap();
        drop(store);
        drop(db);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(store) =
                    Store::open(&dir.path().join("state"), &dir.path().join("key"), &id)
                {
                    assert_eq!(
                        store.prepared_bytes("completed").unwrap().as_slice(),
                        b"exact completed bytes"
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
