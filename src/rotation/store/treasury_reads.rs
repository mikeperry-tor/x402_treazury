//! Scoped source-owner reads; operational sync never expands payment history.
use super::*;

pub(crate) struct TreasurySyncState {
    pub snapshot_revision: i64,
    pub sync: Option<SyncObservation>,
    pub outgoing_pending: bool,
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
        let outgoing_pending = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED') OR EXISTS(SELECT 1 FROM operation_network WHERE preparation IS NOT NULL)",
            [],
            |r| r.get(0),
        )?;
        Ok(TreasurySyncState {
            snapshot_revision,
            sync,
            outgoing_pending,
        })
    }
}
