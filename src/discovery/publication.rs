//! Static publication shares the dynamic registry's serialization boundary.
use super::*;

impl Manager {
    /// Capture the static revision for off-lock loading or pricing. Dynamic
    /// registrations do not invalidate this revision.
    pub async fn static_snapshot(self: &Arc<Self>) -> Result<CatalogSnapshot> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || Ok(this.inner.lock().unwrap().base.clone())).await?
    }

    /// Replace the complete configured static view without losing registrations.
    /// Once accepted by the blocking worker, waiter cancellation does not undo
    /// publication. Retrying an old revision is rejected without changing state.
    pub async fn replace_static(
        self: &Arc<Self>,
        expected_revision: u64,
        mut base: CatalogSnapshot,
    ) -> Result<u64> {
        let permit = crate::limits::acquire_owned_capacity(
            self.mutations.clone(),
            "mutation_queue_full",
            Some(16),
            "hardcoded",
        )
        .await
        .context("mutation_queue_closed")?;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut inner = this.inner.lock().unwrap();
            ensure!(
                inner.base.generation == expected_revision,
                "static_revision_changed"
            );
            validate_static(&base, &this.listeners)?;
            let revision = expected_revision
                .checked_add(1)
                .context("static revision exhausted")?;
            let generation = this
                .catalog
                .read()
                .generation
                .max(inner.state.generation)
                .checked_add(1)
                .context("generation exhausted")?;
            base.generation = revision;
            let snapshot = this.snapshot(&inner.state, &base, generation)?;
            // Validation and all fallible work precede publication. This state is
            // process-local; persisted dynamic records are unchanged.
            inner.base = base;
            this.catalog.publish(snapshot);
            Ok(revision)
        })
        .await?
    }
}

fn validate_static(
    base: &CatalogSnapshot,
    listeners: &BTreeMap<String, ListenerConfig>,
) -> Result<()> {
    ensure!(
        base.views.keys().eq(listeners.keys()),
        "static_listener_views_changed"
    );
    for tools in base.views.values() {
        let mut names = std::collections::BTreeSet::new();
        for bound in tools {
            ensure!(bound.source.is_none(), "static_source_binding_invalid");
            ensure!(
                !bound.tool.name.starts_with("dyn_")
                    && !bound.tool.name.starts_with("x402_treazury_"),
                "static tool uses reserved namespace"
            );
            ensure!(names.insert(&bound.tool.name), "tool_name_collision");
        }
    }
    Ok(())
}
