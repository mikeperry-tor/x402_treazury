//! Scoped source-owner reads; operational sync never expands payment history.
use super::*;

pub(crate) struct TreasurySyncState {
    pub snapshot_revision: i64,
    pub sync: Option<SyncObservation>,
}
impl Store {
    pub(crate) fn treasury_sync_state(&self) -> Result<TreasurySyncState> {
        let snapshot_revision = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        let sync = self
            .db
            .query_row(
                "SELECT observation FROM treasury_sync WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| serde_json::from_str(&s))
            .transpose()?;
        Ok(TreasurySyncState {
            snapshot_revision,
            sync,
        })
    }
}
