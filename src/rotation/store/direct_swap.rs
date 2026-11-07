//! Durable original direct-swap terms; no remote I/O or spending occurs here.
use super::*;
use serde_json::Value;

const EXTENSION: &str = "CREATE TABLE IF NOT EXISTS direct_operations(id TEXT PRIMARY KEY,owner TEXT NOT NULL,kind TEXT NOT NULL,request_id TEXT NOT NULL,payload BLOB NOT NULL,observe_receive INTEGER NOT NULL DEFAULT 0,UNIQUE(owner,kind,request_id)); CREATE INDEX IF NOT EXISTS direct_receive_demand ON direct_operations(id) WHERE observe_receive=1;";
pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(EXTENSION)?;
    Ok(())
}
/// Read only the indexed principals with durable direct work. This supplies
/// startup ownership hints; encrypted status access still authenticates its owner.
pub(crate) fn direct_operation_owners(
    dir: &Path,
    expected_id: Option<&str>,
) -> Result<std::collections::BTreeSet<String>> {
    if !dir.join("state.sqlite").try_exists()? {
        return Ok(Default::default());
    }
    inspect_state(dir, |db| {
        let id = read_runtime_identity(db)?;
        ensure!(
            expected_id.is_none_or(|expected| expected == id),
            "state identity mismatch"
        );
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='direct_operations')",
            [], |row| row.get(0),
        )?;
        if !exists {
            return Ok(Default::default());
        }
        let owners = db
            .prepare("SELECT DISTINCT owner FROM direct_operations")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<std::collections::BTreeSet<String>>>()?;
        Ok(owners)
    })
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectRoute {
    #[default]
    Near,
    Native,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DirectOperation {
    pub id: String,
    #[serde(default)]
    pub route: DirectRoute,
    pub owner: String,
    pub kind: String,
    pub request_id: String,
    pub terms: Value,
    pub address: Option<String>,
    pub quote: Option<Value>,
    pub status: Option<Value>,
    /// Reservation without a saved response cannot authorize another quote creation.
    pub state: String,
}
#[derive(Debug)]
struct DirectDecodeFailure;
impl std::fmt::Display for DirectDecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("saved direct operation is corrupt or incompatible")
    }
}
impl std::error::Error for DirectDecodeFailure {}

