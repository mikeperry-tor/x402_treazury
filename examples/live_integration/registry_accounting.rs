use super::*;
impl Registry {
    pub fn capture_accounting(&self, run: &str, session: &str, now: i64) -> Result<()> {
        self.validate_accounting_session(run, session, now)?;
        let exists: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='accounting_observed' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2)",params![run,session],|r|r.get(0))?;
        ensure!(!exists, "duplicate accounting session");
        let state = self
            .root
            .parent()
            .context("registry state directory missing")?;
        let _owner = files::lock(&state.join("owner.lock"))?;
        let snapshot = x402_treazury::rotation::store::qualification_state(state)?;
        ensure!(
            snapshot["treasury_status"]["treasury_id"] == self.manifest(run)?.treasury_id,
            "accounting treasury mismatch"
        );
        let data = crate::accounting::capture(&snapshot)?;
        let attempts = self.source_attribution_rows(run)?;
        crate::accounting_attribution::source(&data, &attempts)?;
        self.event(
            run,
            "accounting_observed",
            &json!({"session":session,"data":data,"source_attempts":attempts}),
            now,
        )
    }
    fn source_attribution_rows(&self, run: &str) -> Result<Vec<Value>> {
        let mut statement=self.db.prepare("SELECT s.operation,s.source_bound,s.released FROM funding_source_attempts s JOIN funding_permits p ON p.job=s.job WHERE p.run=?1 ORDER BY s.operation LIMIT 10001")?;
        let rows=statement.query_map([run],|r| Ok(json!({"operation":r.get::<_,String>(0)?,"source_bound":r.get::<_,i64>(1)?,"released":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > 10000 {
            eprintln!(
                "source attribution exceeds 10000 operations; export/review support required"
            );
            anyhow::bail!(
                "source attribution exceeds 10000 operations; export/review support required"
            );
        }
        Ok(rows)
    }
    fn validate_accounting_session(&self, run: &str, session: &str, at: i64) -> Result<()> {
        ensure!(
            !session.is_empty() && at >= 0,
            "invalid accounting session/time"
        );
        let count:i64=self.db.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='application_session' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2 AND at<=?3",params![run,session,at],|r|r.get(0))?;
        ensure!(count == 1, "accounting lacks registered session");
        let closed:i64=self.db.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='child_finished' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.success')=1 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.forced_kill')=0 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.valid_output')=1 AND at<=?3",params![run,session,at],|r|r.get(0))?;
        ensure!(closed == 1, "accounting lacks clean owned child exit");
        Ok(())
    }
    pub(super) fn accounting_report(&self, run: &str) -> Result<Value> {
        let manifest = self.manifest(run)?;
        let (raw_baseline, expected_hash): (String, String) = self.db.query_row(
            "SELECT baseline,baseline_hash FROM identity WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            files::hash(raw_baseline.as_bytes()) == expected_hash,
            "accounting baseline hash mismatch"
        );
        let baseline: Value = serde_json::from_str(&raw_baseline)?;
        let mut q = self.db.prepare(
            "SELECT detail,at FROM events WHERE run=?1 AND kind='accounting_observed' ORDER BY seq LIMIT 10001",
        )?;
        let observations = q
            .query_map([run], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if observations.len() > 10000 {
            eprintln!("accounting report exceeds 10000 snapshots; export/review support required");
            anyhow::bail!(
                "accounting report exceeds 10000 snapshots; export/review support required"
            );
        }
        let mut sessions = std::collections::BTreeSet::new();
        let mut result = Vec::new();
        for (raw, at) in observations {
            let observation: Value = serde_json::from_str(&raw)?;
            let session = observation["session"]
                .as_str()
                .context("accounting session missing")?;
            ensure!(
                sessions.insert(session.to_owned()),
                "duplicate accounting session"
            );
            self.validate_accounting_session(run, session, at)?;
            ensure!(
                observation["data"]["treasury_id"] == manifest.treasury_id,
                "accounting treasury mismatch"
            );
            let attributed = match observation.get("source_attempts") {
                None => json!({"status":"unobserved"}),
                Some(value) => crate::accounting_attribution::source(
                    &observation["data"],
                    value.as_array().context("invalid attributed attempts")?,
                )?,
            };
            result.push(json!({"session":session,"observed_at":at,"summary":crate::accounting::summarize(&observation["data"])?,"run_source_accounting":attributed,"baseline_comparison":crate::accounting_baseline::compare(&baseline,&observation["data"])?}));
        }
        Ok(
            json!({"status":if result.is_empty(){"unobserved"}else{"recorded"},"observations":result}),
        )
    }
}
