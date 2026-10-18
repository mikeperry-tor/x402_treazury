//! Existing-wallet cursor paging and exact authorization report bindings.
use super::*;
use crate::rotation::base::PendingAuthorization;
pub(crate) const PAGE_SIZE: usize = 64;
pub(super) const WALLET_PAGE_SQL: &str = "SELECT sequence,id,address FROM wallets WHERE pool_id=?1 AND role!='RETIRED' AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4";
fn page_limit(limit: usize) -> Result<i64> {
    ensure!(
        (1..=PAGE_SIZE).contains(&limit),
        "invalid internal reconciliation page size"
    );
    Ok(limit as i64)
}
impl Store {
    /// Explicit qualification reporting only. This never supplies payment capacity.
    pub(crate) fn qualification_pending_page(
        &self,
        pool: &str,
        after: i64,
        through: i64,
    ) -> Result<Vec<(i64, Result<PendingAuthorization>)>> {
        Ok(self.db.prepare("SELECT reconciliation_sequence,id,wallet_id,payer,nonce,valid_before FROM payment_attempts INDEXED BY payment_pending_sequence WHERE pool_id=?1 AND state='POSSIBLY_SUBMITTED' AND reconciliation_sequence>?2 AND reconciliation_sequence<=?3 ORDER BY reconciliation_sequence LIMIT ?4")?
            .query_map(params![pool,after,through,PAGE_SIZE as i64], |r| {
                let parsed = (|| -> Result<_> {
                    let auth=PendingAuthorization {id:r.get(1)?,wallet:r.get(2)?,payer:r.get(3)?,nonce:r.get(4)?,valid_before:sql_u64(r,5)?};
                    let _:alloy_primitives::Address=auth.payer.parse()?;
                    let _:alloy_primitives::B256=auth.nonce.parse()?;
                    Ok(auth)
                })();
                Ok((r.get(0)?, parsed.map_err(|_| anyhow::anyhow!("malformed_qualification_authorization_row"))))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn qualification_pending_boundary(&self, pool: &str) -> Result<i64> {
        Ok(self.db.query_row(
            "SELECT COALESCE(MAX(reconciliation_sequence),0) FROM payment_attempts WHERE pool_id=?1 AND state='POSSIBLY_SUBMITTED'",
            [pool],
            |r| r.get(0),
        )?)
    }
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
}
