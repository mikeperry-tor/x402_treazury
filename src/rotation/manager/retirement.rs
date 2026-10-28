//! Candidate-scoped canonical work, retained through disconnected waiters.
use super::*;
use crate::rotation::base::evidence::EvidenceFailure;

#[derive(Clone, Copy)]
enum Progress {
    Complete,
    Pending(&'static str),
}

impl ManagedPool {
    /// One finite attempt. The Store owns demand and completion independently of
    /// the maintenance task and disposable balance-certificate cache.
    pub(crate) async fn retire_wallet(&self, wallet: String) -> Result<bool> {
        let pool = self.clone();
        tokio::spawn(async move {
            let owner = pool.store.balance_acquisition_owner(&wallet).await;
            let _observation = owner.lock_owned().await;
            let (group, id) = (pool.pool.clone(), wallet.clone());
            if !pool
                .store
                .call(move |s| s.begin_retirement(&group, &id))
                .await?
            {
                let id = wallet.clone();
                return pool
                    .store
                    .call(move |s| Ok(!s.retirement_requested(&id)))
                    .await;
            }
            let result = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
                pool.retirement_proofs(&wallet),
            ))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("retirement proof owner interrupted")));
            let (retry, blocker) = match &result {
                Ok(Progress::Complete) => (false, None),
                Ok(Progress::Pending(reason)) => (true, Some(*reason)),
                Err(error) if operator_required(error) => {
                    (false, Some("stored_records_require_operator"))
                }
                Err(_) => (true, Some("canonical_observation_unavailable")),
            };
            let failed = result.is_err() && retry;
            let cause = result
                .as_ref()
                .err()
                .map(super::super::base::safe_diagnostic);
            let (group, id) = (pool.pool.clone(), wallet.clone());
            pool.store
                .call(move |s| {
                    s.finish_retirement(&group, &id, retry, failed, blocker, cause);
                    Ok(())
                })
                .await?;
            match result {
                Ok(_) => Ok(!retry),
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| anyhow::Error::new(error).context("wallet retirement owner stopped"))
        .unwrap_or_else(Err)
    }

    async fn retirement_proofs(&self, wallet: &str) -> Result<Progress> {
        let (pool, id) = (self.pool.clone(), wallet.to_owned());
        let through = self
            .store
            .call(move |s| s.retirement_pending_boundary(&pool, &id))
            .await?;
        let mut after = 0;
        loop {
            let (pool, id) = (self.pool.clone(), wallet.to_owned());
            let query = self
                .store
                .call(move |s| {
                    if !s.retirement_eligible(&pool, &id)? {
                        return Ok(None);
                    }
                    let page = s.retirement_pending_page(&pool, &id, after, through)?;
                    let Some((last, _)) = page.last() else {
                        return Ok(None);
                    };
                    Ok(Some((
                        *last,
                        s.scoped_chain_query(
                            &pool,
                            vec![],
                            page.into_iter().map(|(_, auth)| auth).collect(),
                        )?,
                    )))
                })
                .await?;
            let Some((last, query)) = query else { break };
            after = last;
            let view = self.acquire_view(query).await?;
            let pool = self.pool.clone();
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            self.store
                .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                .await?;
        }
        let (pool, id) = (self.pool.clone(), wallet.to_owned());
        let query = self
            .store
            .call(move |s| {
                if !s.retirement_eligible(&pool, &id)? {
                    return Ok(None);
                }
                Ok(Some(s.scoped_chain_query(&pool, vec![id], vec![])?))
            })
            .await?;
        let Some(query) = query else {
            return Ok(Progress::Complete);
        };
        let (pool, id) = (self.pool.clone(), wallet.to_owned());
        let cached = self
            .store
            .call(move |s| s.retirement_cached_view(&pool, &id))
            .await?;
        let view = match cached {
            Some(view) => view,
            None => self.acquire_view(query).await?,
        };
        let (pool, id) = (self.pool.clone(), wallet.to_owned());
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        self.store
            .call_with_guard(guard, move |s| {
                if !s.retirement_eligible(&pool, &id)? {
                    return Ok(Progress::Complete);
                }
                // A newly published scalar is newer demand, not something this
                // captured proof may overwrite. Its independent request survives.
                if s.retirement_requested(&id) {
                    return Ok(Progress::Pending("new_observation"));
                }
                let published = s.publish_scoped_view(&pool, view)?;
                let latest = published.observations[&id].latest.amount;
                let (role, tolerance) = s.db_retirement_status(&pool, &id)?;
                if role == "RETIRED" || latest > tolerance {
                    Ok(Progress::Complete)
                } else {
                    Ok(Progress::Pending("confirmation_or_payment_authorization"))
                }
            })
            .await
    }
}

fn operator_required(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<EvidenceFailure>(),
        Some(
            EvidenceFailure::MalformedSupport
                | EvidenceFailure::MalformedExposure
                | EvidenceFailure::ResolutionConflict
                | EvidenceFailure::ChainRepairRequired
                | EvidenceFailure::EvidenceRepairRequired
        )
    ) || matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(
            rusqlite::Error::InvalidColumnType(..)
                | rusqlite::Error::IntegralValueOutOfRange(..)
                | rusqlite::Error::FromSqlConversionFailure(..)
        )
    )
}

#[cfg(test)]
mod tests;
