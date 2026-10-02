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
impl Store {
    pub fn refund_bindings(&self) -> Result<Vec<(String, String)>> {
        let ids = self.db.prepare("SELECT j.job_id,j.operation_id FROM funding_progress j JOIN funding_refunds r ON r.job_id=j.job_id")?
            .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|(job, operation)| {
                Ok((
                    operation,
                    self.refund_address(&job)?
                        .context("refund binding missing")?,
                ))
            })
            .collect()
    }
    /// Called only by the synchronized treasury owner after checking confirmation
    /// depth. Repeated observations cannot credit the same output twice. Excess
    /// payments cannot refund fees or unrelated operation exposure.
    #[cfg(any(feature = "zcash", test))]
    pub(crate) fn record_refund(&mut self, refund: RefundStatus) -> Result<()> {
        let facts = self.operation(&refund.operation_id)?;
        ensure!(
            facts.submission == "CONFIRMED",
            "refund source not reconciled"
        );
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
                .min(i64::try_from(facts.facts.amount_zatoshis)?.saturating_sub(credited));
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
        s.record_refund(refund)?;
        // Third-party overpayment cannot refund the fee or create negative cost.
        s.record_refund(RefundStatus {
            txid: "extra".into(),
            output_index: 0,
            operation_id: id.clone(),
            amount: 10,
            height: 10,
        })?;
        s.confirm_spend(&id, 100, 1)?; // gross accounting remains idempotent
        assert!(s.reserve("too_much", None, 1, 181, 200).is_err());
        s.reserve("allowed", None, 1, 180, 200)?;
        assert_eq!(s.status()?.refunds.len(), 2);
        Ok(())
    }
}
