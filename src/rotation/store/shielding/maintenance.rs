//! Read-only maintenance inventory. Pages carry no submission or settlement authority.
//! Activation waits for operation-scoped canonical orphan/expiry accounting.
#![allow(dead_code)]
use super::*;

const PAGE_SIZE: i64 = 64;
pub(super) const CURRENT_ADDRESSES: &str =
    "SELECT job_id,job_id,address FROM funding_refunds WHERE job_id>?1 ORDER BY job_id LIMIT ?2";
pub(super) const ARCHIVED_ADDRESSES: &str = "SELECT operation_id,job_id,refund FROM funding_recovery WHERE operation_id>?1 ORDER BY operation_id LIMIT ?2";
pub(super) const PENDING_OPERATION: &str = "SELECT id FROM outgoing WHERE state='PREPARED' AND EXISTS(SELECT 1 FROM shielding_operations WHERE operation_id=outgoing.id)";

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct AddressCursor {
    archive: bool,
    key: String,
}

#[derive(Clone)]
pub(crate) struct AddressCandidate {
    pub(crate) job: String,
    pub(crate) address: String,
    pub(crate) recipient: String,
    pub(crate) max_fee: u64,
    cursor: AddressCursor,
    namespace: Uuid,
}

impl AddressCandidate {
    pub(crate) fn cursor(&self) -> AddressCursor {
        self.cursor.clone()
    }
}

pub(crate) struct AddressPage {
    pub(crate) next: Option<AddressCursor>,
    pub(crate) candidates: Vec<AddressCandidate>,
}

impl Store {
    pub(crate) fn shielding_inputs_available(&self, inputs: &[(String, u32)]) -> Result<bool> {
        for (txid, index) in inputs {
            let owned: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM shielding_inputs i WHERE i.txid=?1 AND i.output_index=?2 AND (NOT EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=i.operation_id) OR EXISTS(SELECT 1 FROM active_source_obligations o WHERE o.operation_id=i.operation_id)))", params![txid,index], |r| r.get(0))?;
            if owned {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Includes manual, disabled-profile, archived and already confirmed IDs.
    /// Observation remains required even without current spending capability.
    pub(crate) fn shielding_operation_page(&self, after: &str) -> Result<Vec<String>> {
        Ok(self.db.prepare("SELECT operation_id FROM shielding_operations WHERE operation_id>?1 ORDER BY operation_id LIMIT ?2")?
            .query_map(params![after, PAGE_SIZE], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Current policy is consulted independently of retained journal discovery.
    /// This read alone is never permission to publish broadcast intent.
    pub(crate) fn automatic_shielding_submission_candidate(
        &self,
        operation: &str,
        authority: &ServingAuthority,
    ) -> Result<bool> {
        if authority.namespace != self.evidence_namespace || authority.treasury != self.id {
            return Ok(false);
        }
        let automatic: Option<bool> = self
            .db
            .query_row(
                "SELECT automatic FROM shielding_operations WHERE operation_id=?1",
                [operation],
                |r| r.get(0),
            )
            .optional()?;
        if automatic != Some(true) {
            return Ok(false);
        }
        let status = self.operation(operation)?;
        if status.attempts != 0 || status.submission != "PREPARED" {
            return Ok(false);
        }
        let Some(binding) = self.owned_shielding_binding(operation, &status.facts)? else {
            return Ok(false);
        };
        if !binding.policy.automatic
            || !authority
                .pools
                .get(&binding.policy.pool)
                .is_some_and(|cap| status.facts.fee_zatoshis <= *cap)
        {
            return Ok(false);
        }
        self.require_current_shielding_policy(&binding)?;
        Ok(true)
    }

    /// Page original address journals, not provider claims, credits or active jobs.
    /// A recovered/complete job's old return address remains eligible inventory.
    pub(crate) fn shielding_address_page(
        &self,
        authority: &ServingAuthority,
        after: &AddressCursor,
    ) -> Result<AddressPage> {
        ensure!(
            authority.namespace == self.evidence_namespace && authority.treasury == self.id,
            "shielding serving owner mismatch"
        );
        let sql = if after.archive {
            ARCHIVED_ADDRESSES
        } else {
            CURRENT_ADDRESSES
        };
        let rows = self
            .db
            .prepare(sql)?
            .query_map(params![after.key, PAGE_SIZE], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() && !after.archive {
            return self.shielding_address_page(
                authority,
                &AddressCursor {
                    archive: true,
                    key: String::new(),
                },
            );
        }
        let mut candidates = Vec::new();
        let mut next = None;
        for (key, job, bytes) in rows {
            let cursor = AddressCursor {
                archive: after.archive,
                key,
            };
            next = Some(cursor.clone());
            // Page physical history before filtering absent addresses, so a long
            // sparse prefix cannot monopolize the serialized Store worker.
            let Some(bytes) = bytes else { continue };
            let funding = self
                .funding_job(&job)?
                .context("shielding address owner missing")?;
            let pool: String = self.db.query_row(
                "SELECT pool_id FROM funding_jobs WHERE id=?1",
                [&job],
                |r| r.get(0),
            )?;
            let Some(max_fee) = authority.pools.get(&pool) else {
                continue;
            };
            let enabled: bool =
                self.db
                    .query_row("SELECT enabled FROM pools WHERE id=?1", [&pool], |r| {
                        r.get(0)
                    })?;
            if !enabled {
                continue;
            }
            let plain = unseal(
                &self.key,
                &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                &bytes,
            )?;
            candidates.push(AddressCandidate {
                job,
                address: std::str::from_utf8(&plain)?.into(),
                recipient: funding.recipient,
                max_fee: *max_fee,
                cursor,
                namespace: self.evidence_namespace,
            });
        }
        Ok(AddressPage { next, candidates })
    }

    /// Recheck the exact original journal entry before crossing the C16 boundary.
    pub(crate) fn begin_shielding_candidate(
        &mut self,
        operation: &str,
        revision: i64,
        candidate: &AddressCandidate,
        authority: &ServingAuthority,
    ) -> Result<(preparation::Preparation, Policy)> {
        ensure!(
            candidate.namespace == self.evidence_namespace,
            "shielding serving owner mismatch"
        );
        let (job, bytes): (String, Vec<u8>) = if candidate.cursor.archive {
            self.db.query_row(
                "SELECT job_id,refund FROM funding_recovery WHERE operation_id=?1",
                [&candidate.cursor.key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        } else {
            self.db.query_row(
                "SELECT job_id,address FROM funding_refunds WHERE job_id=?1",
                [&candidate.cursor.key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        };
        let plain = unseal(
            &self.key,
            &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
            &bytes,
        )?;
        ensure!(
            job == candidate.job && plain.as_slice() == candidate.address.as_bytes(),
            "shielding address binding changed"
        );
        self.begin_bound_shielding(
            operation,
            revision,
            &job,
            &candidate.address,
            PolicySource::Serving(authority),
        )
    }
}
