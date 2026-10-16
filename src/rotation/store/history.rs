//! Runtime owners capture an upper boundary, then acquire bounded keyset pages.
//! No cursor is durable and no inventory size limits financial authority.
pub(super) const WALLET_PAGE_SQL: &str = "SELECT sequence,id,address FROM wallets WHERE pool_id=?1 AND role!='RETIRED' AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4";

use super::*;
use crate::rotation::base::PendingAuthorization;

/// Bounds rows acquired per store job, not pass length or total wallet/history size.
pub(crate) const PAGE_SIZE: usize = 64;

fn page_limit(limit: usize) -> Result<i64> {
    ensure!(
        (1..=PAGE_SIZE).contains(&limit),
        "invalid internal reconciliation page size"
    );
    Ok(limit as i64)
}

/// Only payload decoding may be scoped. Cursor, attempt identity and wallet/pool
/// ownership must already be established; no obligation is removed by this value.
#[derive(Debug, Clone)]
pub(crate) struct MalformedHistory {
    pub id: String,
    pub wallet: String,
}
impl std::fmt::Display for MalformedHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("malformed_authorization_history")
    }
}
impl std::error::Error for MalformedHistory {}
pub(crate) type HistoryPayload = std::result::Result<PendingAuthorization, MalformedHistory>;

