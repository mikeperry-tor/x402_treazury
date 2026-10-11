//! Ephemeral estimates from normal, validated payment challenges.
//! This cache never owns authority, I/O, catalog definitions or payment work.
use alloy_primitives::U256;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

/// Five minutes from receipt, never from validation, admission or cache access.
pub(crate) const TTL: Duration = Duration::from_secs(300);
pub(crate) const CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationKey([u8; 32]);
impl OperationKey {
    /// All inputs come from the captured trusted runtime binding. In particular,
    /// `template` is the OpenAPI template, never the interpolated request URL.
    pub(crate) fn new(
        source: &str,
        revision: u64,
        method: &str,
        template: &str,
        group: &str,
        transport_scope: &[u8],
    ) -> Self {
        let revision = revision.to_be_bytes();
        let method = method.to_ascii_uppercase();
        let mut hash = Sha256::new();
        for field in [
            b"runtime-price-hint-v1".as_slice(),
            source.as_bytes(),
            &revision,
            method.as_bytes(),
            template.as_bytes(),
            group.as_bytes(),
            transport_scope,
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        Self(hash.finalize().into())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Receipt {
    at: Instant,
    sequence: u64,
}
/// Capture immediately on receiving the unsigned HTTP response, before any
/// asynchronous challenge handling. Overflow disables learning, never payment.
pub(crate) fn received() -> Option<Receipt> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let at = Instant::now();
    let mut sequence = SEQUENCE.load(Ordering::Relaxed);
    loop {
        let next = sequence.checked_add(1)?;
        match SEQUENCE.compare_exchange_weak(sequence, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => sequence = current,
        }
    }
    Some(Receipt { at, sequence })
}

struct Entry {
    key: OperationKey,
    amount: U256,
    receipt: Receipt,
}
pub(crate) struct RuntimeHints {
    entries: Mutex<Vec<Entry>>,
}
impl Default for RuntimeHints {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Vec::with_capacity(CAPACITY)),
        }
    }
}
pub(crate) fn process_hints() -> &'static RuntimeHints {
    static HINTS: OnceLock<RuntimeHints> = OnceLock::new();
    HINTS.get_or_init(RuntimeHints::default)
}
impl RuntimeHints {
    #[cfg(test)]
    pub(crate) fn saved_receipt(&self, key: OperationKey) -> Option<Receipt> {
        self.entries
            .lock()
            .expect("runtime hints lock poisoned")
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.receipt)
    }
    pub(crate) fn get(&self, key: OperationKey) -> Option<U256> {
        self.get_at(key, Instant::now())
    }
    fn get_at(&self, key: OperationKey, now: Instant) -> Option<U256> {
        self.entries
            .lock()
            .expect("runtime hints lock poisoned")
            .iter()
            .find(|entry| entry.key == key && now.saturating_duration_since(entry.receipt.at) < TTL)
            .map(|entry| entry.amount)
    }
    /// Called only after selection and full validation of the actual unsigned
    /// offer. A later receipt supersedes an earlier one even if admission fails.
    pub(crate) fn learn(&self, key: OperationKey, amount: U256, receipt: Receipt) {
        self.learn_at(key, amount, receipt, Instant::now());
    }
    fn learn_at(&self, key: OperationKey, amount: U256, receipt: Receipt, now: Instant) {
        if amount == U256::ZERO || now.saturating_duration_since(receipt.at) >= TTL {
            return;
        }
        let mut entries = self.entries.lock().expect("runtime hints lock poisoned");
        if let Some(entry) = entries.iter_mut().find(|entry| entry.key == key) {
            if receipt > entry.receipt {
                *entry = Entry {
                    key,
                    amount,
                    receipt,
                };
            }
            return;
        }
        if entries.len() == CAPACITY {
            let oldest = entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.receipt)
                .map(|(index, _)| index)
                .expect("nonempty bounded hint cache");
            // Keep the newest receipts, including when old validation finishes
            // after eviction. No per-key tombstones or second retention bound.
            if receipt <= entries[oldest].receipt {
                return;
            }
            entries[oldest] = Entry {
                key,
                amount,
                receipt,
            };
        } else {
            entries.push(Entry {
                key,
                amount,
                receipt,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(revision: u64) -> OperationKey {
        OperationKey::new("source", revision, "GET", "/item/{id}", "group", b"direct")
    }
    #[test]
    fn scope_is_fixed_size_and_separates_trusted_bindings() {
        let original = key(1);
        assert_eq!(std::mem::size_of_val(&original), 32);
        assert_eq!(
            original,
            OperationKey::new("source", 1, "get", "/item/{id}", "group", b"direct")
        );
        for changed in [
            key(2),
            OperationKey::new("other", 1, "GET", "/item/{id}", "group", b"direct"),
            OperationKey::new("source", 1, "POST", "/item/{id}", "group", b"direct"),
            OperationKey::new("source", 1, "GET", "/other/{id}", "group", b"direct"),
            OperationKey::new("source", 1, "GET", "/item/{id}", "other", b"direct"),
            OperationKey::new("source", 1, "GET", "/item/{id}", "group", b"tor"),
        ] {
            assert_ne!(original, changed);
        }
    }
    #[test]
    fn receipt_order_not_completion_order_controls_downward_updates() {
        let cache = RuntimeHints::default();
        let at = Instant::now();
        cache.learn_at(key(1), U256::from(2), Receipt { at, sequence: 2 }, at);
        cache.learn_at(key(1), U256::from(8), Receipt { at, sequence: 1 }, at);
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(2)));
        cache.learn_at(key(1), U256::from(1), Receipt { at, sequence: 3 }, at);
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(1)));
        cache.learn_at(key(0), U256::from(9), Receipt { at, sequence: 4 }, at);
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(1)));
    }
    #[test]
    fn preemption_between_clock_and_sequence_does_not_reverse_receipt_order() {
        let cache = RuntimeHints::default();
        let at = Instant::now();
        let later = at + Duration::from_millis(1);
        cache.learn_at(
            key(1),
            U256::from(2),
            Receipt {
                at: later,
                sequence: 1,
            },
            later,
        );
        cache.learn_at(key(1), U256::from(8), Receipt { at, sequence: 2 }, later);
        assert_eq!(cache.get_at(key(1), later), Some(U256::from(2)));
    }
    #[test]
    fn reads_and_late_validation_never_renew_receipt_lifetime() {
        let cache = RuntimeHints::default();
        let at = Instant::now();
        let receipt = Receipt { at, sequence: 0 };
        cache.learn_at(key(1), U256::from(2), receipt, at + TTL / 2);
        assert!(
            cache
                .get_at(key(1), at + TTL - Duration::from_nanos(1))
                .is_some()
        );
        assert!(cache.get_at(key(1), at + TTL).is_none());
        cache.learn_at(key(2), U256::from(2), receipt, at + TTL);
        assert!(cache.get_at(key(2), at + TTL).is_none());
    }
    #[test]
    fn bounded_storage_has_no_unbounded_keys_or_late_resurrection() {
        let cache = RuntimeHints::default();
        let at = Instant::now();
        for sequence in 0..(CAPACITY as u64 * 3) {
            cache.learn_at(key(sequence), U256::from(2), Receipt { at, sequence }, at);
        }
        cache.learn_at(key(0), U256::from(9), Receipt { at, sequence: 0 }, at);
        assert_eq!(cache.entries.lock().unwrap().len(), CAPACITY);
        assert!(cache.get_at(key(0), at).is_none());
        assert!(cache.get_at(key(CAPACITY as u64 * 3 - 1), at).is_some());
        // Fixed entries in one preallocated Vec: no URL/string allocations,
        // per-entry map nodes, mutable byte ledger or active-work ownership.
        assert!(std::mem::size_of::<Entry>() * CAPACITY <= 64 * 1024);
        assert_eq!(cache.entries.lock().unwrap().capacity(), CAPACITY);
    }
}
