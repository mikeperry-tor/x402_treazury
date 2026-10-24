//! Ephemeral estimates from normal, validated payment challenges.
//! This cache never owns authority, I/O, catalog definitions or payment work.
use alloy_primitives::U256;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

/// From receipt, never from validation, admission or cache access. Shares the
/// catalog disk-cache default; per-source catalog overrides remain independent.
pub(crate) const TTL: Duration = Duration::from_secs(crate::limits::API_CACHE_TTL_SECONDS);
pub(crate) const CAPACITY: usize = 65_535;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    amount: U256,
    receipt: Receipt,
}
#[derive(Default)]
struct Entries {
    by_key: HashMap<OperationKey, Entry>,
    by_receipt: BTreeSet<(Receipt, OperationKey)>,
}
pub(crate) struct RuntimeHints {
    entries: Mutex<Entries>,
}
impl Default for RuntimeHints {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
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
            .by_key
            .get(&key)
            .map(|entry| entry.receipt)
    }
    pub(crate) fn get(&self, key: OperationKey) -> Option<U256> {
        self.get_at(key, Instant::now())
    }
    fn get_at(&self, key: OperationKey, now: Instant) -> Option<U256> {
        self.entries
            .lock()
            .expect("runtime hints lock poisoned")
            .by_key
            .get(&key)
            .filter(|entry| now.saturating_duration_since(entry.receipt.at) < TTL)
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
        if let Some(previous) = entries.by_key.get(&key).map(|entry| entry.receipt) {
            if receipt <= previous {
                return;
            }
            entries.by_receipt.remove(&(previous, key));
        } else if entries.by_key.len() == CAPACITY {
            let &(oldest, oldest_key) = entries
                .by_receipt
                .first()
                .expect("nonempty bounded hint cache");
            // Keep the newest receipts, including when old validation finishes
            // after eviction. No per-key tombstones or second retention bound.
            if receipt <= oldest {
                tracing::warn!(
                    limit_entries = CAPACITY,
                    "Runtime price hint capacity reached; older incoming hint discarded"
                );
                return;
            }
            tracing::warn!(
                limit_entries = CAPACITY,
                "Runtime price hint capacity reached; oldest receipt evicted"
            );
            entries.by_receipt.remove(&(oldest, oldest_key));
            entries.by_key.remove(&oldest_key);
        }
        // Both indexes change under the same lock. Each key has exactly one
        // receipt entry, including after replacement or capacity eviction.
        entries.by_key.insert(key, Entry { amount, receipt });
        entries.by_receipt.insert((receipt, key));
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
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let cache = RuntimeHints::default();
        let at = Instant::now();
        for sequence in 0..CAPACITY as u64 {
            cache.learn_at(key(sequence), U256::from(2), Receipt { at, sequence }, at);
        }
        let newest = CAPACITY as u64;
        cache.learn_at(
            key(newest),
            U256::from(2),
            Receipt {
                at,
                sequence: newest,
            },
            at,
        );
        cache.learn_at(key(0), U256::from(9), Receipt { at, sequence: 0 }, at);
        assert!(cache.get_at(key(0), at).is_none());
        assert!(cache.get_at(key(newest), at).is_some());
        // Updating the oldest surviving key must move its eviction position,
        // without leaving an older receipt behind or increasing cache size.
        let updated = Receipt {
            at,
            sequence: newest + 1,
        };
        cache.learn_at(key(1), U256::from(3), updated, at);
        cache.learn_at(key(1), U256::from(9), updated, at);
        cache.learn_at(key(1), U256::from(9), Receipt { at, sequence: 1 }, at);
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(3)));
        cache.learn_at(
            key(newest + 1),
            U256::from(4),
            Receipt {
                at,
                sequence: newest + 2,
            },
            at,
        );
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(3)));
        assert!(cache.get_at(key(2), at).is_none());
        // A different key at exactly the oldest receipt cannot displace it.
        cache.learn_at(key(2), U256::from(9), Receipt { at, sequence: 3 }, at);
        assert!(cache.get_at(key(2), at).is_none());
        assert_eq!(cache.get_at(key(3), at), Some(U256::from(2)));
        let entries = cache.entries.lock().unwrap();
        assert_eq!(entries.by_key.len(), CAPACITY);
        assert_eq!(entries.by_receipt.len(), CAPACITY);
        let logs = std::fs::read_to_string(log.path()).unwrap();
        for message in ["older incoming hint discarded", "oldest receipt evicted"] {
            assert!(
                logs.lines()
                    .any(|line| line.contains("WARN") && line.contains(message))
            );
        }
        assert!(logs.contains("limit_entries=65535"));
        assert!(!logs.contains("source"));
        for (key, entry) in &entries.by_key {
            assert!(entries.by_receipt.contains(&(entry.receipt, *key)));
        }
    }

    #[test]
    fn equal_receipts_for_distinct_keys_survive_independent_replacement() {
        let cache = RuntimeHints::default();
        let at = Instant::now();
        let original = Receipt { at, sequence: 1 };
        cache.learn_at(key(1), U256::from(2), original, at);
        cache.learn_at(key(2), U256::from(3), original, at);
        let newer = Receipt { at, sequence: 2 };
        cache.learn_at(key(1), U256::from(1), newer, at);
        assert_eq!(cache.get_at(key(1), at), Some(U256::from(1)));
        assert_eq!(cache.get_at(key(2), at), Some(U256::from(3)));
        let entries = cache.entries.lock().unwrap();
        assert_eq!(entries.by_key.len(), 2);
        assert_eq!(entries.by_receipt.len(), 2);
        assert!(entries.by_receipt.contains(&(newer, key(1))));
        assert!(entries.by_receipt.contains(&(original, key(2))));
    }
}
