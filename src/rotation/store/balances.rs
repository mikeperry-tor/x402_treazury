//! Advisory observations and scheduling flags. None grants payment authority.
use super::*;
#[derive(Debug)]
pub(crate) struct AdvisoryIdentityUnavailable;
impl std::fmt::Display for AdvisoryIdentityUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("advisory_wallet_identity_unavailable")
    }
}
impl std::error::Error for AdvisoryIdentityUnavailable {}
impl Store {
    pub(crate) fn low_balance(&mut self, pool: &str, wallet: &str) -> Result<()> {
        ensure!(
            self.db.execute(
                "UPDATE wallets SET low_balance=1,refresh_pending=1 WHERE pool_id=?1 AND id=?2",
                params![pool, wallet]
            )? == 1,
            "missing payment wallet"
        );
        self.notify_capacity_changed(pool);
        Ok(())
    }
    pub(crate) fn pending_balances(
        &self,
        pool: &str,
        after: i64,
    ) -> Result<Vec<(i64, Option<String>)>> {
        let mut query = self.db.prepare("SELECT sequence,id FROM wallets WHERE pool_id=?1 AND role!='RETIRED' AND (refresh_pending=1 OR low_balance=1) AND sequence>?2 ORDER BY sequence LIMIT 64")?;
        Ok(query
            .query_map(params![pool, after], |row| {
                Ok((
                    row.get(0)?,
                    row.get_ref(1)?.as_str().ok().map(str::to_owned),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Paging never decodes another wallet's address or estimate. A bad row can
    /// advance by sequence while its accepted financial records remain untouched.
    pub(crate) fn advisory_wallet_page(
        &self,
        pool: &str,
        after: i64,
        through: i64,
    ) -> Result<Vec<(i64, Option<String>)>> {
        #[cfg(test)]
        self.inventory_read_fault(super::inventory::Read::WalletPage)?;
        let mut query=self.db.prepare("SELECT sequence,id FROM wallets WHERE pool_id=?1 AND (role IN ('ACTIVE','RETAINED','READY') OR (role='ALLOCATED' AND EXISTS(SELECT 1 FROM funding_jobs f WHERE f.wallet_id=wallets.id AND f.readiness_requirement IS NULL))) AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4")?;
        Ok(query
            .query_map(
                params![pool, after, through, super::history::PAGE_SIZE as i64],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get_ref(1)?.as_str().ok().map(str::to_owned),
                    ))
                },
            )?
            .collect::<rusqlite::Result<_>>()?)
    }
    /// Clear only the pending request when this read starts. A failure during the
    /// network read sets it again, requiring a later read without a revision layer.
    pub(crate) fn begin_balance(&mut self, pool: &str, wallet: &str) -> Result<String> {
        let tx = self.db.transaction()?;
        let address: Option<String> = tx
            .query_row(
                "SELECT address FROM wallets WHERE pool_id=?1 AND id=?2",
                params![pool, wallet],
                |row| Ok(row.get_ref(0)?.as_str().ok().map(str::to_owned)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    anyhow::Error::new(AdvisoryIdentityUnavailable)
                }
                other => other.into(),
            })?;
        let address = address.context(AdvisoryIdentityUnavailable)?;
        tx.execute("UPDATE wallets SET refresh_pending=0 WHERE id=?1", [wallet])?;
        tx.commit()?;
        Ok(address)
    }
    pub(crate) fn finish_balance(
        &mut self,
        pool: &str,
        wallet: &str,
        result: Option<U256>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        match result {
            Some(value) => {
                tx.execute("UPDATE wallets SET balance=?3,low_balance=refresh_pending,balance_refresh_error=NULL WHERE pool_id=?1 AND id=?2",params![pool,wallet,value.to_string()])?;
                let initial_claim: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM funding_jobs WHERE wallet_id=?1)",
                    [wallet],
                    |r| r.get(0),
                )?;
                if initial_claim && super::funding::record_initial_credit(&tx, wallet, value)? {
                    super::funding::publish_initial_credit(&tx, wallet)?;
                }
            }
            None => {
                tx.execute("UPDATE wallets SET balance_refresh_error='observation_unavailable' WHERE pool_id=?1 AND id=?2",params![pool,wallet])?;
            }
        }
        tx.commit()?;
        self.notify_capacity_changed(pool);
        Ok(())
    }
}
