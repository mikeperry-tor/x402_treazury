//! Runtime owners capture an upper boundary, then acquire bounded keyset pages.
//! No cursor is durable and no inventory size limits financial authority.
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

impl Store {
    pub(crate) fn history_boundary(&self, _pool: &str) -> Result<i64> {
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
    ) -> Result<Vec<(i64, PendingAuthorization)>> {
        let mut statement = self.db.prepare(
            "SELECT reconciliation_sequence,id,wallet_id,payer,nonce,valid_before FROM payment_attempts WHERE pool_id=?1 AND state='POSSIBLY_SUBMITTED' AND reconciliation_sequence>?2 AND reconciliation_sequence<=?3 ORDER BY reconciliation_sequence LIMIT ?4",
        )?;
        Ok(statement
            .query_map(params![pool, after, through, page_limit(limit)?], |row| {
                Ok((
                    row.get(0)?,
                    PendingAuthorization {
                        id: row.get(1)?,
                        wallet: row.get(2)?,
                        payer: row.get(3)?,
                        nonce: row.get(4)?,
                        valid_before: sql_u64(row, 5)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn wallet_boundary(&self, pool: &str) -> Result<i64> {
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
        let mut statement = self.db.prepare(
            "SELECT sequence,id,address FROM wallets WHERE pool_id=?1 AND role!='RETIRED' AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4",
        )?;
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
        let mut statement = self.db.prepare(
            "SELECT sequence,id,address FROM wallets WHERE pool_id=?1 AND role IN ('ACTIVE','READY') AND (role='ACTIVE' OR (role='READY' AND EXISTS(SELECT 1 FROM reserve_claims r WHERE r.pool_id=?1 AND r.wallet_id=wallets.id AND r.enabled=1))) AND sequence<=?2 ORDER BY CASE role WHEN 'ACTIVE' THEN 0 ELSE 1 END,sequence LIMIT 3",
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
        let pool = crate::rotation::store::test_support::declare_and_allocate(
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
        let pool = crate::rotation::store::test_support::declare_and_allocate(
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
        assert_eq!(first[0].1.id, "attempt-00004095");
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
                assert_ne!(authorization.id, "aaa-late-commit");
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
                .id,
            "aaa-late-commit"
        );
    }
}
