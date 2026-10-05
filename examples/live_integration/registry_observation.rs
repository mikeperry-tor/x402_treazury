//! Explicit fresh snapshots answered by the managed application's normal reconciliation.
use super::*;
use rusqlite::OptionalExtension;
impl Registry {
    pub fn request_pool_observation(
        &mut self,
        run: &str,
        pool: &str,
        pool_name: &str,
        now: i64,
    ) -> Result<String> {
        let m = self.manifest(run)?;
        ensure!(
            m.start.pools.iter().any(|p| p == pool_name),
            "observation pool is not selected"
        );
        let session: String = self.db.query_row("SELECT detail FROM events WHERE run=?1 AND kind='application_session' ORDER BY seq DESC LIMIT 1",[run],|r|r.get(0))?;
        let binding: x402_treazury::qualification::Binding = serde_json::from_str(&session)?;
        ensure!(
            binding.run == run && identifier(&binding.session) && identifier(pool),
            "invalid pool observation session or pool binding"
        );
        ensure!(
            now >= 0 && now < binding.expires_at,
            "observation request outside application window"
        );
        let pending: i64 = self.db.query_row("SELECT COUNT(*) FROM events r WHERE r.run=?1 AND r.kind='pool_observation_requested' AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.pool')=?2 AND json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.session')=?3 AND NOT EXISTS(SELECT 1 FROM events e WHERE e.run=r.run AND e.kind='application_pool_observation' AND json_extract(CASE WHEN json_valid(e.detail) THEN e.detail END,'$.request')=json_extract(CASE WHEN json_valid(r.detail) THEN r.detail END,'$.request'))",params![run,pool,binding.session],|r|r.get(0))?;
        ensure!(
            pending == 0,
            "pool observation already pending; await its response, do not issue another request"
        );
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM events WHERE run=?1 AND kind='pool_observation_requested'",
            [run],
            |r| r.get(0),
        )?;
        ensure!(
            count < 10000,
            "pool observation exceeds 10000-request run limit"
        );
        let request = uuid::Uuid::new_v4().to_string();
        self.event(
            run,
            "pool_observation_requested",
            &json!({"request":request,"session":binding.session,"pool":pool,"pool_name":pool_name}),
            now,
        )?;
        Ok(request)
    }
    pub fn pool_observation(&self, run: &str, request: &str) -> Result<Option<Value>> {
        let raw: String = self.db.query_row("SELECT detail FROM events WHERE run=?1 AND kind='pool_observation_requested' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.request')=?2",params![run,request],|r|r.get(0))?;
        let asked: Value = serde_json::from_str(&raw)?;
        let mut query = self.db.prepare("SELECT detail FROM events WHERE run=?1 AND kind='application_pool_observation' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.request')=?2 ORDER BY seq LIMIT 2")?;
        let rows = query
            .query_map(params![run, request], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(rows.len() <= 1, "duplicate pool observation response");
        let Some(raw) = rows.into_iter().next() else {
            return Ok(None);
        };
        let observed: Value = serde_json::from_str(&raw)?;
        for field in ["request", "session", "pool", "pool_name"] {
            ensure!(
                asked[field] == observed[field],
                "pool observation correlation mismatch"
            );
        }
        ensure!(
            observed["state"]["treasury_id"] == self.manifest(run)?.treasury_id,
            "pool observation treasury mismatch"
        );
        ensure!(
            observed["query_started_micros"]
                .as_u64()
                .zip(observed["observed_micros"].as_u64())
                .is_some_and(|(a, b)| a <= b),
            "pool observation timing invalid"
        );
        let registered: Option<String> = self.db.query_row("SELECT detail FROM events WHERE run=?1 AND kind='application_session' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2",params![run,observed["session"].as_str()],|r|r.get(0)).optional()?;
        ensure!(
            registered.is_some(),
            "pool observation lacks application session"
        );
        Ok(Some(observed))
    }
}
