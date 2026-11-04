//! Disposable estimates owned by a remote catalog cache entry.
use alloy_primitives::U256;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Receipt {
    at: Instant,
    sequence: u64,
}
pub(crate) fn received() -> Option<Receipt> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let at = Instant::now();
    let mut sequence = SEQUENCE.load(Ordering::Relaxed);
    loop {
        let next = sequence.checked_add(1)?;
        match SEQUENCE.compare_exchange_weak(sequence, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Some(Receipt { at, sequence }),
            Err(current) => sequence = current,
        }
    }
}

pub(crate) struct Hint {
    pub amount: U256,
    pub version: i64,
    pub receipt: Option<Receipt>,
}
pub struct CatalogHints {
    pub(crate) slot: crate::http_cache::Slot,
    pub(crate) generation: String,
    pub(crate) state: Mutex<State>,
}
pub(crate) struct State {
    pub metadata: crate::http_cache::Metadata,
    pub valid: bool,
    pub prices: BTreeMap<String, Hint>,
}
impl std::fmt::Debug for CatalogHints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CatalogHints")
    }
}
#[derive(Clone, Debug)]
pub(crate) struct OperationKey {
    pub catalog: Arc<CatalogHints>,
    pub operation: String,
}
impl PartialEq for OperationKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.catalog, &other.catalog) && self.operation == other.operation
    }
}
impl Eq for OperationKey {}
impl CatalogHints {
    pub(crate) fn operation(self: &Arc<Self>, method: &str, template: &str) -> OperationKey {
        OperationKey {
            catalog: self.clone(),
            operation: format!("{} {template}", method.to_ascii_uppercase()),
        }
    }
    pub(crate) fn invalidate(&self) {
        self.state.lock().expect("catalog hints poisoned").valid = false;
    }
}
impl OperationKey {
    pub(crate) fn get(&self) -> Option<U256> {
        let state = self.catalog.state.lock().expect("catalog hints poisoned");
        (state.valid && state.metadata.fresh())
            .then(|| state.prices.get(&self.operation).map(|p| p.amount))
            .flatten()
    }
    #[cfg(test)]
    pub(crate) fn saved_receipt(&self) -> Option<Receipt> {
        self.catalog
            .state
            .lock()
            .unwrap()
            .prices
            .get(&self.operation)
            .and_then(|p| p.receipt)
    }
    pub(crate) fn learn(&self, amount: U256, receipt: Receipt) {
        if amount == U256::ZERO {
            return;
        }
        let mut state = self.catalog.state.lock().expect("catalog hints poisoned");
        if !state.valid || !state.metadata.fresh() {
            return;
        }
        let old = state.prices.get(&self.operation);
        if old
            .and_then(|p| p.receipt)
            .is_some_and(|previous| receipt <= previous)
        {
            return;
        }
        let Some(version) = old.map_or(0, |p| p.version).checked_add(1) else {
            return;
        };
        state.prices.insert(
            self.operation.clone(),
            Hint {
                amount,
                version,
                receipt: Some(receipt),
            },
        );
        drop(state);
        let key = self.clone();
        tokio::spawn(async move {
            key.catalog
                .slot
                .persist_price(&key.catalog.generation, &key.operation, amount, version)
                .await;
        });
    }
}

#[cfg(test)]
pub(crate) async fn fixture(directory: &std::path::Path) -> Arc<CatalogHints> {
    let cfg = crate::catalog::Config {
        spec: "https://fixture.example/catalog".into(),
        http_cache_directory: Some(directory.join("http-cache")),
        ..Default::default()
    };
    let slot =
        crate::http_cache::Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let hints = slot
        .write(Some(crate::http_cache::Entry {
            hints: None,
            metadata: crate::http_cache::Metadata::from_catalog_headers(
                &reqwest::header::HeaderMap::new(),
                std::time::Duration::ZERO,
                None,
                cfg.catalog_cache_ttl_seconds,
            )
            .unwrap(),
            data: b"{}".to_vec(),
        }))
        .await;
    hints.unwrap()
}
