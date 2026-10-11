//! Confirmed returned principal, keyed by outpoint; never inferred from API status.
use super::*;
#[derive(Clone, Debug, Serialize)]
pub struct RefundStatus {
    pub txid: String,
    pub output_index: u32,
    pub operation_id: String,
    pub amount: i64,
    pub height: i64,
}
pub(super) fn read_refunds(db: &Connection) -> Result<Vec<RefundStatus>> {
    Ok(db.prepare("SELECT txid,output_index,operation_id,amount,height FROM refund_outputs ORDER BY txid,output_index")?
        .query_map([], |r| Ok(RefundStatus { txid:r.get(0)?, output_index:r.get(1)?, operation_id:r.get(2)?, amount:r.get(3)?, height:r.get(4)? }))?
        .collect::<rusqlite::Result<_>>()?)
}
/// Derived from the original journals on each observation, never a second ledger.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(crate) enum ReturnAccountingIssue {
    MissingOriginalBinding { operation_id: String },
    AmbiguousOriginalBinding { operation_id: String },
}
/// Targeted source-return accounting, independent of destination lifecycle/status.
#[derive(Default)]
pub(crate) struct ReturnAccounting {
    pub bindings: std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    pub confirmed: std::collections::HashSet<String>,
    pub credited: Vec<RefundStatus>,
    pub ambiguous_operations: std::collections::HashSet<String>,
    pub issues: Vec<ReturnAccountingIssue>,
}
impl Store {
    pub(crate) fn return_accounting(&self) -> Result<ReturnAccounting> {
        let mut accounting = ReturnAccounting {
            confirmed: self
                .db
                .prepare("SELECT id FROM treasury_operations WHERE submission='CONFIRMED'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?,
            ..ReturnAccounting::default()
        };
        let mut missing_operations = std::collections::BTreeSet::new();
        // Recovery retains the original encrypted address under the original job's
        // authenticated encryption context, even after that job accepts a new quote.
        let mut query = self.db.prepare(
            "SELECT j.job_id,j.operation_id,r.address FROM funding_progress j LEFT JOIN funding_refunds r ON r.job_id=j.job_id UNION ALL SELECT job_id,operation_id,refund FROM funding_recovery",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })?;
        for row in rows {
            let (job, operation, bytes) = row?;
            let Some(bytes) = bytes else {
                if accounting.confirmed.contains(&operation) {
                    missing_operations.insert(operation);
                }
                continue;
            };
            let plain = unseal(
                &self.key,
                &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                &bytes,
            )?;
            let address = std::str::from_utf8(&plain)?.to_owned();
            accounting
                .bindings
                .entry(address)
                .or_default()
                .insert(operation);
        }
        let mut addresses_by_operation = std::collections::HashMap::<&String, usize>::new();
        for owners in accounting.bindings.values() {
            if owners.len() != 1 {
                accounting
                    .ambiguous_operations
                    .extend(owners.iter().cloned());
            }
            for operation in owners {
                *addresses_by_operation.entry(operation).or_default() += 1;
            }
        }
        accounting.ambiguous_operations.extend(
            addresses_by_operation
                .iter()
                .filter(|(_, count)| **count != 1)
                .map(|(operation, _)| (*operation).clone()),
        );
        accounting.credited = read_refunds(&self.db)?;
        // A credited operation can outlive every funding-progress row. Missing
        // original ownership must still be reported, including after archival loss.
        missing_operations.extend(
            accounting
                .credited
                .iter()
                .map(|credit| credit.operation_id.clone()),
        );
        accounting.issues.extend(
            missing_operations
                .into_iter()
                .filter(|operation| !addresses_by_operation.contains_key(operation))
                .map(|operation_id| ReturnAccountingIssue::MissingOriginalBinding { operation_id }),
        );
        accounting.issues.extend(
            accounting
                .ambiguous_operations
                .iter()
                .cloned()
                .map(
                    |operation_id| ReturnAccountingIssue::AmbiguousOriginalBinding { operation_id },
                ),
        );
        accounting.issues.sort();
        Ok(accounting)
    }
    pub fn refund_bindings(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .return_accounting()?
            .bindings
            .into_iter()
            .flat_map(|(address, operations)| {
                operations
                    .into_iter()
                    .map(move |operation| (operation, address.clone()))
            })
            .collect())
    }
    /// Called only by the synchronized treasury owner after checking confirmation
    /// depth. Repeated observations cannot credit the same output twice. Excess
    /// payments cannot refund fees or unrelated operation exposure.
    #[cfg(any(feature = "zcash", test))]
    pub(crate) fn record_refund(&mut self, refund: RefundStatus) -> Result<()> {
        let (submission, facts): (String, String) = self.db.query_row(
            "SELECT submission,facts FROM treasury_operations WHERE id=?1",
            [&refund.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let facts: TransactionFacts = serde_json::from_str(&facts)?;
        ensure!(submission == "CONFIRMED", "refund source not reconciled");
        let tx = self.db.transaction()?;
        let existing: Option<(String, i64)> = tx
            .query_row(
                "SELECT operation_id,amount FROM refund_outputs WHERE txid=?1 AND output_index=?2",
                params![refund.txid, refund.output_index],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((op, amount)) = existing {
            ensure!(
                op == refund.operation_id && amount == refund.amount,
                "refund conflict"
            );
        } else {
            let credited: i64 = tx.query_row(
                "SELECT COALESCE(SUM(credited),0) FROM refund_outputs WHERE operation_id=?1",
                [&refund.operation_id],
                |r| r.get(0),
            )?;
            ensure!(refund.amount > 0, "invalid refund value");
            let credit = refund
                .amount
                .min(i64::try_from(facts.amount_zatoshis)?.saturating_sub(credited));
            tx.execute(
                "INSERT INTO refund_outputs VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    refund.txid,
                    refund.output_index,
                    refund.operation_id,
                    refund.amount,
                    refund.height,
                    credit
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn return_bindings_keep_original_operation_after_archive_and_reopen() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut s = Store::create(&state, &key, 1, b"snapshot")?;
        s.ensure_pool("return_history", "5")?;
        let job = s.funding_jobs()?.remove(0);
        s.save_refund_address(&job.id, "original-address", 1, b"with-address")?;
        let quote = serde_json::to_vec(
            &serde_json::json!({"request":{"amount":job.target,"recipient":job.recipient}}),
        )?;
        s.save_funding_quote(&job.id, &quote)?;
        s.advance_funding(
            &job.id,
            funding::FundingPhase::Quoted,
            funding::FundingPhase::Preparing,
        )?;
        s.reserve(&job.operation_id, None, 1, 100, 1000)?;
        s.prepare_with_facts(
            &job.operation_id,
            2,
            b"prepared",
            b"signed",
            Some(TransactionFacts {
                txid: "original-source".into(),
                expiry_height: 20,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )?;
        s.confirm_spend(&job.operation_id, 100, 1)?;
        s.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote,refund) SELECT j.operation_id,j.job_id,j.phase,j.quote,r.address FROM funding_progress j JOIN funding_refunds r ON r.job_id=j.job_id WHERE j.job_id=?1", [&job.id])?;
        // Duplicate exact bindings are idempotent, not ambiguous.
        assert!(s.return_accounting()?.ambiguous_operations.is_empty());
        s.db.execute("DELETE FROM funding_refunds WHERE job_id=?1", [&job.id])?;
        s.db.execute(
            "UPDATE funding_progress SET operation_id='replacement' WHERE job_id=?1",
            [&job.id],
        )?;
        s.save_refund_address(&job.id, "replacement-address", 3, b"replacement")?;
        s.db.execute(
            "UPDATE funding_jobs SET state='COMPLETE' WHERE id=?1",
            [&job.id],
        )?;
        s.db.execute(
            "UPDATE wallets SET role='RETIRED' WHERE id=?1",
            [&job.wallet_id],
        )?;
        let id = s.id().to_owned();
        drop(s);
        let mut s = Store::open(&state, &key, &id)?;
        let accounting = s.return_accounting()?;
        assert_eq!(
            accounting.bindings["original-address"].first(),
            Some(&job.operation_id)
        );
        assert_eq!(
            accounting.bindings["replacement-address"]
                .first()
                .map(String::as_str),
            Some("replacement")
        );
        assert!(accounting.ambiguous_operations.is_empty());
        let late = RefundStatus {
            txid: "late-return".into(),
            output_index: 0,
            operation_id: job.operation_id.clone(),
            amount: 50,
            height: 30,
        };
        s.record_refund(late.clone())?;
        s.record_refund(late)?;
        let returned = s.return_accounting()?.credited;
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].operation_id, job.operation_id);
        assert_eq!(
            s.db.query_row(
                "SELECT consumed FROM budget_entries WHERE id=?1",
                [&job.operation_id],
                |r| r.get::<_, i64>(0)
            )?,
            100
        );
        Ok(())
    }

    #[test]
    fn refund_credit_is_idempotent_capped_and_does_not_refund_fees() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut s = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"snapshot",
        )?;
        let id = Uuid::new_v4().to_string();
        s.reserve(&id, None, 1, 100, 200)?;
        s.prepare_with_facts(
            &id,
            1,
            b"prepared",
            b"signed",
            Some(TransactionFacts {
                txid: "source".into(),
                expiry_height: 20,
                amount_zatoshis: 80,
                fee_zatoshis: 20,
                deadline: 1000,
            }),
        )?;
        let refund = RefundStatus {
            txid: "refund".into(),
            output_index: 0,
            operation_id: id.clone(),
            amount: 90,
            height: 10,
        };
        assert!(s.record_refund(refund.clone()).is_err());
        s.confirm_spend(&id, 100, 1)?;
        s.record_refund(refund.clone())?;
        s.record_refund(refund.clone())?;
        let mut conflict = refund.clone();
        conflict.amount += 1;
        assert!(s.record_refund(conflict).is_err());
        let mut unrelated = refund.clone();
        unrelated.operation_id = "unknown-operation".into();
        assert!(s.record_refund(unrelated).is_err());
        // Third-party overpayment cannot refund the fee or create negative cost.
        s.record_refund(RefundStatus {
            txid: "refund".into(),
            output_index: 1,
            operation_id: id.clone(),
            amount: 10,
            height: 10,
        })?;
        s.confirm_spend(&id, 100, 1)?; // gross accounting remains idempotent
        assert!(s.reserve("too_much", None, 1, 181, 200).is_err());
        s.reserve("allowed", None, 1, 180, 200)?;
        assert_eq!(s.status()?.refunds.len(), 2);
        assert_eq!(
            s.return_accounting()?.issues,
            vec![ReturnAccountingIssue::MissingOriginalBinding {
                operation_id: id.clone(),
            }],
            "credited operations without any current/archive row retain a typed obligation"
        );
        let treasury_id = s.id().to_owned();
        drop(s);
        let mut s = Store::open(
            &dir.path().join("state"),
            &dir.path().join("key"),
            &treasury_id,
        )?;
        s.record_refund(refund)?;
        assert_eq!(s.status()?.refunds.len(), 2);
        assert!(s.reserve("extra-budget", None, 1, 1, 200).is_err());
        Ok(())
    }
}
