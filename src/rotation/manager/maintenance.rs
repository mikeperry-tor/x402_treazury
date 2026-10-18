//! One retained advisory refresh owner per Store/group. No payment-history repair.
use super::*;
use crate::rotation::store::{history::PAGE_SIZE, inventory::read_result};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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
        let _owner = tokio::select! { _=stop.cancelled()=>return Ok(()), owner=self.reconciliation.clone().lock_owned()=>owner };
        self.maintenance(stop, false).await
    }
    async fn maintenance(&self, stop: CancellationToken, one_pass: bool) -> Result<()> {
        let pool = self.pool.clone();
        let changes = self
            .store
            .call(move |s| Ok(s.capacity_changes(&pool)))
            .await?;
        let mut cursor = -1;
        let mut pending_cursor = -1;
        let mut through = None;
        let mut page = VecDeque::<(i64, Option<String>)>::new();
        let mut active = BTreeSet::new();
        let mut retry_at = BTreeMap::<String, Instant>::new();
        let mut reads = FuturesUnordered::<BoxFuture<'_, Completion>>::new();
        let mut complete = false;
        let mut due = Instant::now();
        let mut qualification_due = Instant::now();
        let mut qualification_active = false;
        let mut first_error = None;
        loop {
            if stop.is_cancelled() {
                break;
            }
            retry_at.retain(|_, at| *at > Instant::now());
            let changed = changes.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let pool = self.pool.clone();
            let pending = read_result(
                self.store
                    .call(move |s| s.pending_balances(&pool, pending_cursor))
                    .await,
                &mut first_error,
            )?
            .unwrap_or_default();
            let pending_wrapped = pending.is_empty() && pending_cursor >= 0;
            let more_pending = pending.len() == PAGE_SIZE;
            if pending.is_empty() {
                pending_cursor = -1;
            }
            for (sequence, id) in pending {
                if active.len() == BALANCE_READS {
                    break;
                }
                pending_cursor = sequence;
                let Some(id) = id else {
                    if first_error.is_none() {
                        tracing::warn!(
                            category = "malformed_advisory_wallet_row",
                            "wallet observation unavailable; advancing to independent work"
                        );
                    }
                    first_error.get_or_insert_with(|| {
                        crate::rotation::store::balances::AdvisoryIdentityUnavailable.into()
                    });
                    continue;
                };
                if !active.contains(&id) && retry_at.get(&id).is_none_or(|at| *at <= Instant::now())
                {
                    active.insert(id.clone());
                    reads.push(self.read_balance(id).boxed());
                }
            }
            while active.len() < BALANCE_READS && Instant::now() >= due && !(one_pass && complete) {
                if let Some((sequence, id)) = page.pop_front() {
                    cursor = sequence;
                    let Some(id) = id else {
                        if first_error.is_none() {
                            tracing::warn!(
                                category = "malformed_advisory_wallet_row",
                                "wallet observation unavailable; advancing to independent work"
                            );
                        }
                        first_error.get_or_insert_with(|| {
                            crate::rotation::store::balances::AdvisoryIdentityUnavailable.into()
                        });
                        continue;
                    };
                    if !active.contains(&id)
                        && retry_at.get(&id).is_none_or(|at| *at <= Instant::now())
                    {
                        active.insert(id.clone());
                        reads.push(self.read_balance(id).boxed());
                    }
                    continue;
                }
                let pool = self.pool.clone();
                if through.is_none() {
                    through = read_result(
                        self.store.call(move |s| s.wallet_boundary(&pool)).await,
                        &mut first_error,
                    )?;
                    cursor = -1;
                    if through.is_none() {
                        due = Instant::now() + CADENCE;
                        complete = true;
                        break;
                    }
                }
                let pool = self.pool.clone();
                let boundary = through.unwrap();
                let rows = read_result(
                    self.store
                        .call(move |s| s.advisory_wallet_page(&pool, cursor, boundary))
                        .await,
                    &mut first_error,
                )?;
                match rows {
                    Some(rows) if !rows.is_empty() => page = rows.into(),
                    _ => {
                        complete = true;
                        through = None;
                        due = Instant::now() + CADENCE;
                        break;
                    }
                }
            }
            if !qualification_active && Instant::now() >= qualification_due && !one_pass {
                qualification_active = true;
                reads.push(
                    async {
                        Completion {
                            id: None,
                            result: self.qualification_observation().await,
                        }
                    }
                    .boxed(),
                );
            }
            if one_pass && complete && reads.is_empty() {
                if pending_wrapped {
                    continue;
                }
                return first_error.map_or(Ok(()), Err);
            }
            let wake = if more_pending && active.len() < BALANCE_READS {
                Instant::now()
            } else {
                Instant::now() + CADENCE
            };
            tokio::select! {
                _=stop.cancelled()=>break,
                _=&mut changed=>{},
                Some(done)=reads.next(),if !reads.is_empty()=>{
                    if let Some(id)=done.id {
                        active.remove(&id);
                        if done.result.is_err() { retry_at.insert(id,Instant::now()+CADENCE); } else { retry_at.remove(&id); }
                    } else {qualification_active=false;qualification_due=Instant::now()+CADENCE;}
                    if let Err(error)=done.result {
                        // Shared Store ownership/corruption cannot be bypassed by a
                        // second owner. RPC/address failures concern this observation.
                        if error.downcast_ref::<rusqlite::Error>().is_some() && !crate::rotation::store::inventory::transient_read(&error) { return Err(error); }
                        tracing::warn!(category=%super::super::base::safe_diagnostic(&error),"advisory balance observation unavailable; independent payment service continues");
                        first_error.get_or_insert(error);
                    }
                },
                _=tokio::time::sleep_until(wake)=>{},
            }
        }
        // Unsigned reads may stop; already accepted Store writes retain ownership.
        Ok(())
    }
    async fn read_balance(&self, id: String) -> Completion {
        let result = async {
            let (pool, wallet) = (self.pool.clone(), id.clone());
            let address = self
                .store
                .call(move |s| s.begin_balance(&pool, &wallet))
                .await;
            let result = match address {
                Ok(address) => self.base.current_balance(&address).await,
                Err(error)
                    if error
                        .is::<crate::rotation::store::balances::AdvisoryIdentityUnavailable>() =>
                {
                    Err(error)
                }
                Err(error) => return Err(error),
            };
            let value = result.as_ref().ok().copied();
            let (pool, wallet) = (self.pool.clone(), id.clone());
            self.store
                .call(move |s| s.finish_balance(&pool, &wallet, value))
                .await?;
            result.map(|_| ())
        }
        .await;
        Completion {
            id: Some(id),
            result,
        }
    }
    async fn qualification_observation(&self) -> Result<()> {
        let mut malformed = false;
        if crate::qualification::active() {
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
                let view = self.acquire_view(query, false).await?;
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
                let view = self.acquire_view(query, false).await?;
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