impl StoreHandle {
    pub(crate) fn direct_changes(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.direct_changes.clone()
    }
}
impl Store {
    fn direct_aad(&self, id: &str) -> String {
        format!("v1:{}:{}:direct:{id}", self.id, self.network.name())
    }
    fn direct_by_id(&self, id: &str) -> Result<DirectOperation> {
        let bytes: Vec<u8> = self
            .db
            .query_row(
                "SELECT payload FROM direct_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .context("unknown direct operation")?;
        let decode = || -> Result<DirectOperation> {
            let record: DirectOperation =
                serde_json::from_slice(&unseal(&self.key, &self.direct_aad(id), &bytes)?)?;
            ensure!(record.id == id, "direct operation identity mismatch");
            Ok(record)
        };
        decode().map_err(|_| DirectDecodeFailure.into())
    }
    fn direct_inventory_record(&self, id: &str) -> Result<Option<DirectOperation>> {
        match self.direct_by_id(id) {
            Ok(record) => Ok(Some(record)),
            Err(error) if error.is::<DirectDecodeFailure>() => {
                tracing::error!(category = "direct_operation_unreadable", operation_id = ?Uuid::parse_str(id).ok(),
                    "A saved payment operation cannot be read; its status and automatic reconciliation are unavailable. Other operations remain active");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub fn direct_operation(&self, owner: &str, id: &str) -> Result<DirectOperation> {
        // Authorize before decoding or returning any owned terms.
        let owned: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM direct_operations WHERE id=?1 AND owner=?2)",
            params![id, owner],
            |r| r.get(0),
        )?;
        ensure!(owned, "unknown direct operation");
        let record = self.direct_by_id(id)?;
        ensure!(record.owner == owner, "direct operation owner mismatch");
        Ok(record)
    }
    pub fn direct_request(
        &self,
        owner: &str,
        kind: &str,
        request_id: &str,
    ) -> Result<DirectOperation> {
        let id: String = self
            .db
            .query_row(
                "SELECT id FROM direct_operations WHERE owner=?1 AND kind=?2 AND request_id=?3",
                params![owner, kind, request_id],
                |row| row.get(0),
            )
            .optional()?
            .context("unknown direct operation")?;
        self.direct_operation(owner, &id)
    }
    pub fn reserve_direct(
        &mut self,
        owner: &str,
        kind: &str,
        request_id: &str,
        terms: Value,
    ) -> Result<(DirectOperation, bool)> {
        self.reserve_direct_route(owner, kind, request_id, terms, DirectRoute::Near)
    }
    pub fn reserve_native(
        &mut self,
        owner: &str,
        kind: &str,
        request_id: &str,
        terms: Value,
    ) -> Result<(DirectOperation, bool)> {
        native_amount(&terms)?;
        self.reserve_direct_route(owner, kind, request_id, terms, DirectRoute::Native)
    }
    fn reserve_direct_route(
        &mut self,
        owner: &str,
        kind: &str,
        request_id: &str,
        terms: Value,
        route: DirectRoute,
    ) -> Result<(DirectOperation, bool)> {
        ensure!(
            !owner.is_empty() && !request_id.is_empty(),
            "direct operation owner and request ID are required"
        );
        ensure!(
            matches!(kind, "receive" | "send"),
            "unknown direct operation kind"
        );
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM direct_operations WHERE owner=?1 AND kind=?2 AND request_id=?3",
                params![owner, kind, request_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            let record = self.direct_operation(owner, &id)?;
            ensure!(
                record.terms == terms && record.route == route,
                "request ID already used with different terms"
            );
            return Ok((record, false));
        }
        let record = DirectOperation {
            id: Uuid::new_v4().to_string(),
            route,
            owner: owner.into(),
            kind: kind.into(),
            request_id: request_id.into(),
            terms,
            address: None,
            quote: None,
            status: None,
            state: if route == DirectRoute::Native {
                "reserved"
            } else {
                "unknown_create"
            }
            .into(),
        };
        let payload = seal(
            &self.key,
            &self.direct_aad(&record.id),
            &serde_json::to_vec(&record)?,
        )?;
        self.db.execute("INSERT INTO direct_operations(id,owner,kind,request_id,payload) VALUES (?1,?2,?3,?4,?5)", params![record.id, owner, kind, request_id, payload])?;
        Ok((record, true))
    }
    fn save_direct(&mut self, record: &DirectOperation) -> Result<()> {
        let payload = seal(
            &self.key,
            &self.direct_aad(&record.id),
            &serde_json::to_vec(record)?,
        )?;
        let terminal = matches!(provider_state(record), Some("FAILED" | "REFUNDED"));
        let observe = record.kind == "receive"
            && match record.route {
                DirectRoute::Near => record.quote.is_some() && !terminal,
                DirectRoute::Native => record.address.is_some() && record.state != "confirmed",
            };
        self.db.execute(
            "UPDATE direct_operations SET payload=?2,observe_receive=?3 WHERE id=?1",
            params![record.id, payload, observe],
        )?;
        if observe {
            self.treasury_changes.notify_one();
        }
        Ok(())
    }
    pub fn save_direct_quote(&mut self, id: &str, quote: Value) -> Result<()> {
        let mut record = self.direct_by_id(id)?;
        ensure!(
            record.route == DirectRoute::Near,
            "native operations have no provider quote"
        );
        if let Some(original) = &record.quote {
            ensure!(*original == quote, "direct operation quote is immutable");
            return Ok(());
        }
        ensure!(
            record.address.is_some(),
            "direct address must be persisted before quote"
        );
        record.quote = Some(quote);
        record.state = "pending".into();
        self.save_direct(&record)?;
        self.direct_changes.notify_one();
        Ok(())
    }
    pub fn save_direct_status(&mut self, id: &str, mut status: Value) -> Result<()> {
        let mut record = self.direct_by_id(id)?;
        ensure!(
            record.route == DirectRoute::Near,
            "native operations have no provider status"
        );
        ensure!(
            record.quote.is_some(),
            "direct operation has no saved quote"
        );
        if matches!(
            provider_state(&record),
            Some("SUCCESS" | "FAILED" | "REFUNDED")
        ) {
            return Ok(());
        }
        if status
            .get("provider")
            .and_then(|provider| provider.get("status"))
            .and_then(Value::as_str)
            == Some("SUCCESS")
        {
            let baseline = self.treasury_sync_state()?.snapshot_revision;
            status
                .as_object_mut()
                .context("invalid direct status")?
                .insert("after_snapshot_revision".into(), baseline.into());
        }
        record.status = Some(status);
        self.save_direct(&record)
    }
    pub fn save_native_status(&mut self, id: &str, receipt: Value) -> Result<()> {
        let mut record = self.direct_by_id(id)?;
        ensure!(
            record.route == DirectRoute::Native
                && record.kind == "receive"
                && record.address.is_some(),
            "native receive address is unavailable"
        );
        let revision = receipt
            .get("snapshot_revision")
            .and_then(Value::as_i64)
            .context("native receipt snapshot revision is required")?;
        let previous = record
            .status
            .as_ref()
            .and_then(|status| status.get("receipt"));
        if previous
            .and_then(|receipt| receipt.get("snapshot_revision"))
            .and_then(Value::as_i64)
            .is_some_and(|saved| saved > revision)
            || previous == Some(&receipt)
        {
            return Ok(());
        }
        let was_confirmed = record.state == "confirmed";
        let confirmed = receipt
            .get("confirmed_zatoshis")
            .and_then(Value::as_u64)
            .context("invalid native receipt")?;
        record.state = if confirmed >= native_amount(&record.terms)? {
            "confirmed"
        } else {
            "pending"
        }
        .into();
        record.status = Some(serde_json::json!({"receipt":receipt}));
        self.save_direct(&record)?;
        if was_confirmed && record.state == "pending" {
            self.direct_changes.notify_one();
        }
        Ok(())
    }
    pub(crate) fn native_receive_binding(&self, id: &str) -> Result<String> {
        let record = self.direct_by_id(id)?;
        ensure!(
            record.route == DirectRoute::Native && record.kind == "receive",
            "operation is not a native receive"
        );
        record
            .address
            .context("native receive address is unavailable")
    }
    /// Exact remote operations still awaiting a provider outcome, or native receipt.
    pub fn direct_pending(&self) -> Result<Vec<DirectOperation>> {
        let mut statement = self
            .db
            .prepare("SELECT id FROM direct_operations WHERE observe_receive=1 OR kind='send'")?;
        let mut records = Vec::new();
        for id in statement.query_map([], |row| row.get::<_, String>(0))? {
            let Some(record) = self.direct_inventory_record(&id?)? else {
                continue;
            };
            if (record.route == DirectRoute::Native
                && record.kind == "receive"
                && record.address.is_some()
                && record.state != "confirmed")
                || (record.route == DirectRoute::Near
                    && record.quote.is_some()
                    && !matches!(
                        provider_state(&record),
                        Some("SUCCESS" | "FAILED" | "REFUNDED")
                    ))
            {
                records.push(record);
            }
        }
        Ok(records)
    }
    pub(super) fn direct_receive_needed(&self) -> Result<bool> {
        let sync = self.treasury_sync_state()?.sync;
        let ready = sync.as_ref().filter(|sync| sync.phase == SyncPhase::Ready);
        let mut statement = self
            .db
            .prepare("SELECT id FROM direct_operations WHERE observe_receive=1")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut needed = false;
        for id in ids {
            let Some(record) = self.direct_inventory_record(&id)? else {
                continue;
            };
            if record.route == DirectRoute::Native {
                needed |= record.address.is_some() && record.state != "confirmed";
                continue;
            }
            let observed = record
                .status
                .as_ref()
                .and_then(|s| s.get("observed_at"))
                .and_then(Value::as_u64);
            let baseline = record
                .status
                .as_ref()
                .and_then(|status| status.get("after_snapshot_revision"))
                .and_then(Value::as_i64);
            let completed_after = ready.is_some_and(|sync| match baseline {
                Some(revision) => sync
                    .started_snapshot_revision
                    .is_some_and(|start| start > revision),
                None => observed
                    .zip(sync.checked_at)
                    .is_some_and(|(seen, scanned)| scanned > seen),
            });
            if provider_state(&record) == Some("SUCCESS") && completed_after {
                self.db.execute(
                    "UPDATE direct_operations SET observe_receive=0 WHERE id=?1",
                    [&id],
                )?;
            } else {
                needed = true;
            }
        }
        Ok(needed)
    }
    pub(crate) fn direct_address(&self, id: &str) -> Result<Option<String>> {
        Ok(self.direct_by_id(id)?.address)
    }
    pub(crate) fn save_direct_address(
        &mut self,
        id: &str,
        address: &str,
        expected: i64,
        snapshot: &[u8],
    ) -> Result<i64> {
        let mut record = self.direct_by_id(id)?;
        ensure!(record.address.is_none(), "direct address is immutable");
        record.address = Some(address.into());
        let native_receive = record.route == DirectRoute::Native && record.kind == "receive";
        if native_receive {
            record.state = "pending".into();
        }
        let payload = seal(
            &self.key,
            &self.direct_aad(id),
            &serde_json::to_vec(&record)?,
        )?;
        let revision = self.save_bound_snapshot(
            expected,
            snapshot,
            None,
            None,
            Some((id, payload, native_receive)),
        )?;
        if native_receive {
            self.treasury_changes.notify_one();
            self.direct_changes.notify_one();
        }
        Ok(revision)
    }
}

fn native_amount(terms: &Value) -> Result<u64> {
    let amount: u64 = terms
        .get("amount")
        .and_then(Value::as_str)
        .context("native amount is required")?
        .parse()
        .context("invalid native amount")?;
    ensure!(amount > 0, "native amount must be positive");
    zcash_protocol::value::Zatoshis::from_u64(amount)
        .context("native amount exceeds SDK monetary range")?;
    Ok(amount)
}

fn provider_state(record: &DirectOperation) -> Option<&str> {
    record
        .status
        .as_ref()?
        .get("provider")?
        .get("status")?
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"initial",
        )
        .unwrap();
        (dir, store)
    }
    #[tokio::test]
    async fn concurrent_requests_reserve_one_original_and_reject_changed_terms() {
        let (_dir, store) = fixture();
        let (handle, worker) = StoreHandle::spawn(store);
        let left = handle.call(|s| {
            s.reserve_direct(
                "owner",
                "receive",
                "same",
                json!({"amount":"123456789012345678901234567890"}),
            )
        });
        let right = handle.call(|s| {
            s.reserve_direct(
                "owner",
                "receive",
                "same",
                json!({"amount":"123456789012345678901234567890"}),
            )
        });
        let (left, right) = tokio::join!(left, right);
        let (left, fresh_left) = left.unwrap();
        let (right, fresh_right) = right.unwrap();
        assert_eq!(left, right);
        assert_ne!(fresh_left, fresh_right);
        assert_eq!(left.state, "unknown_create");
        assert!(
            handle
                .call(|s| s.reserve_direct("owner", "receive", "same", json!({"amount":"2"})))
                .await
                .is_err()
        );
        let id = left.id;
        assert!(
            handle
                .call(move |s| s.direct_operation("other", &id))
                .await
                .is_err()
        );
        drop(handle);
        worker.await.unwrap();
    }
    #[tokio::test]
    async fn quote_commit_wakes_its_owner_after_lost_store_ack_without_consuming_treasury_event() {
        use futures_util::FutureExt;
        let (_dir, mut store) = fixture();
        let (op, _) = store
            .reserve_direct("owner", "receive", "lost-ack", json!({}))
            .unwrap();
        store
            .save_direct_address(&op.id, "shielded", 1, b"address")
            .unwrap();
        let (handle, worker) = StoreHandle::spawn(store);
        let direct = handle.direct_changes();
        let treasury = handle.treasury_changes();
        let accepted = handle.clone();
        let id = op.id.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let caller = tokio::spawn(async move {
            accepted
                .call(move |s| {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                    s.save_direct_quote(&id, json!({"deposit":"foreign"}))
                })
                .await
        });
        started.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        let id = op.id;
        handle
            .call(move |s| {
                assert!(s.direct_operation("owner", &id)?.quote.is_some());
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            direct.notified().now_or_never().is_some(),
            "durable quote publication survives reply loss"
        );
        assert!(
            treasury.notified().now_or_never().is_some(),
            "direct owner cannot steal the treasury observation wakeup"
        );
        assert!(direct.notified().now_or_never().is_none());
        drop(handle);
        worker.await.unwrap();
    }
    #[test]
    fn address_snapshot_commit_is_atomic_encrypted_and_survives_restart() {
        let (dir, mut store) = fixture();
        let treasury = store.id().to_owned();
        let (op, _) = store
            .reserve_direct(
                "owner",
                "receive",
                "invoice",
                json!({"secret_terms":"payer-refund"}),
            )
            .unwrap();
        assert!(
            store
                .save_direct_address(&op.id, "shielded", 0, b"new")
                .is_err()
        );
        assert_eq!(store.snapshot().unwrap().0, 1);
        assert!(store.direct_address(&op.id).unwrap().is_none());
        store
            .save_direct_address(&op.id, "shielded", 1, b"new")
            .unwrap();
        store
            .save_direct_quote(&op.id, json!({"depositAddress":"foreign"}))
            .unwrap();
        assert!(store.treasury_observation_needed(&[]).unwrap());
        let encrypted: Vec<u8> = store
            .db
            .query_row("SELECT payload FROM direct_operations", [], |r| r.get(0))
            .unwrap();
        assert!(
            !encrypted
                .windows(b"payer-refund".len())
                .any(|v| v == b"payer-refund")
        );
        assert_eq!(store.snapshot().unwrap().1.as_slice(), b"new");
        drop(store);
        let mut store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )
        .unwrap();
        assert_eq!(
            store
                .direct_operation("owner", &op.id)
                .unwrap()
                .address
                .as_deref(),
            Some("shielded")
        );
        assert_eq!(store.direct_pending().unwrap().len(), 1);
        store
            .save_direct_status(
                &op.id,
                json!({"provider":{"status":"REFUNDED"},"observed_at":1}),
            )
            .unwrap();
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        assert!(store.funding_jobs().unwrap().is_empty());
        assert!(!store.preparation_pending().unwrap());
    }
    fn sync_observation(phase: SyncPhase) -> SyncObservation {
        SyncObservation {
            phase,
            last_error: None,
            snapshot_revision: 0,
            started_snapshot_revision: Some(3),
            checked_at: Some(100),
            checkpoint_at: 100,
            scanned_blocks: 1,
            target_height: Some(1),
            height: Some(1),
            observed_tip_height: Some(1),
            confirmations: 1,
            confirmed_pool_balances_zatoshis: None,
            confirmed_shielded_zatoshis: 0,
            spendable_shielded_zatoshis: 0,
        }
    }
    #[test]
    fn receive_success_requires_a_completed_later_snapshot_even_with_same_second_timestamps() {
        let (_dir, mut store) = fixture();
        let (op, _) = store
            .reserve_direct("owner", "receive", "same-second", json!({"amount":"1"}))
            .unwrap();
        store
            .save_direct_address(&op.id, "shielded", 1, b"address")
            .unwrap();
        store
            .save_direct_quote(&op.id, json!({"deposit":"foreign"}))
            .unwrap();
        let before = store
            .save_sync_snapshot(
                2,
                b"pre-success scan",
                Some(sync_observation(SyncPhase::Ready)),
            )
            .unwrap();
        store
            .save_direct_status(
                &op.id,
                json!({"provider":{"status":"SUCCESS"},"observed_at":100}),
            )
            .unwrap();
        assert_eq!(
            store
                .direct_operation("owner", &op.id)
                .unwrap()
                .status
                .unwrap()["after_snapshot_revision"],
            before
        );
        assert!(
            store.direct_receive_needed().unwrap(),
            "same-second earlier scan cannot finish receive observation"
        );
        let old_completion = store
            .save_sync_snapshot(
                before,
                b"old scan finishes after success",
                Some(sync_observation(SyncPhase::Ready)),
            )
            .unwrap();
        assert!(
            store.direct_receive_needed().unwrap(),
            "a later final checkpoint cannot renew an earlier scan start"
        );
        let mut newer_scan = sync_observation(SyncPhase::Syncing);
        newer_scan.started_snapshot_revision = Some(old_completion + 1);
        let checkpoint = store
            .save_sync_snapshot(old_completion, b"progress", Some(newer_scan.clone()))
            .unwrap();
        assert!(
            store.direct_receive_needed().unwrap(),
            "progress is not scan completion"
        );
        newer_scan.phase = SyncPhase::Failed;
        let failed = store
            .save_sync_snapshot(checkpoint, b"failed", Some(newer_scan.clone()))
            .unwrap();
        assert!(
            store.direct_receive_needed().unwrap(),
            "failed scan is not completion"
        );
        newer_scan.phase = SyncPhase::Ready;
        store
            .save_sync_snapshot(failed, b"post-success completed", Some(newer_scan))
            .unwrap();
        assert!(!store.direct_receive_needed().unwrap());
    }
    #[test]
    fn historical_sync_observation_decodes_without_scan_start_revision() {
        let mut old = serde_json::to_value(sync_observation(SyncPhase::Ready)).unwrap();
        old.as_object_mut()
            .unwrap()
            .remove("started_snapshot_revision");
        let decoded: SyncObservation = serde_json::from_value(old).unwrap();
        assert_eq!(decoded.started_snapshot_revision, None);
        assert_eq!(decoded.checked_at, Some(100));
    }
    #[test]
    fn encrypted_owner_binding_rejects_a_substituted_lookup_owner() {
        let (_dir, mut store) = fixture();
        let (op, _) = store
            .reserve_direct("owner", "receive", "bound", json!({"amount":"1"}))
            .unwrap();
        store
            .db
            .execute(
                "UPDATE direct_operations SET owner='other' WHERE id=?1",
                [&op.id],
            )
            .unwrap();
        assert!(store.direct_operation("other", &op.id).is_err());
    }
    #[test]
    fn direct_owner_inspection_is_read_only_compatible_and_checks_treasury_identity() {
        let missing = tempfile::tempdir().unwrap();
        let state = missing.path().join("absent");
        assert!(
            direct_operation_owners(&state, Some("unknown"))
                .unwrap()
                .is_empty()
        );
        assert!(!state.exists());
        let (dir, mut store) = fixture();
        let state = dir.path().join("state");
        let id = store.id().to_owned();
        store
            .reserve_direct("owner-a", "receive", "a", json!({}))
            .unwrap();
        store
            .reserve_direct("owner-a", "receive", "b", json!({}))
            .unwrap();
        store
            .reserve_direct("owner-b", "receive", "c", json!({}))
            .unwrap();
        assert_eq!(
            direct_operation_owners(&state, Some(&id)).unwrap(),
            ["owner-a".to_owned(), "owner-b".to_owned()].into()
        );
        assert!(direct_operation_owners(&state, Some("different-treasury")).is_err());
        store
            .db
            .execute_batch("DROP TABLE direct_operations")
            .unwrap();
        assert!(
            direct_operation_owners(&state, Some(&id))
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='direct_operations')",
                    [],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
        assert!(direct_operation_owners(&state, Some("different-treasury")).is_err());
    }
    #[test]
    fn native_reservation_address_and_partial_receipt_are_durable_without_provider_quotes() {
        let (dir, mut store) = fixture();
        let id = store.id().to_owned();
        let terms = json!({"amount":"40000"});
        let (reserved, fresh) = store
            .reserve_native("owner", "receive", "native", terms.clone())
            .unwrap();
        assert!(fresh);
        assert_eq!(reserved.state, "reserved");
        assert!(store.direct_pending().unwrap().is_empty());
        drop(store);
        let mut store =
            Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        let (resumed, fresh) = store
            .reserve_native("owner", "receive", "native", terms.clone())
            .unwrap();
        assert!(!fresh);
        assert_eq!(reserved, resumed);
        assert!(
            store
                .reserve_direct("owner", "receive", "native", terms)
                .is_err()
        );
        store
            .save_direct_address(&reserved.id, "shielded-native", 1, b"native-address")
            .unwrap();
        assert_eq!(
            store.native_receive_binding(&reserved.id).unwrap(),
            "shielded-native"
        );
        assert_eq!(store.direct_pending().unwrap().len(), 1);
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert!(store.save_direct_quote(&reserved.id, json!({})).is_err());
        store
            .save_native_status(
                &reserved.id,
                json!({"confirmed_zatoshis":20000,"spendable_zatoshis":20000,"snapshot_revision":2}),
            )
            .unwrap();
        assert_eq!(
            store.direct_operation("owner", &reserved.id).unwrap().state,
            "pending"
        );
        assert_eq!(store.direct_pending().unwrap().len(), 1);
        store
            .save_native_status(
                &reserved.id,
                json!({"confirmed_zatoshis":50000,"spendable_zatoshis":0,"snapshot_revision":3}),
            )
            .unwrap();
        assert_eq!(
            store.direct_operation("owner", &reserved.id).unwrap().state,
            "confirmed"
        );
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        assert!(store.funding_jobs().unwrap().is_empty());
    }
    #[test]
    fn native_receipt_publication_preserves_newer_sdk_facts_and_reopens_reorg_demand() {
        use futures_util::FutureExt;
        let (_dir, mut store) = fixture();
        let (op, _) = store
            .reserve_native("owner", "receive", "ordered", json!({"amount":"40000"}))
            .unwrap();
        store
            .save_direct_address(&op.id, "shielded", 1, b"address")
            .unwrap();
        let partial = json!({"confirmed_zatoshis":20000,"snapshot_revision":4});
        store.save_native_status(&op.id, partial.clone()).unwrap();
        store
            .save_native_status(
                &op.id,
                json!({"confirmed_zatoshis":50000,"snapshot_revision":3}),
            )
            .unwrap();
        assert_eq!(
            store
                .direct_operation("owner", &op.id)
                .unwrap()
                .status
                .unwrap()["receipt"],
            partial
        );
        let paid = json!({"confirmed_zatoshis":50000,"snapshot_revision":5});
        store.save_native_status(&op.id, paid.clone()).unwrap();
        store.save_native_status(&op.id, partial).unwrap();
        let record = store.direct_operation("owner", &op.id).unwrap();
        assert_eq!(record.status.unwrap()["receipt"], paid);
        assert_eq!(record.state, "confirmed");
        let _ = store.direct_changes.notified().now_or_never();
        let _ = store.treasury_changes.notified().now_or_never();
        store
            .save_native_status(
                &op.id,
                json!({"confirmed_zatoshis":0,"snapshot_revision":6}),
            )
            .unwrap();
        assert_eq!(
            store.direct_operation("owner", &op.id).unwrap().state,
            "pending"
        );
        assert!(store.direct_receive_needed().unwrap());
        assert!(store.direct_changes.notified().now_or_never().is_some());
        assert!(store.treasury_changes.notified().now_or_never().is_some());
    }
    #[test]
    fn corrupt_direct_payload_cannot_stop_valid_sibling_observation_or_release_its_record() {
        let (_dir, mut store) = fixture();
        let (bad, _) = store
            .reserve_native("owner", "receive", "corrupt", json!({"amount":"1"}))
            .unwrap();
        store
            .save_direct_address(&bad.id, "bad-address", 1, b"bad-address")
            .unwrap();
        let (valid, _) = store
            .reserve_native("owner", "receive", "valid", json!({"amount":"1"}))
            .unwrap();
        store
            .save_direct_address(&valid.id, "valid-address", 2, b"valid-address")
            .unwrap();
        store
            .db
            .execute(
                "UPDATE direct_operations SET payload=X'0102' WHERE id=?1",
                [&bad.id],
            )
            .unwrap();
        let revision = store.snapshot().unwrap().0;
        assert_eq!(
            store
                .direct_pending()
                .unwrap()
                .iter()
                .map(|record| &record.id)
                .collect::<Vec<_>>(),
            vec![&valid.id]
        );
        assert!(store.direct_receive_needed().unwrap());
        assert!(store.treasury_observation_needed(&[]).unwrap());
        assert!(
            store
                .direct_operation("owner", &bad.id)
                .unwrap_err()
                .is::<DirectDecodeFailure>()
        );
        store
            .save_native_status(
                &valid.id,
                json!({"confirmed_zatoshis":1,"snapshot_revision":revision}),
            )
            .unwrap();
        assert!(store.direct_pending().unwrap().is_empty());
        assert!(!store.direct_receive_needed().unwrap());
        assert!(!store.treasury_observation_needed(&[]).unwrap());
        let (payload, observe): (Vec<u8>, bool) = store
            .db
            .query_row(
                "SELECT payload,observe_receive FROM direct_operations WHERE id=?1",
                [&bad.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(payload, vec![1, 2]);
        assert!(observe);
        assert_eq!(store.snapshot().unwrap().0, revision);
        assert!(store.funding_jobs().unwrap().is_empty());
        assert!(!store.status().unwrap().outgoing_pending);
    }
    #[test]
    fn caller_request_lookup_is_scoped_to_owner_and_operation_kind() {
        let (_dir, mut store) = fixture();
        let (receive, _) = store
            .reserve_native("owner", "receive", "same", json!({"amount":"1"}))
            .unwrap();
        let (send, _) = store
            .reserve_native("owner", "send", "same", json!({"amount":"1"}))
            .unwrap();
        let (other, _) = store
            .reserve_native("other", "receive", "same", json!({"amount":"2"}))
            .unwrap();
        assert_eq!(
            store.direct_request("owner", "receive", "same").unwrap().id,
            receive.id
        );
        assert_eq!(
            store.direct_request("owner", "send", "same").unwrap().id,
            send.id
        );
        assert_eq!(
            store.direct_request("other", "receive", "same").unwrap().id,
            other.id
        );
        assert!(store.direct_request("unknown", "receive", "same").is_err());
        assert!(store.direct_request("other", "send", "same").is_err());
    }
    #[test]
    fn old_direct_records_default_to_near_and_native_amounts_are_sdk_bounded() {
        let (_dir, mut store) = fixture();
        let (operation, _) = store
            .reserve_direct("owner", "receive", "legacy", json!({}))
            .unwrap();
        let mut historical = serde_json::to_value(operation).unwrap();
        historical.as_object_mut().unwrap().remove("route");
        let decoded: DirectOperation = serde_json::from_value(historical).unwrap();
        assert_eq!(decoded.route, DirectRoute::Near);
        for amount in ["0", "2100000000000001", "18446744073709551616", "-1"] {
            assert!(
                store
                    .reserve_native("owner", "receive", amount, json!({"amount":amount}))
                    .is_err()
            );
        }
    }
    #[test]
    fn old_schema23_without_extension_opens_and_backups_preserve_records() {
        let (dir, store) = fixture();
        let treasury = store.id().to_owned();
        store
            .db
            .execute_batch("DROP TABLE direct_operations")
            .unwrap();
        drop(store);
        let mut store = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury,
        )
        .unwrap();
        let (op, _) = store
            .reserve_direct("owner", "receive", "restored", json!({"amount":"1"}))
            .unwrap();
        let backup = dir.path().join("backup");
        store.backup(&backup).unwrap();
        drop(store);
        let restored = Store::open(&backup, &backup.join("key"), &treasury).unwrap();
        assert_eq!(restored.direct_operation("owner", &op.id).unwrap(), op);
    }
}
