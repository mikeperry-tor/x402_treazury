//! Durable stop boundaries. A skipped call is neither a payment nor a refunded reservation.
use super::*;
use crate::manifest::{Phase, RotationRound, Scenario};
use std::collections::BTreeSet;

const SKIPPED: &str = "SKIPPED_TARGET_REACHED";
fn round(phase: &Phase, index: usize) -> Result<&RotationRound> {
    let rounds = match &phase.scenario {
        Scenario::Rotation { rounds, .. }
        | Scenario::RefillService { rounds, .. }
        | Scenario::Lifecycle { rounds, .. } => rounds,
        _ => anyhow::bail!("promotion boundary requires a rotation scenario"),
    };
    rounds.get(index).context("unknown rotation round")
}
fn binding(
    m: &Manifest,
    phase: &Phase,
    index: usize,
    before: &Value,
    after: &Value,
) -> Result<String> {
    let round = round(phase, index)?;
    ensure!(
        before["treasury_id"] == m.treasury_id,
        "promotion treasury differs from manifest"
    );
    let mut pools = before["pools"]
        .as_array()
        .context("promotion pools missing")?
        .iter()
        .filter(|p| p["name"] == round.pool);
    let pool = pools.next().context("selected rotation pool missing")?;
    ensure!(pools.next().is_none(), "ambiguous rotation pool name");
    x402_treazury::qualification::lifecycle::promotion(
        before,
        after,
        pool["id"].as_str().context("rotation pool ID missing")?,
    )
}
impl Registry {
    /// Called by the supervisor after observing a promotion. No CLI can manufacture skips.
    pub fn stop_depletion(
        &mut self,
        run: &str,
        phase_id: &str,
        index: usize,
        before: &Value,
        after: &Value,
        now: i64,
    ) -> Result<Vec<String>> {
        ensure!(now >= 0, "invalid promotion timestamp");
        self.validated_skips(run)?;
        let m = self.manifest(run)?;
        let phase = m
            .phases
            .iter()
            .find(|p| p.id == phase_id)
            .context("unknown rotation phase")?;
        let round = round(phase, index)?;
        let job = binding(&m, phase, index, before, after)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='rotation_promotion' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.phase')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.round')=?3)",params![run,phase_id,i64::try_from(index)?],|r|r.get(0))?;
        ensure!(!exists, "rotation boundary already recorded; observe only");
        let mut skipped = Vec::new();
        let mut attempted = 0;
        for id in &round.depletion_cases {
            let state: String = tx.query_row(
                "SELECT execution FROM cases WHERE run=?1 AND id=?2",
                params![run, id],
                |r| r.get(0),
            )?;
            if state == "UNATTEMPTED" {
                skipped.push(id.clone());
            } else {
                ensure!(
                    skipped.is_empty()
                        && matches!(state.as_str(), "COMPLETED" | "TRANSPORT_UNCERTAIN"),
                    "rotation boundary requires a finished depletion prefix without in-flight calls"
                );
                attempted += 1;
            }
        }
        ensure!(
            attempted > 0,
            "rotation boundary requires an attempted depletion call"
        );
        for id in &round.service_cases {
            let state: String = tx.query_row(
                "SELECT execution FROM cases WHERE run=?1 AND id=?2",
                params![run, id],
                |r| r.get(0),
            )?;
            ensure!(
                state == "UNATTEMPTED",
                "service dispatched before durable promotion boundary"
            );
        }
        for id in &skipped {
            ensure!(tx.execute("UPDATE cases SET execution='SKIPPED_TARGET_REACHED',semantic='NOT_EXECUTED',settlement='NOT_SIGNED' WHERE run=?1 AND id=?2 AND execution='UNATTEMPTED' AND result_hash IS NULL",params![run,id])? == 1,
                "only untouched depletion cases can be skipped");
        }
        let detail = bounded_json(
            &json!({"phase":phase_id,"round":index,"before":before,"after":after,"replacement_job":job,"skipped":skipped}),
        )?;
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'rotation_promotion',?2,?3)",
            params![run, detail, now],
        )?;
        validate(&tx, &m, run)?;
        tx.commit()?;
        eprintln!(
            "qualification phase {phase_id} round {index}: promotion recorded; {} unused depletion calls skipped, attempted reservations retained",
            skipped.len()
        );
        Ok(skipped)
    }
    /// Recompute skipped-case legitimacy from the immutable manifest and retained snapshots.
    pub fn validated_skips(&self, run: &str) -> Result<BTreeSet<String>> {
        let m = self.manifest(run)?;
        validate(&self.db, &m, run)
    }
}
fn validate(db: &Connection, m: &Manifest, run: &str) -> Result<BTreeSet<String>> {
    let mut query = db.prepare("SELECT detail FROM events WHERE run=?1 AND kind='rotation_promotion' ORDER BY seq LIMIT 10001")?;
    let rows = query
        .query_map([run], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        rows.len() <= 10000,
        "rotation evidence exceeds 10000-boundary run limit"
    );
    let mut boundaries = BTreeSet::new();
    let mut skipped = BTreeSet::new();
    let mut jobs = BTreeSet::new();
    let mut generations = std::collections::BTreeMap::new();
    for raw in rows {
        let event: Value = serde_json::from_str(&raw)?;
        let phase_id = event["phase"].as_str().context("promotion phase missing")?;
        let index = usize::try_from(event["round"].as_u64().context("promotion round missing")?)?;
        ensure!(
            boundaries.insert((phase_id.to_owned(), index)),
            "duplicate rotation boundary"
        );
        let phase = m
            .phases
            .iter()
            .find(|p| p.id == phase_id)
            .context("promotion phase not reviewed")?;
        ensure!(
            index == 0 || boundaries.contains(&(phase_id.to_owned(), index - 1)),
            "rotation boundaries out of order"
        );
        let round = round(phase, index)?;
        let job = binding(m, phase, index, &event["before"], &event["after"])?;
        let pool = event["before"]["pools"]
            .as_array()
            .context("promotion pools missing")?
            .iter()
            .find(|p| p["name"] == round.pool)
            .context("rotation pool missing")?;
        let generation = pool["generation"]
            .as_u64()
            .context("rotation generation missing")?;
        if let Some((id, expected)) = generations.get(&round.pool) {
            ensure!(
                id == &pool["id"] && *expected == generation,
                "rotation boundaries reused or skipped a generation"
            );
        }
        generations.insert(
            round.pool.clone(),
            (
                pool["id"].clone(),
                generation
                    .checked_add(1)
                    .context("rotation generation overflow")?,
            ),
        );
        ensure!(
            jobs.insert(job.clone()),
            "replacement job reused across rotation boundaries"
        );
        ensure!(
            event["replacement_job"] == job,
            "promotion replacement evidence changed"
        );
        let ids: Vec<String> = serde_json::from_value(event["skipped"].clone())?;
        ensure!(
            ids.len() < round.depletion_cases.len() && round.depletion_cases.ends_with(&ids),
            "skipped calls must be the unused suffix after at least one depletion call"
        );
        for id in &round.depletion_cases[..round.depletion_cases.len() - ids.len()] {
            let state: String = db.query_row(
                "SELECT execution FROM cases WHERE run=?1 AND id=?2",
                params![run, id],
                |r| r.get(0),
            )?;
            ensure!(
                matches!(state.as_str(), "COMPLETED" | "TRANSPORT_UNCERTAIN"),
                "promotion evidence lacks finished depletion prefix"
            );
        }
        for id in ids {
            ensure!(skipped.insert(id), "case skipped by multiple boundaries");
        }
    }
    let mut cases =
        db.prepare("SELECT id,execution,semantic,settlement,result_hash FROM cases WHERE run=?1")?;
    for row in cases.query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })? {
        let (id, state, semantic, settlement, result) = row?;
        ensure!(
            (state == SKIPPED) == skipped.contains(&id),
            "skipped case lacks matching promotion evidence"
        );
        if state == SKIPPED {
            ensure!(
                semantic == "NOT_EXECUTED" && settlement == "NOT_SIGNED" && result.is_none(),
                "skipped case has execution evidence"
            );
            let accepted: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind IN ('application_claim','mcp_dispatch_intent','application_payment') AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2)",params![run,id],|r|r.get(0))?;
            let reserved: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM batches, json_each(batches.members) member WHERE batches.run=?1 AND member.value=?2)",params![run,id],|r|r.get(0))?;
            ensure!(
                !accepted && !reserved,
                "skipped case was reserved, accepted or dispatched"
            );
        }
    }
    Ok(skipped)
}
