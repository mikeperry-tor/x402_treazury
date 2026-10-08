//! USDC allocation budgets reuse the durable source journal and encrypted quotes.
//! No new ledger may forget old spending when a configuration changes or restarts.
use super::*;
use crate::rotation::{config::FundingBudgetLimits, near::Quote};

impl Store {
    /// Runs in the serialized store worker immediately before the ZEC reservation.
    /// Gross allocations are never replenished by refunds. Unresolved allocations
    /// count against every day's allowance until canonical credit/refund resolution.
    pub fn check_funding_allocation(
        &self,
        operation: &str,
        day: u32,
        limits: FundingBudgetLimits,
    ) -> Result<()> {
        let (job, quote): (String, Vec<u8>) = self
            .db
            .query_row(
                "SELECT job_id,quote FROM funding_progress WHERE operation_id=?1",
                [operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .context("funding_allocation_evidence_missing")?;
        let proposed = self.allocation_quote(&job, &quote)?;
        let proposed_amount = allocation_amount(&proposed)?;
        let mut daily = 0u64;
        let mut total = 0u64;
        let mut already_reserved = false;
        // Include archived immutable operation bindings, never today's pool target.
        let mut stmt = self.db.prepare(
            "SELECT b.id,b.day,b.reserved,COALESCE(p.job_id,r.job_id),COALESCE(p.quote,r.quote),f.state,
             COALESCE((SELECT SUM(credited) FROM refund_outputs o WHERE o.operation_id=b.id),0)
             FROM budget_entries b
             LEFT JOIN funding_progress p ON p.operation_id=b.id
             LEFT JOIN funding_recovery r ON r.operation_id=b.id
             LEFT JOIN funding_jobs f ON f.id=COALESCE(p.job_id,r.job_id)
             WHERE b.pool_id IS NOT NULL AND (b.reserved>0 OR b.consumed>0)",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let charged_day: u32 = row.get(1)?;
            let reserved: i64 = row.get(2)?;
            let job: Option<String> = row.get(3)?;
            let bytes: Option<Vec<u8>> = row.get(4)?;
            let state: Option<String> = row.get(5)?;
            let refunded = u64::try_from(row.get::<_, i64>(6)?)?;
            let quote = self.allocation_quote(
                &job.context("funding_allocation_evidence_missing")?,
                &bytes.context("funding_allocation_evidence_missing")?,
            )?;
            let amount = allocation_amount(&quote)?;
            total = total
                .checked_add(amount)
                .context("funding allocation total overflow")?;
            if charged_day == day
                || reserved > 0
                || (state.as_deref() != Some("COMPLETE") && refunded < quote.input)
            {
                daily = daily
                    .checked_add(amount)
                    .context("funding allocation daily overflow")?;
            }
            if id == operation {
                ensure!(
                    amount == proposed_amount,
                    "funding allocation binding mismatch"
                );
                already_reserved = true;
            }
        }
        let additional = if already_reserved { 0 } else { proposed_amount };
        for (used, limit, category, setting) in [
            (
                daily,
                limits.daily,
                "daily_funding_limit_exceeded",
                "daily_funding_limit_usdc",
            ),
            (
                total,
                limits.total,
                "total_funding_limit_exceeded",
                "total_funding_limit_usdc",
            ),
        ] {
            if let Some(limit) = limit
                && used.checked_add(additional).is_none_or(|n| n > limit)
            {
                tracing::warn!(
                    used_micro_usdc = used,
                    requested_micro_usdc = additional,
                    limit_micro_usdc = limit,
                    setting,
                    "USDC funding allocation limit exceeded; new funding paused"
                );
                anyhow::bail!(category);
            }
        }
        Ok(())
    }

    fn allocation_quote(&self, job: &str, bytes: &[u8]) -> Result<Quote> {
        let bytes = unseal(
            &self.key,
            &format!("v1:{}:{}:funding:{job}", self.id, self.network.name()),
            bytes,
        )?;
        serde_json::from_slice(&bytes).context("funding_allocation_evidence_invalid")
    }
}

fn allocation_amount(quote: &Quote) -> Result<u64> {
    let amount_text = quote.request["amount"]
        .as_str()
        .context("funding_allocation_evidence_invalid")?;
    ensure!(
        !amount_text.is_empty() && amount_text.bytes().all(|b| b.is_ascii_digit()),
        "funding_allocation_evidence_invalid"
    );
    let amount: u64 = amount_text
        .parse()
        .context("funding_allocation_evidence_invalid")?;
    ensure!(
        amount > 0
            && quote.input > 0
            && amount <= i64::MAX as u64
            && quote.request["swapType"] == "EXACT_OUTPUT"
            && quote.request["destinationAsset"] == crate::rotation::near::USDC
            && quote.response["quote"]["amountIn"]
                .as_str()
                .and_then(|v| v.parse::<u64>().ok())
                == Some(quote.input)
            && quote.response["quote"]["amountOut"] == quote.request["amount"],
        "funding_allocation_evidence_invalid"
    );
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::store::funding::{FundingJob, FundingPhase};
    use serde_json::json;
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn quoted(store: &mut Store, name: &str, amount: &str) -> FundingJob {
        let pool = store.ensure_pool(name, amount).unwrap();
        let job = store
            .funding_jobs()
            .unwrap()
            .into_iter()
            .find(|j| j.pool_id == pool)
            .unwrap();
        let quote = Quote {
            request: json!({"amount":job.target,"swapType":"EXACT_OUTPUT","destinationAsset":crate::rotation::near::USDC}),
            response: json!({"quote":{"amountIn":"100","amountOut":job.target}}),
            input: 100,
            deadline: u64::MAX,
            deposit: Some("fixture".into()),
        };
        store
            .save_funding_quote(&job.id, &serde_json::to_vec(&quote).unwrap())
            .unwrap();
        job
    }

    fn reserve(
        store: &mut Store,
        job: &FundingJob,
        day: u32,
        limits: FundingBudgetLimits,
    ) -> Result<()> {
        store.check_funding_allocation(&job.operation_id, day, limits)?;
        store.reserve(&job.operation_id, Some(&job.pool_id), day, 110, 10_000)
    }

    fn confirm(store: &mut Store, job: &FundingJob, day: u32) {
        store
            .advance_funding(&job.id, FundingPhase::Quoted, FundingPhase::Preparing)
            .unwrap();
        let revision = store.status().unwrap().snapshot_revision;
        store
            .prepare_with_facts(
                &job.operation_id,
                revision,
                b"snapshot",
                b"signed fixture",
                Some(crate::rotation::transaction::TransactionFacts {
                    txid: "fixture".into(),
                    amount_zatoshis: 100,
                    fee_zatoshis: 10,
                    expiry_height: 999,
                    deadline: u64::MAX,
                }),
            )
            .unwrap();
        store.confirm_spend(&job.operation_id, 110, day).unwrap();
    }

    #[test]
    fn allocations_survive_rollover_restart_and_pool_changes() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture").unwrap();
        let first = quoted(&mut store, "first", "2");
        let second = quoted(&mut store, "second", "2");
        let limits = FundingBudgetLimits {
            daily: Some(3_000_000),
            total: Some(3_000_000),
        };
        reserve(&mut store, &first, 1, limits).unwrap();
        let logs = Logs::default();
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let error = tracing::subscriber::with_default(subscriber, || {
            reserve(&mut store, &second, 1, limits).unwrap_err()
        });
        assert_eq!(error.to_string(), "daily_funding_limit_exceeded");
        let output = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("limit_micro_usdc=3000000"));
        assert!(output.contains("daily_funding_limit_usdc"));
        // Rechecking the same immutable operation does not double charge it.
        store
            .check_funding_allocation(&first.operation_id, 1, limits)
            .unwrap();
        for day in [1, 2] {
            assert_eq!(
                reserve(&mut store, &second, day, limits)
                    .unwrap_err()
                    .to_string(),
                "daily_funding_limit_exceeded"
            );
        }
        confirm(&mut store, &first, 2);
        // Source confirmation alone is not destination credit.
        assert!(reserve(&mut store, &second, 3, limits).is_err());
        store
            .record_credit(&first.wallet_id, &first.target, "block", 1)
            .unwrap();
        store.disable_pool("first").unwrap();
        store.ensure_pool("first", "9").unwrap();
        // Historical operations keep their own authenticated quote even when the
        // current job binding has advanced. A new pool target cannot rewrite cost.
        store.db.execute("INSERT INTO funding_recovery SELECT operation_id,job_id,phase,quote,NULL FROM funding_progress WHERE job_id=?1", [&first.id]).unwrap();
        store
            .db
            .execute(
                "UPDATE funding_progress SET operation_id=?2,quote=NULL WHERE job_id=?1",
                params![first.id, Uuid::new_v4().to_string()],
            )
            .unwrap();
        let id = store.id.clone();
        drop(store);
        let mut store = Store::open(&state, &key, &id).unwrap();
        assert_eq!(
            reserve(&mut store, &second, 3, limits)
                .unwrap_err()
                .to_string(),
            "total_funding_limit_exceeded"
        );
        reserve(
            &mut store,
            &second,
            3,
            FundingBudgetLimits {
                total: Some(4_000_000),
                ..limits
            },
        )
        .unwrap();
    }

