//! Reuse an enclosing retained call owner, or retain direct library work through
//! response conversion. The existing signing-owner bound also bounds new tasks.
use anyhow::{Context, Result};
use std::future::Future;
use tokio::sync::{Semaphore, SemaphorePermit};

static FOREGROUND: Semaphore = Semaphore::const_new(128);
tokio::task_local! {
    static HAS_FOREGROUND: ();
}
pub(crate) fn foreground() -> Result<Option<SemaphorePermit<'static>>> {
    if HAS_FOREGROUND.try_with(|_| ()).is_ok() {
        return Ok(None);
    }
    Ok(Some(
        crate::limits::try_capacity(&FOREGROUND, "foreground_work_busy", 128)
            .context("managed payment queue capacity unavailable")?,
    ))
}
pub(crate) fn run<T: Send + 'static>(
    owner: &crate::server::work::Work,
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Result<T>> + Send + '_ {
    let work = Box::pin(work);
    async move {
        // Retention alone grants no resource admission. Only HAS_FOREGROUND
        // proves that an enclosing workflow actually owns this same permit.
        let permit = foreground()?;
        let context = crate::qualification::capture_case();
        owner
            .run(async move {
                let _permit = permit;
                HAS_FOREGROUND
                    .scope((), crate::qualification::inherit_case(context, work))
                    .await
            })
            .await
    }
}
