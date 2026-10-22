//! Replace only the SDK capability; the serialized Store and accepted work stay owned.
use super::*;

impl Treasury {
    pub(super) async fn reconstruct_sdk(&mut self, stop: &CancellationToken) -> Result<()> {
        ensure!(
            self.completed.is_none(),
            "completed publication still owned"
        );
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self.healthy = false;
        self.reusable_sync_revision = None;
        self.last_completed_sync = None;
        drop(self.client.take());
        // This serialized read follows every accepted Store write, including a
        // commit whose acknowledgment was lost. Never save the failed SDK here.
        let (id, revision, bytes) = self
            .store
            .call(|store| {
                let (revision, bytes) = store.snapshot()?;
                Ok((store.id().to_owned(), revision, bytes))
            })
            .await?;
        crate::network::global()
            .release_grpc(&crate::network::IsolationId::treasury(&id))
            .await;
        ensure!(!stop.is_cancelled(), "treasury stopping");
        let client = LightClient::from_reader(
            bytes.as_slice(),
            config(self._scratch.path(), WalletConfig::Read, self.network)?,
        )
        .await
        .map_err(|_| SnapshotUnavailable)?;
        ensure!(
            client.indexer_uri().is_none(),
            "reconstructed SDK must be offline"
        );
        ensure!(!stop.is_cancelled(), "treasury stopping");
        self.client = Some(client);
        self.revision = revision;
        self.healthy = true;
        tracing::info!(
            "treasury SDK reconstructed from committed state; accepted work remains owned"
        );
        Ok(())
    }
}

pub(super) fn retryable(error: &anyhow::Error) -> bool {
    error.is::<SnapshotUnavailable>() || crate::rotation::store::inventory::transient_read(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    async fn fixture() -> (tempfile::TempDir, Treasury) {
        let dir = tempfile::tempdir().unwrap();
        let owner = Treasury::create(
            dir.path().join("state"),
            dir.path().join("key"),
            2_000_000,
            Some(Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        (dir, owner)
    }
    #[tokio::test]
    async fn failed_derivation_reconstructs_committed_range_without_reopening_shared_store() {
        let (dir, mut owner) = fixture().await;
        let addresses = owner.addresses().await.unwrap();
        let id = owner.status().await.unwrap().treasury_id;
        let base = owner.store.clone();
        let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_snapshot BEFORE INSERT ON snapshots BEGIN SELECT RAISE(ABORT,'fixture snapshot failure'); END;").unwrap();
        assert!(owner.derive_address().await.is_err());
        assert!(!owner.healthy);
        db.execute_batch("DROP TRIGGER reject_snapshot;").unwrap();
        owner
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(owner.addresses().await.unwrap(), addresses);
        assert_eq!(owner.revision, 1);
        assert!(Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).is_err());
        assert_eq!(
            base.call(|s| Ok(s.status()?.treasury_id)).await.unwrap(),
            id
        );
        let derived = owner.derive_address().await.unwrap();
        assert_eq!(
            derived.as_array().unwrap().len(),
            addresses.as_array().unwrap().len() + 1
        );
        drop(base);
        owner.close().await.unwrap();
    }
    #[tokio::test]
    async fn lost_ack_reconstructs_latest_commit_and_keeps_uncertain_input_owner_and_restriction() {
        let (_dir, mut owner) = fixture().await;
        owner
            .client
            .as_mut()
            .unwrap()
            .generate_unified_address(ReceiverSelection::all_shielded(), zip32::AccountId::ZERO)
            .await
            .unwrap();
        let committed_addresses = owner.addresses().await.unwrap();
        let bytes = snapshot(owner.client.as_ref().unwrap()).await.unwrap();
        owner
            .store
            .call(move |s| {
                s.save_snapshot(1, &bytes)?;
                s.accept_preparation(
                    "uncertain",
                    None,
                    100,
                    Some(crate::rotation::store::preparation::CalculationInputs {
                        destination: "original destination".into(),
                        inputs: vec![crate::rotation::store::preparation::OwnedInput {
                            pool: crate::rotation::store::preparation::InputPool::Orchard,
                            txid: "original input".into(),
                            output_index: 7,
                        }],
                    }),
                )?;
                s.deny_new_funding();
                Err::<(), _>(anyhow::anyhow!("fixture lost acknowledgment"))
            })
            .await
            .unwrap_err();
        owner.healthy = false;
        owner
            .reconstruct_sdk(&CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(owner.revision, 2);
        assert_eq!(owner.addresses().await.unwrap(), committed_addresses);
        owner
            .store
            .call(|s| {
                assert!(s.require_unstarted_preparation("uncertain").is_err());
                assert!(s.abandon_unprepared("uncertain").is_err());
                assert!(s.require_preparation_available().is_err());
                let inputs = s.prepared_inputs("uncertain")?;
                assert_eq!(
                    (inputs[0].txid.as_str(), inputs[0].output_index),
                    ("original input", 7)
                );
                assert!(s.accept_preparation("new", None, 1, None).is_err());
                Ok(())
            })
            .await
            .unwrap();
        assert!(owner.healthy);
        owner.close().await.unwrap();
    }
    #[tokio::test]
    async fn shutdown_during_store_barrier_never_installs_a_replacement_sdk() {
        let (_dir, mut owner) = fixture().await;
        owner.healthy = false;
        let store = owner.store.clone();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let block = tokio::spawn(async move {
            store
                .call(move |_| {
                    entered.send(()).unwrap();
                    wait.blocking_recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap();
        });
        ready.await.unwrap();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let result = owner.reconstruct_sdk(&stopped).await;
            (owner, result)
        });
        tokio::task::yield_now().await;
        stop.cancel();
        release.send(()).unwrap();
        block.await.unwrap();
        let (owner, result) = task.await.unwrap();
        assert!(result.is_err());
        assert!(!owner.healthy);
        // Cancellation before quiescence may leave the old offline client; a
        // replacement must never become healthy or resume queued commands.
        owner.close().await.unwrap();
    }
}
