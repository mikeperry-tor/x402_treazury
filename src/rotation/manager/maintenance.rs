//! One retained observation owner per Store/group; historical proofs stay wallet scoped.
use super::*;
use crate::rotation::store::{history::PAGE_SIZE, inventory::read_result};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
const BALANCE_READS: usize = 4;
const CADENCE: Duration = Duration::from_secs(5);
#[derive(Debug)]
pub(crate) struct ReconciliationInProgress;
impl std::fmt::Display for ReconciliationInProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reconciliation_already_in_progress")
    }
}
impl std::error::Error for ReconciliationInProgress {}
#[derive(Debug)]
struct RetirementObservation;
impl std::fmt::Display for RetirementObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("wallet retirement observation unavailable")
    }
}
impl std::error::Error for RetirementObservation {}
struct Completion {
    id: Option<String>,
    result: Result<()>,
}
impl ManagedPool {
    pub async fn reconcile(&self) -> Result<()> {
        let owner = self
            .reconciliation
            .clone()
            .try_lock_owned()
            .map_err(|_| ReconciliationInProgress)?;
        let pool = self.clone();
        tokio::spawn(async move {
            let _owner = owner;
            pool.maintenance(CancellationToken::new(), true).await
        })
        .await
        .context("reconciliation owner stopped")?
    }
    pub(crate) async fn run_reconciliation(&self, stop: CancellationToken) -> Result<()> {
        loop {
            if stop.is_cancelled() {
                return Ok(());
            }
            let pool = self.clone();
            let stopped = stop.clone();
            let owner = tokio::spawn(async move {
                let _owner = tokio::select! { _=stopped.cancelled()=>return Ok(()), owner=pool.reconciliation.clone().lock_owned()=>owner };
                pool.maintenance(stopped, false).await
            });
            match owner.await {
                Ok(result) => return result,
                Err(_) if stop.is_cancelled() => return Ok(()),
                Err(_) => tracing::warn!(
                    category = "reconciliation_owner_restart_pending",
                    "reconciliation owner exited; retrying existing ownership; payment journals remain unchanged"
                ),
            }
            tokio::select! { _=stop.cancelled()=>return Ok(()), _=tokio::time::sleep(CADENCE)=>{} }
        }
    }
    async fn maintenance(&self, stop: CancellationToken, one_pass: bool) -> Result<()> {
        #[cfg(test)]
        if let Some(hook) = &self.reconciliation_test_hook {
            hook();
        }
        let pool = self.pool.clone();
        let changes = self
            .store
            .call(move |s| Ok(s.capacity_changes(&pool)))
            .await?;
        // Capture once: new allocations never extend this startup/inspection pass.
        let pool = self.pool.clone();
        let mut first_error = None;
        let boundary = read_result(
            self.store.call(move |s| s.wallet_boundary(&pool)).await,
            &mut first_error,
        )?
        .unwrap_or(-1);
        let mut cursor = -1;
        let mut pending_cursor = -1;
        let mut page = VecDeque::<(i64, Option<String>)>::new();
        let mut startup_complete = boundary < 0;
        let mut startup_ready = one_pass;
        let network = crate::network::global();
        let socks = network.wait_for_startup_socks();
        tokio::pin!(socks);
        let mut active = BTreeSet::new();
        let mut reads = FuturesUnordered::<BoxFuture<'_, Completion>>::new();
        let mut qualification_due = Instant::now();
        let mut qualification_active = false;
        loop {
            if stop.is_cancelled() {
                break;
            }
            let changed = changes.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut wake = Instant::now() + CADENCE;
            let mut capacity_deferred = false;
            // The finite pass is independent of recurring requested work. Its
            // failed reads report failure; only explicit requests retain retries.
            while startup_ready && !startup_complete && active.len() < BALANCE_READS {
                if let Some((sequence, id)) = page.pop_front() {
                    cursor = sequence;
                    self.schedule_balance(id, true, &mut active, &mut reads, &mut first_error);
                    continue;
                }
                let pool = self.pool.clone();
                let upper = boundary;
                let rows = read_result(
                    self.store
                        .call(move |s| s.advisory_wallet_page(&pool, cursor, upper))
                        .await,
                    &mut first_error,
                )?;
                match rows {
                    Some(rows) if !rows.is_empty() => page = rows.into(),
                    _ => startup_complete = true,
                }
            }
            capacity_deferred |= startup_ready
                && active.len() == BALANCE_READS
                && page
                    .iter()
                    .any(|(_, id)| id.as_ref().is_none_or(|wallet| !active.contains(wallet)));
            if !one_pass {
                let pool = self.pool.clone();
                let pending = read_result(
                    self.store
                        .call(move |s| s.observation_page(&pool, pending_cursor))
                        .await,
                    &mut first_error,
                )?
                .unwrap_or_default();
                let more = pending.len() == PAGE_SIZE;
                if pending.is_empty() {
                    pending_cursor = -1;
                }
                let mut pending = pending.into_iter();
                while let Some(row) = pending.next() {
                    let (sequence, id) = (row.sequence, row.wallet);
                    let Some(work) = row.work else {
                        pending_cursor = sequence;
                        continue;
                    };
                    if active.len() == BALANCE_READS {
                        capacity_deferred |= std::iter::once(&id)
                            .chain(
                                pending
                                    .as_slice()
                                    .iter()
                                    .filter(|row| row.work.is_some())
                                    .map(|row| &row.wallet),
                            )
                            .any(|id| id.as_ref().is_none_or(|wallet| !active.contains(wallet)));
                        break;
                    }
                    pending_cursor = sequence;
                    let mut scalar = work.scalar_requested;
                    if let Some(wallet) = &id {
                        if active.contains(wallet) {
                            continue;
                        }
                        let scalar_due = if work.scalar_requested {
                            Some(self.store.balance_due(wallet).await)
                        } else {
                            None
                        };
                        let proof_due = if startup_ready {
                            work.retirement_due
                        } else {
                            None
                        };
                        let Some(due) = scalar_due.into_iter().chain(proof_due).min() else {
                            continue;
                        };
                        scalar = scalar_due == Some(due);
                        if due > Instant::now() {
                            wake = wake.min(due);
                            continue;
                        }
                    }
                    self.schedule_balance(id, scalar, &mut active, &mut reads, &mut first_error);
                }
                if more && active.len() < BALANCE_READS {
                    wake = Instant::now();
                }
                if !qualification_active && Instant::now() >= qualification_due {
                    qualification_active = true;
                    reads.push(
                        async {
                            Completion {
                                id: None,
                                result: self
                                    .qualification_observation(crate::qualification::active())
                                    .await,
                            }
                        }
                        .boxed(),
                    );
                }
            }
            if capacity_deferred {
                tracing::debug!(
                    limit_reads = BALANCE_READS,
                    "Advisory balance observation capacity reached; remaining inventory retained for a later pass"
                );
            }
            if one_pass && startup_complete && reads.is_empty() {
                let pool = self.pool.clone();
                let pending = self
                    .store
                    .call(move |s| s.retirement_unfinished(&pool))
                    .await?;
                if pending {
                    first_error.get_or_insert_with(|| {
                        anyhow::anyhow!(
                            "wallet retirement remains pending; original obligations are retained"
                        )
                    });
                }
                return first_error.map_or(Ok(()), Err);
            }
            tokio::select! {
                _=stop.cancelled()=>break,
                _=&mut socks, if !startup_ready => startup_ready=true,
                _=&mut changed=>{},
                Some(done)=reads.next(),if !reads.is_empty()=>{
                    if let Some(id)=&done.id { active.remove(id); }
                    else { qualification_active=false; qualification_due=Instant::now()+CADENCE; }
                    if let Err(error)=done.result {
                        if error.is::<RetirementObservation>() {
                            tracing::debug!(pool=%self.pool, wallet=?done.id, category=%super::super::base::safe_diagnostic(&error), "Wallet retirement proof attempt did not complete; financial obligations remain owned");
                            first_error.get_or_insert(error);
                            continue;
                        }
                        if error.downcast_ref::<rusqlite::Error>().is_some() && !crate::rotation::store::inventory::transient_read(&error) { return Err(error); }
                        tracing::warn!(category=%super::super::base::safe_diagnostic(&error),"advisory balance observation unavailable; independent payment service continues");
                        first_error.get_or_insert(error);
                    }
                },
                _=tokio::time::sleep_until(wake)=>{},
            }
        }
        Ok(())
    }
    fn schedule_balance<'a>(
        &'a self,
        id: Option<String>,
        scalar: bool,
        active: &mut BTreeSet<String>,
        reads: &mut FuturesUnordered<BoxFuture<'a, Completion>>,
        first_error: &mut Option<anyhow::Error>,
    ) {
        let Some(id) = id else {
            tracing::warn!(
                category = "malformed_advisory_wallet_row",
                "wallet observation unavailable; advancing to independent work"
            );
            first_error.get_or_insert_with(|| {
                crate::rotation::store::balances::AdvisoryIdentityUnavailable.into()
            });
            return;
        };
        if active.insert(id.clone()) {
            reads.push(
                async move {
                    if scalar {
                        self.read_balance(id).await
                    } else {
                        let result = self
                            .retire_wallet(id.clone())
                            .await
                            .map(|_| ())
                            .map_err(|error| error.context(RetirementObservation));
                        Completion {
                            id: Some(id),
                            result,
                        }
                    }
                }
                .boxed(),
            );
        }
    }
    async fn read_balance(&self, id: String) -> Completion {
        let result = self
            .store
            .observe_balance(self.pool.clone(), id.clone(), self.base.clone())
            .await
            .map(|_| ());
        let retirement = self
            .retire_wallet(id.clone())
            .await
            .map(|_| ())
            .map_err(|error| error.context(RetirementObservation));
        let result = result.and(retirement);
        Completion {
            id: Some(id),
            result,
        }
    }
    pub(super) async fn qualification_observation(&self, canonical: bool) -> Result<()> {
        let mut malformed = false;
        if canonical {
            let pool = self.pool.clone();
            let through = self
                .store
                .call(move |s| s.qualification_pending_boundary(&pool))
                .await?;
            let mut after = 0;
            loop {
                let pool = self.pool.clone();
                let page = self
                    .store
                    .call(move |s| s.qualification_pending_page(&pool, after, through))
                    .await?;
                let Some((last, _)) = page.last() else { break };
                after = *last;
                let pending: Vec<_> = page.into_iter().filter_map(|(_, auth)| match auth {
                    Ok(auth) => Some(auth),
                    Err(_) => {
                        malformed = true;
                        tracing::warn!(category="malformed_qualification_authorization_row", "authorization retained; qualification incomplete; advancing independent observations");
                        None
                    }
                }).collect();
                if pending.is_empty() {
                    continue;
                }
                let pool = self.pool.clone();
                let query = self
                    .store
                    .call(move |s| s.scoped_chain_query(&pool, vec![], pending))
                    .await?;
                let view = self.acquire_view(query).await?;
                let pool = self.pool.clone();
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                self.store
                    .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                    .await?;
            }
        }
        if let Some(observation) = crate::qualification::pool_observation(&self.pool).await {
            let pool = self.pool.clone();
            let through = self.store.call(move |s| s.wallet_boundary(&pool)).await?;
            let mut cursor = -1;
            let mut anchor = None;
            let mut wallets = std::collections::BTreeMap::new();
            loop {
                let pool = self.pool.clone();
                let page = self
                    .store
                    .call(move |s| s.wallet_page(&pool, cursor, through, PAGE_SIZE))
                    .await?;
                if page.is_empty() {
                    break;
                }
                cursor = page.last().expect("nonempty page").0;
                let pool = self.pool.clone();
                let query = self
                    .store
                    .call(move |s| {
                        s.scoped_chain_query(
                            &pool,
                            page.into_iter().map(|(_, id, _)| id).collect(),
                            vec![],
                        )
                    })
                    .await?;
                let view = self.acquire_view(query).await?;
                ensure!(
                    anchor.as_ref().is_none_or(|saved| saved == &view.anchor),
                    "qualification_full_inventory_anchor_changed"
                );
                anchor = Some(view.anchor.clone());
                wallets.extend(view.observations.iter().map(|(id, value)| {
                    (id.clone(), crate::qualification::observed_balance(value))
                }));
                ensure!(
                    serde_json::to_vec(&wallets)?.len() <= crate::qualification::LIMIT,
                    "qualification pool evidence exceeds byte limit"
                );
                let pool = self.pool.clone();
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                self.store
                    .call_with_guard(guard, move |s| s.publish_scoped_evidence(&pool, view))
                    .await?;
            }
            let anchor = anchor.context("qualification pool has no observed wallets")?;
            let pool = self.pool.clone();
            let guard = Arc::new(self.gate.clone().lock_owned().await);
            self.store.call_with_guard(guard,move |s| {
                ensure!(s.wallet_boundary(&pool)?==through,"qualification inventory changed during observation");
                let evidence=serde_json::json!({"schema_version":2,"block_height":anchor.height,"block_hash":anchor.hash,"wallets":wallets,"unresolved":s.qualification_exposure(&pool)?});
                observation.record(s.qualification_lifecycle()?,evidence)
            }).await?;
        }
        crate::qualification::reconcile_debit(&self.base, &self.store, &self.pool).await?;
        ensure!(!malformed, "malformed_qualification_authorization_row");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
