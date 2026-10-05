//! Observe production evidence; never infer settlement from an MCP/seller result.
use super::*;
impl Registry {
    pub fn observe_settlement(&mut self, run: &str, owner_closed: bool) -> Result<()> {
        let state = self
            .root
            .parent()
            .context("registry state directory missing")?;
        // Absence of signing evidence is meaningful only once accepted store work
        // has drained and this supervisor owns the treasury.
        let _owner = owner_closed
            .then(|| files::lock(&state.join("owner.lock")))
            .transpose()?;
        let snapshot = x402_treazury::rotation::store::qualification_state(state)?;
        self.apply_settlement(run, &snapshot, owner_closed)
    }
    pub(crate) fn apply_settlement(
        &mut self,
        run: &str,
        snapshot: &Value,
        owner_closed: bool,
    ) -> Result<()> {
        let manifest = self.manifest(run)?;
        ensure!(
            snapshot["treasury_status"]["treasury_id"] == manifest.treasury_id,
            "settlement observation treasury mismatch"
        );
        self.application_evidence(run)?;
        let attempts = snapshot["payment_attempts"]
            .as_array()
            .context("payment attempts missing")?;
        let resolutions = snapshot["payment_resolutions"]
            .as_array()
            .context("payment resolutions missing")?;
        let tx = self.db.transaction()?;
        for case in manifest.cases.iter().filter(|c| !c.unsigned) {
            let (execution, previous): (String, String) = tx.query_row(
                "SELECT execution,settlement FROM cases WHERE run=?1 AND id=?2",
                params![run, case.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if !matches!(execution.as_str(), "COMPLETED" | "TRANSPORT_UNCERTAIN")
                || previous != "PENDING"
            {
                continue;
            }
            let raw: Option<String> = tx.query_row(
                "SELECT detail FROM events WHERE run=?1 AND kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2",
                params![run,case.id], |r| r.get(0)).optional()?;
            let completion: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_finished' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2)",
                params![run,case.id], |r| r.get(0))?;
            let mut evidence = json!({"case":case.id,"owner_closed":owner_closed});
            let outcome = if let Some(raw) = raw {
                let payment: Value = serde_json::from_str(&raw)?;
                let attempt = attempts.iter().find(|a| a["id"] == payment["attempt_id"]);
                let resolution = resolutions
                    .iter()
                    .find(|r| r["attempt_id"] == payment["attempt_id"]);
                if let Some(attempt) = attempt {
                    for (field, recorded) in [
                        ("pool", "pool"),
                        ("wallet", "wallet"),
                        ("generation", "generation"),
                        ("amount", "amount"),
                    ] {
                        ensure!(
                            attempt[field] == payment[recorded],
                            "payment evidence changed: {field}"
                        );
                    }
                    if let Some(resolution) = resolution {
                        ensure!(
                            attempt["state"] == "RESOLVED"
                                && resolution["height"].as_u64().is_some()
                                && resolution["block_time"].as_u64().is_some()
                                && resolution["hash"].as_str().is_some_and(|s| !s.is_empty()),
                            "invalid canonical payment resolution"
                        );
                        let outcome = resolution["outcome"]
                            .as_str()
                            .context("resolution outcome missing")?;
                        ensure!(
                            matches!(outcome, "USED" | "EXPIRED_UNUSED"),
                            "unknown payment resolution"
                        );
                        evidence["canonical"] = resolution.clone();
                        Some(outcome.to_owned())
                    } else if owner_closed && completion && attempt["state"] == "ADMITTED" {
                        Some("NOT_SIGNED".into())
                    } else {
                        None
                    }
                } else {
                    ensure!(
                        resolution.is_none(),
                        "canonical resolution lacks payment attempt"
                    );
                    (owner_closed && completion).then(|| "NOT_SIGNED".into())
                }
            } else {
                (owner_closed && completion).then(|| "NOT_SIGNED".into())
            };
            if let Some(outcome) = outcome {
                evidence["outcome"] = json!(outcome);
                tx.execute("UPDATE cases SET settlement=?3 WHERE run=?1 AND id=?2 AND settlement='PENDING'",
                    params![run,case.id,outcome])?;
                tx.execute(
                    "INSERT INTO events(run,kind,detail,at) VALUES(?1,'payment_observed',?2,?3)",
                    params![
                        run,
                        bounded_json(&evidence)?,
                        i64::try_from(x402_treazury::rotation::base::now()?)?
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
use rusqlite::OptionalExtension;
