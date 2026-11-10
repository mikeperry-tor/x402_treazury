//! Snapshot-derived refund visibility; never a balance or spending ledger.
use super::*;

impl Store {
    pub(crate) fn funding_refund_observed(&self, operation: &str) -> Result<bool> {
        let Some((observed_revision, true)) = self.refund_observations.get(operation) else {
            return Ok(false);
        };
        let revision: i64 = self
            .db
            .query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        Ok(*observed_revision == revision)
    }

    /// Current originals only. Archived terminal refunds never become watchers.
    pub(crate) fn funding_refund_page(&self, after: &str) -> Result<Vec<(String, String, String)>> {
        Ok(self.db.prepare("SELECT j.job_id,j.operation_id,p.name FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN pools p ON p.id=f.pool_id WHERE j.operation_id>?1 AND j.quote IS NOT NULL AND (EXISTS(SELECT 1 FROM outgoing x WHERE x.id=j.operation_id) OR EXISTS(SELECT 1 FROM treasury_operations t WHERE t.id=j.operation_id)) AND NOT(typeof(j.provider_success)='integer' AND j.provider_success!=0) AND ((typeof(j.provider_failed)='integer' AND j.provider_failed!=0) OR (typeof(j.provider_refunded)='integer' AND j.provider_refunded!=0)) ORDER BY j.operation_id LIMIT ?2")?
            .query_map(params![after, history::PAGE_SIZE as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn funding_refund_scan_needed(&self) -> Result<bool> {
        let mut after = String::new();
        loop {
            let page = self.funding_refund_page(&after)?;
            if page.is_empty() {
                return Ok(false);
            }
            for (_, operation, _) in page {
                if !self.funding_refund_observed(&operation)? {
                    return Ok(true);
                }
                after = operation;
            }
        }
    }

    pub(crate) fn funding_refund_receiver(
        &self,
        job: &str,
        operation: &str,
        revision: i64,
    ) -> Result<Option<String>> {
        if self
            .refund_observations
            .get(operation)
            .is_some_and(|(seen, _)| *seen == revision)
        {
            return Ok(None);
        }
        let quote: super::super::near::Quote =
            serde_json::from_slice(&self.funding_quote_for_operation(job, operation)?)?;
        Ok(Some(
            quote.request["refundTo"]
                .as_str()
                .context("saved refund receiver missing")?
                .to_owned(),
        ))
    }

    /// Called only by the SDK owner after inspecting the persisted snapshot.
    pub(crate) fn observe_funding_refund(
        &mut self,
        job: &str,
        operation: &str,
        revision: i64,
        received: bool,
    ) -> Result<bool> {
        let current: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM funding_progress WHERE job_id=?1 AND operation_id=?2) AND (SELECT MAX(revision) FROM snapshots)=?3",params![job,operation,revision],|r|r.get(0))?;
        if !current {
            return Ok(false);
        }
        let changed = self
            .refund_observations
            .get(operation)
            .is_none_or(|(_, previous)| *previous != received);
        self.refund_observations
            .insert(operation.to_owned(), (revision, received));
        Ok(changed)
    }
}