    #[test]
    fn canonical_refunds_end_daily_pending_but_do_not_replenish_total() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let first = quoted(&mut store, "first", "2");
        let next = quoted(&mut store, "next", "2");
        let limits = FundingBudgetLimits {
            daily: Some(2_000_000),
            total: Some(2_000_000),
        };
        reserve(&mut store, &first, 1, limits).unwrap();
        confirm(&mut store, &first, 1);
        store
            .record_refund(refunds::RefundStatus {
                txid: "refund".into(),
                output_index: 0,
                operation_id: first.operation_id.clone(),
                amount: 90,
                height: 2,
            })
            .unwrap();
        assert_eq!(
            reserve(&mut store, &next, 2, limits)
                .unwrap_err()
                .to_string(),
            "daily_funding_limit_exceeded"
        );
        store
            .record_refund(refunds::RefundStatus {
                txid: "refund".into(),
                output_index: 1,
                operation_id: first.operation_id,
                amount: 10,
                height: 2,
            })
            .unwrap();
        assert_eq!(
            reserve(&mut store, &next, 2, limits)
                .unwrap_err()
                .to_string(),
            "total_funding_limit_exceeded"
        );
        reserve(
            &mut store,
            &next,
            2,
            FundingBudgetLimits {
                total: Some(4_000_000),
                ..limits
            },
        )
        .unwrap();
    }

    #[test]
    fn unprepared_recovery_releases_only_unused_allocation_and_missing_history_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let first = quoted(&mut store, "first", "2");
        let next = quoted(&mut store, "next", "2");
        let limits = FundingBudgetLimits {
            daily: Some(2_000_000),
            total: Some(2_000_000),
        };
        reserve(&mut store, &first, 1, limits).unwrap();
        store.refresh_unprepared_quote(&first.id).unwrap();
        reserve(&mut store, &next, 1, limits).unwrap();
        let third = quoted(&mut store, "third", "2");
        // Simulate incomplete historical accounting, not an empty wallet.
        store
            .db
            .execute(
                "UPDATE funding_progress SET quote=NULL WHERE job_id=?1",
                [&next.id],
            )
            .unwrap();
        assert_eq!(
            store
                .check_funding_allocation(&third.operation_id, 2, limits)
                .unwrap_err()
                .to_string(),
            "funding_allocation_evidence_missing"
        );
    }

    #[tokio::test]
    async fn concurrent_preparations_share_one_allowance() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let first = quoted(&mut store, "first", "2");
        let second = quoted(&mut store, "second", "2");
        let (store, worker) = StoreHandle::spawn(store);
        let limits = FundingBudgetLimits {
            daily: Some(2_000_000),
            total: Some(2_000_000),
        };
        let (a, b) = tokio::join!(
            store.call(move |s| reserve(s, &first, 1, limits)),
            store.call(move |s| reserve(s, &second, 1, limits)),
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        drop(store);
        worker.await.unwrap();
    }
}
