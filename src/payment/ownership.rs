//! Retain direct library work through response conversion and preserve qualification context.
use anyhow::Result;
use std::future::Future;

pub(crate) fn run<T: Send + 'static>(
    owner: &crate::server::work::Work,
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Result<T>> + Send + '_ {
    // Preserve the existing indirection: deeply nested MCP futures otherwise
    // exceed the compiler's layout-query depth. Context is captured on first poll.
    let work = Box::pin(work);
    async move {
        let context = crate::qualification::capture_case();
        owner
            .run(crate::qualification::inherit_case(context, work))
            .await
    }
}
