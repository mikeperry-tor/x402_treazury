//! On-demand observations piggyback on normal chain reconciliation. No signing authority.
use super::*;
use rusqlite::OptionalExtension;

pub(crate) struct PoolObservation {
    guard: Arc<Guard>,
    request: Value,
    started_micros: u128,
}
pub(crate) async fn pool_observation(pool: &str) -> Option<PoolObservation> {
    let guard = ACTIVE.get().filter(|g| g.managed)?.clone();
    let pool = pool.to_owned();
    match tokio::task::spawn_blocking(move || pending(guard, &pool)).await {
        Ok(Ok(value)) => value,
        _ => {
            tracing::warn!(
                category = "qualification_pool_observation_unavailable",
                "requested pool evidence unavailable; normal reconciliation continues, qualification must not infer a fresh balance"
            );
            None
        }
    }
}
fn pending(guard: Arc<Guard>, pool: &str) -> Result<Option<PoolObservation>> {
    let db = connection(&guard.binding)?;
    let mut query = db.prepare("SELECT r.detail FROM events r WHERE r.run=?1 AND r.kind='pool_observation_requested' AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.pool')=?2 AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.session')=?3 AND NOT EXISTS(SELECT 1 FROM events e WHERE e.run=r.run AND e.kind='application_pool_observation' AND json_extract(CASE WHEN json_valid(e.detail) THEN e.detail END,'$.request')=json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.request')) ORDER BY r.seq LIMIT 2")?;
    let requests = query
        .query_map(
            params![guard.binding.run, pool, guard.binding.session],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        requests.len() <= 1,
        "multiple pending pool observation requests"
    );
    let Some(raw) = requests.into_iter().next() else {
        return Ok(None);
    };
    let request = bounded(raw)?;
    ensure!(
        request["request"].as_str().is_some_and(identifier),
        "invalid pool observation request"
    );
    let manifest = authority(&db, &guard.binding, now()?)?;
    ensure!(
        manifest["start"]["pools"]
            .as_array()
            .context("selected pools missing")?
            .contains(&request["pool_name"]),
        "observation pool outside manifest"
    );
    let started_micros = guard.origin.elapsed().as_micros();
    Ok(Some(PoolObservation {
        guard,
        request,
        started_micros,
    }))
}
impl PoolObservation {
    /// Invoked in the same store-worker command as successful reconciliation, gate held.
    pub(crate) fn record(&self, state: Value, balances: Value) -> Result<()> {
        let mut db = connection(&self.guard.binding)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Completion may outlive dispatch authority; it grants no further work.
        let raw: String = tx.query_row("SELECT detail FROM events WHERE run=?1 AND kind='pool_observation_requested' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.request')=?2",params![self.guard.binding.run,self.request["request"].as_str()],|r|r.get(0))?;
        ensure!(
            bounded(raw)? == self.request,
            "pool observation request changed"
        );
        let existing: Option<i64> = tx.query_row("SELECT seq FROM events WHERE run=?1 AND kind='application_pool_observation' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.request')=?2",params![self.guard.binding.run,self.request["request"].as_str()],|r|r.get(0)).optional()?;
        ensure!(existing.is_none(), "pool observation already completed");
        let manifest: String = tx.query_row(
            "SELECT manifest FROM runs WHERE id=?1",
            [&self.guard.binding.run],
            |r| r.get(0),
        )?;
        ensure!(
            state["treasury_id"] == bounded(manifest)?["treasury_id"],
            "pool observation treasury mismatch"
        );
        let pools = state["pools"]
            .as_array()
            .context("observed pools missing")?;
        ensure!(
            pools
                .iter()
                .any(|p| p["id"] == self.request["pool"] && p["name"] == self.request["pool_name"]),
            "pool observation binding mismatch"
        );
        let event = json!({"request":self.request["request"],"pool":self.request["pool"],"pool_name":self.request["pool_name"],
            "session":self.guard.binding.session,"query_started_micros":self.started_micros,
            "observed_micros":self.guard.origin.elapsed().as_micros(),"state":state,"balances":balances});
        let encoded = event.to_string();
        ensure!(
            encoded.len() <= LIMIT,
            "pool observation exceeds {LIMIT}-byte evidence limit"
        );
        tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_pool_observation',?2,?3)",params![self.guard.binding.run,encoded,now()?])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Arc<Guard>, Connection) {
        let (dir, mut guard, db) = super::super::tests::fixture();
        Arc::get_mut(&mut guard).unwrap().managed = true;
        let manifest = json!({"treasury_id":"treasury","start":{"pools":["pool"]}});
        db.execute("UPDATE runs SET manifest=?1", [manifest.to_string()])
            .unwrap();
        db.execute("INSERT INTO events(run,kind,detail,at) VALUES('run','pool_observation_requested',?1,0)",
            [json!({"request":"request_1","pool":"p","pool_name":"pool","session":"session"}).to_string()]).unwrap();
        (dir, guard, db)
    }
    #[test]
    fn fresh_observation_is_correlated_once_and_cannot_grant_another_request() {
        let (_dir, guard, db) = fixture();
        assert!(pending(guard.clone(), "other").unwrap().is_none());
        let observation = pending(guard.clone(), "p").unwrap().unwrap();
        let state = json!({"treasury_id":"treasury","pools":[{"id":"p","name":"pool"}]});
        let balances = json!({"block_height":10,"block_hash":format!("0x{:064x}",1),"confirmed":{"wallet":"100"},"unresolved":{"wallet":"7"}});
        observation.record(state.clone(), balances.clone()).unwrap();
        assert!(observation.record(state, balances).is_err());
        assert!(pending(guard, "p").unwrap().is_none());
        let raw: String = db
            .query_row(
                "SELECT detail FROM events WHERE kind='application_pool_observation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["request"], "request_1");
        assert!(
            value["observed_micros"].as_u64().unwrap()
                >= value["query_started_micros"].as_u64().unwrap()
        );
        assert_eq!(value["balances"]["unresolved"]["wallet"], "7");
    }
    #[test]
    fn changed_requests_wrong_pool_and_stale_sessions_never_produce_evidence() {
        let (_dir, guard, db) = fixture();
        let observation = pending(guard.clone(), "p").unwrap().unwrap();
        let wrong = json!({"treasury_id":"treasury","pools":[{"id":"p","name":"other"}]});
        assert!(observation.record(wrong, json!({})).is_err());
        db.execute("UPDATE events SET detail=json_set(detail,'$.session','previous') WHERE kind='pool_observation_requested'",[]).unwrap();
        assert!(pending(guard.clone(), "p").unwrap().is_none());
        assert!(observation.record(json!({}), json!({})).is_err());
        db.execute("UPDATE events SET detail=json_set(detail,'$.session','session','$.pool_name','unselected') WHERE kind='pool_observation_requested'",[]).unwrap();
        assert!(pending(guard, "p").is_err());
    }
    #[test]
    fn pending_observation_fails_closed_for_ambiguous_requests_and_expired_authority() {
        let (_dir, mut guard, db) = fixture();
        db.execute("INSERT INTO events(run,kind,detail,at) SELECT run,kind,detail,at FROM events WHERE kind='pool_observation_requested'",[]).unwrap();
        assert!(pending(guard.clone(), "p").is_err());
        db.execute(
            "DELETE FROM events WHERE seq=(SELECT MAX(seq) FROM events)",
            [],
        )
        .unwrap();
        Arc::get_mut(&mut guard).unwrap().binding.expires_at = 0;
        assert!(pending(guard, "p").is_err());
    }
}
