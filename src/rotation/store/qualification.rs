//! Private qualification evidence: identifiers and accounting, never key material.
use super::*;
use serde_json::{Value, json};

impl Store {
    /// Unresolved exposure after applying a fresh chain view, while the pool gate is held.
    pub(crate) fn qualification_exposure(&self, pool: &str) -> Result<Value> {
        let mut query = self.db.prepare(
            "SELECT wallet_id,amount FROM payment_attempts WHERE pool_id=?1 AND state!='RESOLVED'",
        )?;
        let mut totals = std::collections::BTreeMap::<String, U256>::new();
        for row in query.query_map([pool], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (wallet, value) = row?;
            let sum = totals.entry(wallet).or_default();
            *sum = sum
                .checked_add(amount(&value)?)
                .context("pool observation exposure overflow")?;
        }
        Ok(json!(
            totals
                .into_iter()
                .map(|(w, a)| (w, a.to_string()))
                .collect::<std::collections::BTreeMap<_, _>>()
        ))
    }
    pub fn qualification_payment(&self, attempt: &str) -> Result<Value> {
        let mut payment: Value = self.db.query_row("SELECT a.id,a.pool_id,p.name,a.wallet_id,a.generation,a.amount,a.requirements_hash,w.address,w.balance,w.block_height,w.block_hash FROM payment_attempts a JOIN pools p ON p.id=a.pool_id JOIN wallets w ON w.id=a.wallet_id WHERE a.id=?1 AND a.state='ADMITTED'", [attempt], |r| Ok(json!({"attempt_id":r.get::<_,String>(0)?,"pool":r.get::<_,String>(1)?,"pool_name":r.get::<_,String>(2)?,"wallet":r.get::<_,String>(3)?,"generation":r.get::<_,i64>(4)?,"amount":r.get::<_,String>(5)?,"requirements_hash":r.get::<_,String>(6)?,"address":r.get::<_,String>(7)?,"balance_evidence":{"confirmed_balance_atomic":r.get::<_,String>(8)?,"block_height":r.get::<_,Option<i64>>(9)?,"block_hash":r.get::<_,Option<String>>(10)?}})))?;
        // Called synchronously in the same store-worker command as admission,
        // with the pool gate still held. Include this ADMITTED reservation and
        // every earlier unresolved authorization; never read this after signing.
        let wallet = payment["wallet"]
            .as_str()
            .context("admitted wallet missing")?;
        let mut query = self.db.prepare(
            "SELECT amount FROM payment_attempts WHERE wallet_id=?1 AND state!='RESOLVED'",
        )?;
        let after = query
            .query_map([wallet], |r| r.get::<_, String>(0))?
            .try_fold(U256::ZERO, |sum, value| -> Result<U256> {
                sum.checked_add(amount(&value?)?)
                    .context("admission accounting reservation overflow")
            })?;
        let cost = amount(
            payment["amount"]
                .as_str()
                .context("admitted amount missing")?,
        )?;
        let balance = amount(
            payment["balance_evidence"]["confirmed_balance_atomic"]
                .as_str()
                .context("admitted balance missing")?,
        )?;
        payment["balance_evidence"]["reserved_before_atomic"] = json!(
            after
                .checked_sub(cost)
                .context("admission accounting missing current reservation")?
                .to_string()
        );
        payment["balance_evidence"]["reserved_after_atomic"] = json!(after.to_string());
        payment["balance_evidence"]["available_after_atomic"] = json!(
            balance
                .checked_sub(after)
                .context("admission accounting exceeds confirmed balance")?
                .to_string()
        );
        crate::qualification::validate_admission_balance(&payment)?;
        payment["lifecycle"] = self.qualification_lifecycle()?;
        Ok(payment)
    }
    /// Public structural state for case-correlated lifecycle observations. No
    /// seeds, signatures, quote bodies, upstream error prose or wallet bytes.
    pub fn qualification_lifecycle(&self) -> Result<Value> {
        lifecycle(&self.status()?)
    }
    pub fn qualification_state(&self) -> Result<Value> {
        read(&self.db)
    }
}
fn lifecycle(status: &Status) -> Result<Value> {
    let jobs: Vec<_> = status
        .funding_jobs
        .iter()
        .map(|j| {
            json!({
                "id":j.id,"pool_id":j.pool_id,"pool_name":j.pool_name,
                "wallet_id":j.wallet_id,"recipient":j.recipient,"target":j.target,
                "operation_id":j.operation_id,"phase":j.phase
            })
        })
        .collect();
    let operations: Vec<_> = status
        .treasury_operations
        .iter()
        .map(|o| {
            json!({
                "operation_id":o.operation_id,"submission":o.submission,"attempts":o.attempts
            })
        })
        .collect();
    Ok(
        json!({"treasury_id":status.treasury_id,"pools":status.pools,
            "funding_jobs":jobs,"source_operations":operations}),
    )
}
pub fn qualification_state(dir: &Path) -> Result<Value> {
    directory(dir)?;
    database_paths(dir)?;
    // SQLite NOFOLLOW also rejects symlinked parents (e.g. macOS /var).
    // Parent aliases are supported; the validated database leaf remains protected.
    let dir = dir.canonicalize()?;
    let mut db = Connection::open_with_flags(
        dir.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    let tx = db.transaction()?;
    let state = read(&tx)?;
    tx.commit()?;
    Ok(state)
}
fn read(db: &Connection) -> Result<Value> {
    for (table, label) in [
        ("payment_attempts", "payment attempts"),
        ("budget_entries", "source budget entries"),
        ("wallets", "wallet observations"),
    ] {
        let count: i64 =
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        if count > 10000 {
            tracing::warn!(
                resource = label,
                limit = 10000,
                "qualification snapshot exceeds row limit; export/review support required"
            );
        }
        ensure!(
            count <= 10000,
            "qualification snapshot exceeds 10000 {label}; export/review support required"
        );
    }
    let mut stmt = db.prepare(
        "SELECT id,pool_id,wallet_id,generation,amount,state,payer,payee,nonce,valid_after,valid_before,requirements_hash FROM payment_attempts ORDER BY id",
    )?;
    let attempts=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"pool":r.get::<_,String>(1)?,"wallet":r.get::<_,String>(2)?,"generation":r.get::<_,i64>(3)?,"amount":r.get::<_,String>(4)?,"state":r.get::<_,String>(5)?,"payer":r.get::<_,Option<String>>(6)?,"payee":r.get::<_,Option<String>>(7)?,"nonce":r.get::<_,Option<String>>(8)?,"valid_after":r.get::<_,Option<i64>>(9)?,"valid_before":r.get::<_,Option<i64>>(10)?,"requirements_hash":r.get::<_,String>(11)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    // Historical resolved attempts have no classified evidence; never infer it.
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let resolutions = if version < 11 {
        Vec::new()
    } else {
        db.prepare("SELECT attempt_id,outcome,height,hash,block_time FROM payment_resolutions ORDER BY attempt_id")?
        .query_map([], |r| Ok(json!({"attempt_id":r.get::<_,String>(0)?,"outcome":r.get::<_,String>(1)?,
            "height":r.get::<_,i64>(2)?,"hash":r.get::<_,String>(3)?,"block_time":r.get::<_,i64>(4)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut stmt = db.prepare(
        "SELECT id,day,original_day,requested,reserved,consumed FROM budget_entries ORDER BY id",
    )?;
    let budget=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"day":r.get::<_,i64>(1)?,"original_day":r.get::<_,i64>(2)?,"requested":r.get::<_,i64>(3)?,"reserved":r.get::<_,i64>(4)?,"consumed":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let pending = db
        .prepare("SELECT id FROM outgoing WHERE state='PREPARED' ORDER BY id")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let archived = db.prepare("SELECT r.operation_id FROM funding_recovery r WHERE NOT EXISTS(SELECT 1 FROM outgoing o WHERE o.id=r.operation_id) AND NOT EXISTS(SELECT 1 FROM budget_entries b WHERE b.id=r.operation_id AND b.consumed!=0) ORDER BY r.operation_id")?.query_map([], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let status = read_status(db)?;
    let wallets = db.prepare("SELECT id,pool_id,role,balance,block_height,block_hash FROM wallets ORDER BY id")?
        .query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"pool":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"balance":r.get::<_,String>(3)?,"height":r.get::<_,Option<i64>>(4)?,"hash":r.get::<_,Option<String>>(5)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"wallet_observations":wallets,"lifecycle":lifecycle(&status)?,"treasury_status":status,"payment_attempts":attempts,"payment_resolutions":resolutions,"source_budget_entries":budget,"pending_source_operations":pending,"unprepared_archives":archived}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::base::{Anchor, AuthorizationOutcome, AuthorizationResolution, ChainView};
    #[test]
    fn snapshot_row_bound_rejects_with_warning_and_consumer_error() {
        use std::sync::{Arc, Mutex};
        #[derive(Clone)]
        struct Writer(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE payment_attempts(id); CREATE TABLE budget_entries(id); CREATE TABLE wallets(id); WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10001) INSERT INTO wallets SELECT x FROM n;").unwrap();
        let writer = Writer(Arc::default());
        let sink = writer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let error = tracing::subscriber::with_default(subscriber, || read(&db).unwrap_err());
        assert!(error.to_string().contains("10000 wallet observations"));
        let log = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("limit=10000"));
        assert!(log.contains("qualification snapshot exceeds row limit"));
    }
    #[test]
    fn admission_evidence_counts_all_unresolved_exposure_and_pins_its_balance() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = store.ensure_pool("pool", "100").unwrap();
        let wallet = store.status().unwrap().pools[0].addresses[0].id.clone();
        let snapshot = store.qualification_state().unwrap();
        assert!(
            snapshot["wallet_observations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|w| w["height"].is_null() && w["hash"].is_null())
        );
        let hash = format!("0x{:064x}", 1);
        store
            .db
            .execute(
                "UPDATE wallets SET balance='100',block_height=12,block_hash=?1 WHERE id=?2",
                params![hash, wallet],
            )
            .unwrap();
        for (id, cost, state) in [
            ("pending", "10", "POSSIBLY_SUBMITTED"),
            ("new", "7", "ADMITTED"),
            ("resolved", "50", "RESOLVED"),
        ] {
            store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES(?1,?2,?3,0,?4,'requirements',?5)",params![id,pool,wallet,cost,state]).unwrap();
        }
        let snapshot = store.qualification_state().unwrap();
        let saved = snapshot["wallet_observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == wallet)
            .unwrap();
        assert_eq!(saved["balance"], "100");
        assert_eq!(saved["height"], 12);
        assert_eq!(saved["hash"], hash);
        let evidence = store.qualification_payment("new").unwrap();
        assert_eq!(store.qualification_exposure(&pool).unwrap()[&wallet], "17");
        assert_eq!(
            evidence["lifecycle"]["funding_jobs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        store
            .db
            .execute(
                "UPDATE funding_progress SET last_error='private upstream diagnostic'",
                [],
            )
            .unwrap();
        assert!(
            !store
                .qualification_lifecycle()
                .unwrap()
                .to_string()
                .contains("private upstream diagnostic")
        );
        assert_eq!(
            evidence["balance_evidence"],
            json!({"confirmed_balance_atomic":"100","reserved_before_atomic":"10","reserved_after_atomic":"17","available_after_atomic":"83","block_height":12,"block_hash":hash})
        );
        assert!(store.qualification_payment("pending").is_err());
        store
            .db
            .execute(
                "UPDATE payment_attempts SET amount='94' WHERE id='pending'",
                [],
            )
            .unwrap();
        assert!(store.qualification_payment("new").is_err());
        store
            .db
            .execute(
                "UPDATE payment_attempts SET state='RESOLVED' WHERE id='pending'",
                [],
            )
            .unwrap();
        assert_eq!(
            store.qualification_payment("new").unwrap()["balance_evidence"]["available_after_atomic"],
            "93"
        );
        assert_eq!(store.qualification_exposure(&pool).unwrap()[&wallet], "7");
        store
            .db
            .execute("UPDATE wallets SET block_hash=NULL WHERE id=?1", [wallet])
            .unwrap();
        assert!(store.qualification_payment("new").is_err());
    }
    #[test]
    fn canonical_resolution_is_atomic_immutable_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let treasury = store.id().to_owned();
        let pool = store.ensure_pool("pool", "1").unwrap();
        let wallets = store.status().unwrap().pools.remove(0).addresses;
        for (id, wallet) in ["used", "expired"].into_iter().zip(&wallets) {
            store.db.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES(?1,?2,?3,0,'1','hash','POSSIBLY_SUBMITTED')",
                params![id,pool,wallet.id]).unwrap();
        }
        let view = || ChainView {
            anchor: Anchor {
                height: 10,
                hash: "block".into(),
            },
            balances: wallets
                .iter()
                .map(|w| (w.id.clone(), U256::from(1_000_000)))
                .collect(),
            released: vec!["used".into(), "expired".into()],
            resolutions: [
                (
                    "used".into(),
                    AuthorizationResolution {
                        outcome: AuthorizationOutcome::Used,
                        block_time: 100,
                    },
                ),
                (
                    "expired".into(),
                    AuthorizationResolution {
                        outcome: AuthorizationOutcome::ExpiredUnused,
                        block_time: 100,
                    },
                ),
            ]
            .into(),
        };
        let mut incomplete = view();
        incomplete.balances.clear();
        assert!(store.reconcile_pool(&pool, incomplete).is_err());
        assert_eq!(
            store.qualification_state().unwrap()["payment_resolutions"],
            json!([])
        );
        let mut incomplete = view();
        incomplete.resolutions.remove("expired");
        assert!(store.reconcile_pool(&pool, incomplete).is_err());
        store.reconcile_pool(&pool, view()).unwrap();
        let expected = store.qualification_state().unwrap()["payment_resolutions"].clone();
        assert_eq!(expected[0]["outcome"], "EXPIRED_UNUSED");
        assert_eq!(expected[1]["outcome"], "USED");
        assert_eq!(expected[1]["height"], 10);
        let mut later = view();
        later.anchor.height = 11;
        later.resolutions.get_mut("used").unwrap().outcome = AuthorizationOutcome::ExpiredUnused;
        store.reconcile_pool(&pool, later).unwrap();
        assert_eq!(
            store.qualification_state().unwrap()["payment_resolutions"],
            expected
        );
        drop(store);
        let store = Store::open(&state, &key, &treasury).unwrap();
        assert_eq!(
            store.qualification_state().unwrap()["payment_resolutions"],
            expected
        );
        assert_eq!(
            qualification_state(&state).unwrap()["payment_resolutions"],
            expected
        );
    }
}