impl Store {
    /// Buffered pages are snapshots, not authority to acquire another nonce proof.
    /// Revalidate identity and state in the serialized store immediately before I/O.
    pub(super) fn pending_history(
        &self,
        pool: &str,
        pending: Vec<PendingAuthorization>,
    ) -> Result<Vec<PendingAuthorization>> {
        let mut current = Vec::with_capacity(pending.len());
        for auth in pending {
            let (wallet, payer, nonce, valid_before, state, resolved): (String, String, String, u64, String, bool) = self.db.query_row(
                "SELECT wallet_id,payer,nonce,valid_before,state,EXISTS(SELECT 1 FROM payment_resolutions r WHERE r.attempt_id=a.id) FROM payment_attempts a WHERE id=?1 AND pool_id=?2",
                params![auth.id, pool],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, sql_u64(r, 3)?, r.get(4)?, r.get(5)?)),
            )?;
            ensure!(
                wallet == auth.wallet
                    && payer == auth.payer
                    && nonce == auth.nonce
                    && valid_before == auth.valid_before,
                "authorization_evidence_binding_mismatch"
            );
            match (state.as_str(), resolved) {
                ("POSSIBLY_SUBMITTED", false) => current.push(auth),
                ("RESOLVED", true) => {}
                _ => anyhow::bail!("inconsistent_authorization_resolution_state"),
            }
        }
        Ok(current)
    }

    pub(crate) fn history_boundary(&self, _pool: &str) -> Result<i64> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::HistoryBoundary)?;
        // Capture the indexed global committed sequence. Filtering the page by
        // group preserves its scope; arrivals in any group cannot expand this
        // pass. Resolved rows retain their immutable sequence across reopen.
        Ok(self.db.query_row(
            "SELECT COALESCE(MAX(reconciliation_sequence),0) FROM payment_attempts",
            [],
            |row| row.get(0),
        )?)
    }
    pub(crate) fn history_page(
        &self,
        pool: &str,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<(i64, HistoryPayload)>> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::HistoryPage)?;
        let mut statement = self.db.prepare(
            "SELECT a.reconciliation_sequence,a.id,a.wallet_id,a.payer,a.nonce,a.valid_before,EXISTS(SELECT 1 FROM wallets w WHERE w.id=a.wallet_id AND w.pool_id=a.pool_id) FROM payment_attempts a WHERE a.pool_id=?1 AND a.state='POSSIBLY_SUBMITTED' AND a.reconciliation_sequence>?2 AND a.reconciliation_sequence<=?3 ORDER BY a.reconciliation_sequence LIMIT ?4",
        )?;
        let mut rows = statement.query(params![pool, after, through, page_limit(limit)?])?;
        let mut page = Vec::new();
        let mut cursor = after;
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let id: String = row.get(1)?;
            let wallet: String = row.get(2)?;
            ensure!(
                sequence > cursor && sequence <= through,
                "invalid_history_cursor"
            );
            ensure!(
                !id.is_empty() && !wallet.is_empty() && row.get::<_, bool>(6)?,
                "unknown_history_ownership"
            );
            let payload = (|| -> rusqlite::Result<PendingAuthorization> {
                Ok(PendingAuthorization {
                    id: id.clone(),
                    wallet: wallet.clone(),
                    payer: row.get(3)?,
                    nonce: row.get(4)?,
                    valid_before: sql_u64(row, 5)?,
                })
            })()
            .map_err(|_| MalformedHistory { id, wallet });
            page.push((sequence, payload));
            cursor = sequence;
        }
        Ok(page)
    }
    pub(crate) fn wallet_boundary(&self, pool: &str) -> Result<i64> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::WalletBoundary)?;
        Ok(self.db.query_row(
            "SELECT COALESCE(MAX(sequence),-1) FROM wallets WHERE pool_id=?1",
            [pool],
            |row| row.get(0),
        )?)
    }
    pub(crate) fn wallet_page(
        &self,
        pool: &str,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<(i64, String, String)>> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::WalletPage)?;
        let mut statement = self.db.prepare(WALLET_PAGE_SQL)?;
        Ok(statement
            .query_map(params![pool, after, through, page_limit(limit)?], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Few preferred identities receive acquisition opportunities alongside the
    /// ordinary cursor. The runtime owner deduplicates both paths and in-flight IDs.
    pub(crate) fn priority_wallets(
        &self,
        pool: &str,
        through: i64,
    ) -> Result<Vec<(i64, String, String)>> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::Priority)?;
        let mut statement = self.db.prepare(
            "SELECT sequence,id,address FROM wallets WHERE pool_id=?1 AND role IN ('ACTIVE','READY','ALLOCATED') AND (role='ACTIVE' OR (role IN ('READY','ALLOCATED') AND EXISTS(SELECT 1 FROM reserve_claims r WHERE r.pool_id=?1 AND r.wallet_id=wallets.id AND r.enabled=1))) AND sequence<=?2 ORDER BY CASE role WHEN 'ACTIVE' THEN 0 ELSE 1 END,sequence LIMIT 3",
        )?;
        Ok(statement
            .query_map(params![pool, through], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_pages_finish_captured_inventory_without_adopting_arrivals() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "pages",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let tx = store.db.transaction().unwrap();
        for sequence in 2..1026 {
            tx.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES(?1,?2,?3,?4,X'00','1000000','ALLOCATED')", params![format!("wallet-{:08}", 2048-sequence),pool,sequence,format!("0x{sequence:040x}")]).unwrap();
        }
        tx.commit().unwrap();
        let upper = store.wallet_boundary(&pool).unwrap();
        assert_eq!(upper, 1025);
        store.db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES('aaa-arrival',?1,1026,'arrival',X'00','1000000','ALLOCATED')",[&pool]).unwrap();
        store
            .db
            .execute(
                "UPDATE wallets SET role='RETIRED' WHERE pool_id=?1 AND sequence=65",
                [&pool],
            )
            .unwrap();
        let mut cursor = -1;
        let mut count = 0;
        loop {
            let page = store.wallet_page(&pool, cursor, upper, PAGE_SIZE).unwrap();
            assert!(page.len() <= PAGE_SIZE);
            if page.is_empty() {
                break;
            }
            for (sequence, id, _) in page {
                assert!(sequence > cursor && sequence <= upper);
                assert_ne!(id, "aaa-arrival");
                assert_ne!(sequence, 65);
                cursor = sequence;
                count += 1;
            }
        }
        assert_eq!(count, 1025);
        assert_eq!(store.wallet_boundary(&pool).unwrap(), 1026);
        assert_eq!(
            store.wallet_page(&pool, upper, 1026, PAGE_SIZE).unwrap()[0].1,
            "aaa-arrival"
        );
    }

    #[test]
    fn history_pages_ignore_uuid_order_and_preserve_unfinished_work_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let identity = store.id().to_owned();
        let pool = crate::rotation::store::test_support::declare_two_reserves(
            &mut store,
            "pages",
            "1",
            Some(alloy_primitives::U256::from(100_000_000u64)),
            0,
        )
        .unwrap();
        let wallet = store.wallet_page(&pool, -1, 1, 1).unwrap().remove(0).1;
        let tx = store.db.transaction().unwrap();
        for sequence in 1..=2048 {
            tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES(?1,?2,?3,0,'1','fixture','POSSIBLY_SUBMITTED','payer',?4,999999,?5)",params![format!("attempt-{:08}",4096-sequence),pool,wallet,format!("nonce-{sequence}"),sequence]).unwrap();
        }
        evidence::add_exposure(&tx, &wallet, U256::from(2048)).unwrap();
        tx.commit().unwrap();
        let upper = store.history_boundary(&pool).unwrap();
        let first = store.history_page(&pool, 0, upper, PAGE_SIZE).unwrap();
        assert_eq!(first.len(), PAGE_SIZE);
        assert_eq!(first[0].0, 1);
        assert_eq!(first[0].1.as_ref().unwrap().id, "attempt-00004095");
        let tx = store.db.transaction().unwrap();
        tx.execute("UPDATE payment_attempts SET state='RESOLVED' WHERE pool_id=?1 AND reconciliation_sequence=65",[&pool]).unwrap();
        evidence::subtract_exposure(&tx, &wallet, U256::from(1)).unwrap();
        tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,nonce,valid_before,reconciliation_sequence) VALUES('aaa-late-commit',?1,?2,0,'1','fixture','POSSIBLY_SUBMITTED','payer','late',999999,2049)",params![pool,wallet]).unwrap();
        evidence::add_exposure(&tx, &wallet, U256::from(1)).unwrap();
        tx.commit().unwrap();
        let mut cursor = first.last().unwrap().0;
        let mut count = first.len();
        loop {
            let page = store.history_page(&pool, cursor, upper, PAGE_SIZE).unwrap();
            assert!(page.len() <= PAGE_SIZE);
            if page.is_empty() {
                break;
            }
            for (sequence, authorization) in page {
                assert!(sequence > cursor && sequence <= upper);
                assert_ne!(sequence, 65);
                assert_ne!(authorization.unwrap().id, "aaa-late-commit");
                cursor = sequence;
                count += 1;
            }
        }
        assert_eq!(count, 2047);
        assert_eq!(
            evidence::exposure(&store.db, &wallet).unwrap(),
            U256::from(2048)
        );
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        assert_eq!(store.history_boundary(&pool).unwrap(), 2049);
        assert_eq!(
            store.history_page(&pool, 0, 2049, 1).unwrap()[0].0,
            1,
            "enumeration alone never releases liability; restart revisits unfinished work"
        );
        assert_eq!(
            store.history_page(&pool, 2048, 2049, PAGE_SIZE).unwrap()[0]
                .1
                .as_ref()
                .unwrap()
                .id,
            "aaa-late-commit"
        );
        store.db.execute("UPDATE payment_attempts SET valid_before=-1 WHERE reconciliation_sequence<=64 OR reconciliation_sequence IN (66,128,2048)", []).unwrap();
        let mut cursor = 0;
        let mut malformed = 0;
        let mut healthy = 0;
        loop {
            let page = store.history_page(&pool, cursor, 2049, PAGE_SIZE).unwrap();
            if page.is_empty() {
                break;
            }
            if cursor == 0 {
                assert!(page.iter().all(|(_, payload)| payload.is_err()));
            }
            for (sequence, payload) in page {
                assert!(sequence > cursor);
                cursor = sequence;
                match payload {
                    Ok(_) => healthy += 1,
                    Err(fault) => {
                        assert_eq!(fault.wallet, wallet);
                        assert!(!fault.id.is_empty());
                        malformed += 1;
                    }
                }
            }
        }
        assert_eq!((malformed, healthy), (67, 1981));
        assert_eq!(
            evidence::exposure(&store.db, &wallet).unwrap(),
            U256::from(2048)
        );
        drop(store);
        let store = Store::open(&state, &key, &identity).unwrap();
        assert!(store.history_page(&pool, 0, 2049, 1).unwrap()[0].1.is_err());
        assert_eq!(
            evidence::exposure(&store.db, &wallet).unwrap(),
            U256::from(2048)
        );
        // Ownership and storage failures must never masquerade as local payload faults.
        store.db.execute("PRAGMA foreign_keys=OFF", []).unwrap();
        store
            .db
            .execute(
                "UPDATE payment_attempts SET wallet_id='unknown' WHERE reconciliation_sequence=1",
                [],
            )
            .unwrap();
        assert!(
            store
                .history_page(&pool, 0, 2049, PAGE_SIZE)
                .err()
                .unwrap()
                .to_string()
                .contains("unknown_history_ownership")
        );
        store.db.execute("DROP TABLE payment_attempts", []).unwrap();
        assert!(store.history_page(&pool, 0, 2049, PAGE_SIZE).is_err());
    }
}
