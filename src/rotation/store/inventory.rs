//! Retry classification for audited read-only inventory boundaries.
#[cfg(test)]
use super::*;

pub(crate) fn transient_read(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(failure, _))
        if matches!(failure.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Priority,
    WalletBoundary,
    WalletPage,
    HistoryBoundary,
    HistoryPage,
}

#[cfg(test)]
impl Store {
    pub(crate) fn fail_next_inventory_read(&mut self, read: Read, code: i32) {
        *self.inventory_failure.lock().unwrap() = Some((read, code));
    }
    pub(super) fn inventory_read_fault(&self, read: Read) -> Result<()> {
        let mut fault = self.inventory_failure.lock().unwrap();
        if fault
            .as_ref()
            .is_some_and(|(selected, _)| *selected == read)
        {
            let (_, code) = fault.take().unwrap();
            return Err(
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None).into(),
            );
        }
        Ok(())
    }
}
