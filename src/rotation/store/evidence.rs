//! Exact supporting values live with the journal; runtime credentials never survive reopen.
use super::*;
use crate::rotation::base::{Anchor, evidence::BalanceObservation};

pub(super) fn initialize(db: &Connection) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("
ALTER TABLE wallets ADD COLUMN financial_height INTEGER;
ALTER TABLE wallets ADD COLUMN financial_hash TEXT;
ALTER TABLE wallets ADD COLUMN debit_height INTEGER;
ALTER TABLE wallets ADD COLUMN debit_hash TEXT;
ALTER TABLE wallets ADD COLUMN evidence_repair TEXT;
ALTER TABLE wallets ADD COLUMN unresolved_exposure TEXT NOT NULL DEFAULT '0';
ALTER TABLE payment_attempts ADD COLUMN reconciliation_sequence INTEGER;
WITH ordered AS (SELECT id,ROW_NUMBER() OVER (ORDER BY id) AS sequence FROM payment_attempts)
UPDATE payment_attempts SET reconciliation_sequence=(SELECT sequence FROM ordered WHERE ordered.id=payment_attempts.id);
CREATE UNIQUE INDEX payment_reconciliation_sequence ON payment_attempts(reconciliation_sequence);
CREATE INDEX payment_pending_sequence ON payment_attempts(pool_id,state,reconciliation_sequence);
CREATE INDEX payment_wallet_state ON payment_attempts(wallet_id,state);
ALTER TABLE payment_admission_evidence ADD COLUMN support TEXT;
ALTER TABLE payment_admission_evidence ADD COLUMN latest_height INTEGER;
ALTER TABLE payment_admission_evidence ADD COLUMN latest_hash TEXT;
ALTER TABLE payment_admission_evidence ADD COLUMN endpoint TEXT;
CREATE INDEX admission_confirmed_support ON payment_admission_evidence(block_hash,attempt_id);
CREATE INDEX admission_latest_support ON payment_admission_evidence(latest_hash,attempt_id);
ALTER TABLE payment_resolutions ADD COLUMN support TEXT;
CREATE INDEX resolution_anchor_support ON payment_resolutions(hash,attempt_id);
CREATE INDEX IF NOT EXISTS refund_operation_outputs ON refund_outputs(operation_id);
PRAGMA user_version=14;")?;
    // Decimal text is never summed by SQLite's floating point coercion. This is
    // a one-time journal derivation, not an ordinary admission/history sweep.
    {
        let mut rows = tx.prepare("SELECT wallet_id,amount FROM payment_attempts WHERE state!='RESOLVED' ORDER BY wallet_id")?;
        let mut rows = rows.query([])?;
        let mut wallet = String::new();
        let mut total = U256::ZERO;
        while let Some(row) = rows.next()? {
            let next: String = row.get(0)?;
            if next != wallet {
                if !wallet.is_empty() {
                    tx.execute(
                        "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
                        params![wallet, total.to_string()],
                    )?;
                }
                wallet = next;
                total = U256::ZERO;
            }
            total = total
                .checked_add(amount(&row.get::<_, String>(1)?)?)
                .context("reservation overflow")?;
        }
        if !wallet.is_empty() {
            tx.execute(
                "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
                params![wallet, total.to_string()],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BalanceSupport {
    pub observation: BalanceObservation,
    pub checkpoint: Option<Anchor>,
}

pub(super) fn exposure(db: &Connection, wallet: &str) -> Result<U256> {
    let value: String = db.query_row(
        "SELECT unresolved_exposure FROM wallets WHERE id=?1",
        [wallet],
        |row| row.get(0),
    )?;
    amount(&value)
}
pub(super) fn add_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = exposure(db, wallet)?
        .checked_add(value)
        .context("reservation overflow")?;
    ensure!(
        db.execute(
            "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
            params![wallet, total.to_string()]
        )? == 1,
        "missing liability wallet"
    );
    Ok(())
}
pub(super) fn subtract_exposure(db: &Connection, wallet: &str, value: U256) -> Result<()> {
    let total = exposure(db, wallet)?
        .checked_sub(value)
        .context("reservation underflow")?;
    ensure!(
        db.execute(
            "UPDATE wallets SET unresolved_exposure=?2 WHERE id=?1",
            params![wallet, total.to_string()]
        )? == 1,
        "missing liability wallet"
    );
    Ok(())
}

pub(super) fn next_sequence(db: &Connection) -> Result<i64> {
    let previous: i64 = db.query_row(
        "SELECT COALESCE(MAX(reconciliation_sequence),0) FROM payment_attempts",
        [],
        |row| row.get(0),
    )?;
    previous
        .checked_add(1)
        .context("reconciliation sequence exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_history_derivation_streams_exact_amounts_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let identity = store.id().to_owned();
        let pool = store.ensure_pool("history", "1").unwrap();
        let wallets = store.status().unwrap().pools.remove(0).addresses;
        let cost = U256::from(10).pow(U256::from(30));
        let mut expected = [U256::ZERO; 2];
        let tx = store.db.transaction().unwrap();
        for number in 0..2048 {
            let owner = number % 2;
            let state = if number % 3 == 0 {
                "RESOLVED"
            } else {
                "POSSIBLY_SUBMITTED"
            };
            tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES(?1,?2,?3,0,?4,'fixture',?5)",params![format!("attempt-{number:08}"),pool,wallets[owner].id,cost.to_string(),state]).unwrap();
            if state != "RESOLVED" {
                expected[owner] += cost;
            }
        }
        tx.commit().unwrap();
        store
            .db
            .execute_batch(include_str!(
                "../../../tests/fixtures/state/remove_evidence_v14.sql"
            ))
            .unwrap();
        store.db.execute_batch("PRAGMA user_version=13;").unwrap();
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        for (wallet, expected) in wallets.iter().zip(expected) {
            assert_eq!(exposure(&store.db, &wallet.id).unwrap(), expected);
            let checkpoint: Option<i64> = store
                .db
                .query_row(
                    "SELECT financial_height FROM wallets WHERE id=?1",
                    [&wallet.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                checkpoint, None,
                "migration cannot invent financial support"
            );
        }
        assert_eq!(next_sequence(&store.db).unwrap(), 2049);
        let earliest: String = store
            .db
            .query_row(
                "SELECT id FROM payment_attempts ORDER BY reconciliation_sequence LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(earliest, "attempt-00000000");
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        assert_eq!(next_sequence(&store.db).unwrap(), 2049);
    }

    #[test]
    fn exposure_arithmetic_is_checked_and_rollback_preserves_total() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        store.ensure_pool("history", "1").unwrap();
        let wallet = store
            .status()
            .unwrap()
            .pools
            .remove(0)
            .addresses
            .remove(0)
            .id;
        add_exposure(&store.db, &wallet, U256::MAX).unwrap();
        assert!(add_exposure(&store.db, &wallet, U256::from(1)).is_err());
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::MAX);
        subtract_exposure(&store.db, &wallet, U256::MAX).unwrap();
        assert!(subtract_exposure(&store.db, &wallet, U256::from(1)).is_err());
        let tx = store.db.transaction().unwrap();
        add_exposure(&tx, &wallet, U256::from(10)).unwrap();
        tx.rollback().unwrap();
        assert_eq!(exposure(&store.db, &wallet).unwrap(), U256::ZERO);
    }
}
