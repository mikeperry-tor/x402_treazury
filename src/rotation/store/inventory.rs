//! Retry classification for audited read-only inventory boundaries.
#[cfg(test)]
use super::*;

pub(crate) fn transient_read(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(failure, _))
        if matches!(failure.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}

// Only read-only inventory acquisition uses this retry classifier. Financial
// publication, owner/channel failures and unknown corruption still propagate.
pub(crate) fn read_result<T>(
    result: anyhow::Result<T>,
    first_error: &mut Option<anyhow::Error>,
) -> anyhow::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if transient_read(&error) => {
            tracing::warn!(
                category = "inventory_busy",
                "inventory observation deferred; active work remains owned"
            );
            first_error.get_or_insert(error);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    WalletBoundary,
    WalletPage,
    FundingBoundary,
    FundingPage,
}

#[cfg(test)]
impl Store {
    pub(crate) fn inventory_fault_pending(&self) -> bool {
        self.inventory_failure.lock().unwrap().is_some()
    }
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
