use super::*;
use crate::pricing::runtime::{OperationKey, received};
use alloy_primitives::U256;
use std::sync::Arc;

fn config(root: &Path) -> crate::catalog::Config {
    crate::catalog::Config {
        spec: "https://catalog.example/spec".into(),
        http_cache_directory: Some(root.join("http-cache")),
        discovery_source: Some("first".into()),
        ..Default::default()
    }
}
fn slot(cfg: &crate::catalog::Config) -> Slot {
    Slot::new(cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap()
}
fn entry(data: &[u8]) -> Entry {
    Entry {
        hints: None,
        metadata: Metadata::from_catalog_headers(
            &HeaderMap::new(),
            Duration::ZERO,
            None,
            crate::catalog::CacheLifetime::Indefinite,
        )
        .unwrap(),
        data: data.to_vec(),
    }
}
async fn persist(key: &OperationKey) {
    let (amount, version) = {
        let state = key.catalog.state.lock().unwrap();
        let hint = &state.prices[&key.operation];
        (hint.amount, hint.version)
    };
    key.catalog
        .slot
        .persist_price(&key.catalog.generation, &key.operation, amount, version)
        .await;
}
fn forget(slot: &Slot) {
    registry()
        .lock()
        .unwrap()
        .remove(&(slot.directory.clone(), slot.key.clone()));
}

#[derive(Debug)]
pub(super) struct CommitHold {
    entered: tokio::sync::Notify,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl CommitHold {
    pub(super) fn wait(&self) {
        self.entered.notify_one();
        // Dropping the test's sender also releases a worker after test failure.
        let _ = self.release.lock().unwrap().recv();
    }
}

#[tokio::test]
async fn dispatched_pricing_write_cannot_commit_after_retirement() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let mut slot = slot(&cfg);
    let current = slot.write(Some(entry(b"current"))).await.unwrap();
    let observed = current.operation("GET", "/read");
    observed.learn(U256::from(7), received().unwrap());
    let mut tools = crate::catalog::build_tools(
        &cfg,
        &serde_json::json!({"paths":{"/read":{"get":{}}}}),
        "fixture",
    )
    .unwrap();
    let binding = crate::catalog_refresh::Binding::new(|_| {});
    binding.own_prices(&tools);
    let owner = tools.remove(0).advertised_pricing;
    let (release, receive) = std::sync::mpsc::channel();
    let hold = Arc::new(CommitHold {
        entered: Default::default(),
        release: std::sync::Mutex::new(receive),
    });
    slot.before_commit = Some(hold.clone());
    let writer = tokio::spawn({
        let slot = slot.clone();
        let owner = owner.clone();
        async move {
            slot.write_if_current(Some(entry(b"obsolete")), Some(owner))
                .await
        }
    });
    hold.entered.notified().await;
    binding.retire();
    release.send(()).unwrap();
    assert!(writer.await.unwrap().is_none());
    slot.before_commit = None;
    let saved = slot.read().await.unwrap();
    assert_eq!(saved.data, b"current");
    assert!(Arc::ptr_eq(saved.hints.as_ref().unwrap(), &current));
    assert_eq!(observed.get(), Some(U256::from(7)));

    // A distinct active alias still replaces the cache. Retirement after its
    // completed commit cannot retroactively roll back accepted metadata.
    let active = crate::pricing::advertised::AdvertisedPricing::default();
    assert!(
        slot.write_if_current(Some(entry(b"active alias")), Some(active.clone()))
            .await
            .is_some()
    );
    active.invalidate();
    assert_eq!(slot.read().await.unwrap().data, b"active alias");
}

#[tokio::test]
async fn prices_restore_share_expire_and_never_renew_catalogs() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let slot = slot(&cfg);
    assert_eq!(
        cfg.catalog_cache_ttl_seconds,
        crate::catalog::CacheLifetime::Indefinite
    );
    let raw = b"{ \"paths\": {} }\n";
    let hints = slot.write(Some(entry(raw))).await.unwrap();
    let key = hints.operation("get", "/items/{id}");
    let expiry = hints.state.lock().unwrap().metadata.expires;
    let older = received().unwrap();
    let newer = received().unwrap();
    key.learn(U256::from(2), newer);
    key.learn(U256::from(9), older);
    assert_eq!(key.get(), Some(U256::from(2)));
    assert!(hints.operation("POST", "/items/{id}").get().is_none());
    assert!(hints.operation("GET", "/other").get().is_none());
    persist(&key).await;
    // An older detached write cannot replace an already persisted newer one.
    slot.persist_price(&hints.generation, &key.operation, U256::from(9), 0)
        .await;
    let saved = slot.read().await.unwrap();
    assert_eq!(saved.data, raw);
    assert_eq!(saved.metadata.expires, expiry);
    assert!(Arc::ptr_eq(saved.hints.as_ref().unwrap(), &hints));
    forget(&slot); // fresh-process hydration, without changing disk or its clock
    let restored = slot.read().await.unwrap().hints.unwrap();
    assert!(!Arc::ptr_eq(&hints, &restored));
    assert_eq!(
        restored.operation("GET", "/items/{id}").get(),
        Some(U256::from(2))
    );
    assert_eq!(restored.state.lock().unwrap().metadata.expires, expiry);
    let mut expired = slot.read().await.unwrap();
    let expired_at = now();
    expired.metadata.expires = expired_at;
    slot.write(Some(expired)).await;
    let restored_key = restored.operation("GET", "/items/{id}");
    restored_key.learn(U256::from(7), received().unwrap());
    assert!(restored_key.get().is_none());
    slot.persist_price(&restored.generation, &key.operation, U256::from(7), 99)
        .await;
    let db = open(&slot.directory).unwrap();
    assert_eq!(
        db.query_row("SELECT amount FROM prices WHERE key=?1", [&slot.key], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "2"
    );
    assert_eq!(slot.read().await.unwrap().metadata.expires, expired_at);
}

#[tokio::test]
async fn unchanged_aliases_keep_prices_configuration_and_content_changes_retire_them() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let first = slot(&cfg);
    let hints = first.write(Some(entry(b"{}"))).await.unwrap();
    let key = hints.operation("GET", "/items");
    key.learn(U256::from(4), received().unwrap());
    persist(&key).await;
    let mut alias = cfg.clone();
    alias.discovery_source = Some("second".into());
    alias.prefix = Some("other".into());
    alias.base_url = Some("https://other.example".into());
    alias.include = vec!["/items".into()];
    let second = slot(&alias);
    assert_eq!(first.key, second.key);
    second.record_configuration(&alias).await;
    second.check_revision().await;
    let same = second.write(Some(entry(b"{}"))).await.unwrap();
    assert!(Arc::ptr_eq(&hints, &same));
    assert_eq!(key.get(), Some(U256::from(4)));
    forget(&first);
    alias.base_url = Some("https://updated.example".into());
    let changed = slot(&alias);
    changed.check_revision().await;
    assert!(changed.read().await.is_none());
    first
        .persist_price(&hints.generation, &key.operation, U256::from(99), 99)
        .await;
    let replacement = changed.write(Some(entry(b"{}"))).await.unwrap();
    assert!(replacement.operation("GET", "/items").get().is_none());
    assert_ne!(hints.generation, replacement.generation);
    let newkey = replacement.operation("GET", "/items");
    newkey.learn(U256::from(3), received().unwrap());
    persist(&newkey).await;
    let replaced = changed.write(Some(entry(b"{\"paths\":{}}"))).await.unwrap();
    assert!(newkey.get().is_none());
    assert!(replaced.operation("GET", "/items").get().is_none());
    changed
        .persist_price(
            &replacement.generation,
            &newkey.operation,
            U256::from(99),
            999,
        )
        .await;
    let db = open(&changed.directory).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM prices", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    changed.write(None).await;
    assert!(replaced.operation("GET", "/items").get().is_none());
    assert_eq!(
        open(&changed.directory)
            .unwrap()
            .query_row("SELECT count(*) FROM catalog_revisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn absent_evicted_and_failed_storage_never_recreate_observations() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let slot = slot(&cfg);
    let hints = slot.write(Some(entry(b"{}"))).await.unwrap();
    let key = hints.operation("GET", "/items");
    key.learn(U256::from(3), received().unwrap());
    persist(&key).await;
    slot.write(None).await;
    slot.persist_price(&hints.generation, &key.operation, U256::from(9), 99)
        .await;
    assert!(slot.read().await.is_none());
    assert!(key.get().is_none());
    let hints = slot.write(Some(entry(b"{}"))).await.unwrap();
    let key = hints.operation("GET", "/items");
    key.learn(U256::from(3), received().unwrap());
    persist(&key).await;
    {
        let db = open(&slot.directory).unwrap();
        db.execute("UPDATE entries SET stored=0 WHERE key=?1", [&slot.key])
            .unwrap();
        db.execute("WITH RECURSIVE rows(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM rows WHERE x<4095) INSERT INTO entries SELECT 'filler-'||x,'{}',x'7b7d',1 FROM rows",[]).unwrap();
    }
    let mut pressure = slot.clone();
    pressure.key = "pressure".into();
    pressure.write(Some(entry(b"{}"))).await;
    assert!(key.get().is_none());
    assert!(slot.read().await.is_none());
    slot.persist_price(&hints.generation, &key.operation, U256::from(99), 99)
        .await;
    assert!(slot.read().await.is_none());
    let hints = slot.write(Some(entry(b"{}"))).await.unwrap();
    let key = hints.operation("GET", "/items");
    // Fail price storage without renaming files under detached SQLite tasks.
    open(&slot.directory).unwrap().execute_batch("CREATE TRIGGER reject_prices BEFORE INSERT ON prices BEGIN SELECT RAISE(FAIL,'fixture write failure'); END;").unwrap();
    key.learn(U256::from(5), received().unwrap());
    persist(&key).await;
    assert_eq!(key.get(), Some(U256::from(5)));
    // No database or persistence task exists for this missing-parent binding.
    let mut missing = slot.clone();
    missing.directory = root.path().join("absent/cache");
    let missing_hints = Arc::new(crate::pricing::runtime::CatalogHints {
        slot: missing.clone(),
        generation: "missing-fixture".into(),
        state: std::sync::Mutex::new(crate::pricing::runtime::State {
            metadata: entry(b"{}").metadata,
            valid: true,
            prices: std::collections::BTreeMap::from([(
                "GET /items".into(),
                crate::pricing::runtime::Hint {
                    amount: U256::from(5),
                    version: 1,
                    receipt: None,
                },
            )]),
        }),
    });
    registry().lock().unwrap().insert(
        (missing.directory.clone(), missing.key.clone()),
        Arc::downgrade(&missing_hints),
    );
    let missing_key = missing_hints.operation("GET", "/items");
    assert_eq!(missing_key.get(), Some(U256::from(5)));
    missing.write(None).await;
    assert!(
        missing_key.get().is_none(),
        "explicit invalidation retires memory even without a cache parent"
    );
    assert!(!missing.directory.exists());
    let mut disabled = cfg.clone();
    disabled.http_cache_enabled = false;
    assert!(Slot::new(&disabled, &cfg.spec, "catalog", cfg.max_spec_bytes).is_none());
    disabled.http_cache_enabled = true;
    disabled.catalog_cache_ttl_seconds = 0.into();
    assert!(Slot::new(&disabled, &cfg.spec, "catalog", cfg.max_spec_bytes).is_none());
    disabled.catalog_cache_ttl_seconds = 604800.into();
    disabled.http_cache_directory = None;
    assert!(Slot::new(&disabled, &cfg.spec, "catalog", cfg.max_spec_bytes).is_none());
}

#[tokio::test]
async fn existing_transport_relay_and_direct_warm_boundaries_also_own_prices() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let ordinary = slot(&cfg);
    let mut direct = ordinary.clone();
    direct.key = ordinary.direct_key.clone().unwrap();
    direct.direct_key = None;
    let warmed = direct.write(Some(entry(b"{}"))).await.unwrap();
    let key = warmed.operation("GET", "/items");
    key.learn(U256::from(2), received().unwrap());
    let reused = ordinary.read().await.unwrap();
    assert!(reused.metadata.direct_reuse);
    assert!(Arc::ptr_eq(reused.hints.as_ref().unwrap(), &warmed));
    let mut other = cfg.clone();
    other.allow_http1 = true;
    let isolated = slot(&other).write(Some(entry(b"{}"))).await.unwrap();
    assert!(isolated.operation("GET", "/items").get().is_none());
    let relayed = ordinary
        .clone()
        .relay("relay-a")
        .write(Some(entry(b"{}")))
        .await
        .unwrap();
    assert!(relayed.operation("GET", "/items").get().is_none());
    assert_ne!(ordinary.key, ordinary.clone().relay("relay-a").key);
}

#[tokio::test]
async fn oversized_disposable_catalog_eviction_retires_published_hints() {
    let root = tempfile::tempdir().unwrap();
    let mut cfg = config(root.path());
    cfg.max_spec_bytes = 4;
    let slot = slot(&cfg);
    let hints = slot.write(Some(entry(b"{}"))).await.unwrap();
    let key = hints.operation("GET", "/items");
    key.learn(U256::from(3), received().unwrap());
    persist(&key).await;
    open(&slot.directory)
        .unwrap()
        .execute(
            "UPDATE entries SET data=?2 WHERE key=?1",
            params![slot.key, b"large".as_slice()],
        )
        .unwrap();
    assert!(slot.read().await.is_none());
    assert!(key.get().is_none());
    key.learn(U256::from(9), received().unwrap());
    slot.persist_price(&hints.generation, &key.operation, U256::from(9), 99)
        .await;
    assert!(slot.read().await.is_none());
}

#[tokio::test]
async fn accepted_replacements_retire_direct_origin_and_relay_variants_together() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let ordinary = slot(&cfg);
    let mut warm_cfg = cfg.clone();
    warm_cfg.http_cache_direct_warm_target = Some(crate::network::global().policy.clone());
    let direct = slot(&warm_cfg);
    let relay = ordinary.clone().relay("synthetic-relay-position");
    assert_ne!(ordinary.key, direct.key);
    assert_eq!(ordinary.family, direct.family);
    assert_eq!(ordinary.family, relay.family);
    let first = ordinary.write(Some(entry(b"{\"old\":1}"))).await.unwrap();
    let direct_old = direct.write(Some(entry(b"{\"old\":2}"))).await.unwrap();
    let relay_old = relay.write(Some(entry(b"{\"old\":3}"))).await.unwrap();
    for hints in [&first, &direct_old, &relay_old] {
        hints
            .operation("GET", "/read")
            .learn(U256::from(1), received().unwrap());
    }
    let mut force_cfg = cfg.clone();
    force_cfg.catalog_cache_force_refresh = true;
    let replacement = slot(&force_cfg).replacement();
    replacement
        .write(Some(entry(b"{\"new\":1}")))
        .await
        .unwrap();
    assert!(direct.read_primary().await.is_none());
    assert!(relay.read_primary().await.is_none());
    assert_eq!(ordinary.read().await.unwrap().data, b"{\"new\":1}");
    for hints in [&first, &direct_old, &relay_old] {
        assert!(hints.operation("GET", "/read").get().is_none());
    }
    ordinary.write(None).await; // Eviction cannot uncover an obsolete direct entry.
    assert!(ordinary.read().await.is_none());

    ordinary.write(Some(entry(b"{\"old\":4}"))).await.unwrap();
    relay.write(Some(entry(b"{\"old\":5}"))).await.unwrap();
    direct
        .replacement()
        .write(Some(entry(b"{\"new\":2}")))
        .await
        .unwrap();
    assert!(ordinary.read_primary().await.is_none());
    assert!(relay.read_primary().await.is_none());
    let warmed = ordinary.read().await.unwrap();
    assert_eq!(warmed.data, b"{\"new\":2}");
    assert!(warmed.metadata.direct_reuse);

    // A successfully accepted uncacheable response retires old variants too.
    ordinary.replacement().write(None).await;
    assert!(ordinary.read().await.is_none());
    assert!(direct.read_primary().await.is_none());
}

// A deferred FK violation permits every statement to succeed, then fails the
// actual COMMIT. This checks memory publication rather than an early SQL error.
fn reject_commit(slot: &Slot, event: &str) {
    open(&slot.directory).unwrap().execute_batch(&format!(
        "CREATE TABLE commit_parent(id INTEGER PRIMARY KEY);
         CREATE TABLE commit_failure(id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER reject_commit AFTER {event} ON entries BEGIN INSERT INTO commit_failure VALUES(1); END;"
    )).unwrap();
}

#[tokio::test]
async fn failed_family_commit_preserves_primary_and_sibling_observations() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let primary = slot(&cfg);
    let sibling = primary.clone().relay("fixture");
    let first = primary.write(Some(entry(b"old"))).await.unwrap();
    let second = sibling.write(Some(entry(b"sibling"))).await.unwrap();
    for hints in [&first, &second] {
        hints
            .operation("GET", "/read")
            .learn(U256::from(7), received().unwrap());
    }
    reject_commit(&primary, "INSERT");
    assert!(
        primary
            .replacement()
            .write(Some(entry(b"new")))
            .await
            .is_none()
    );
    for (slot, hints, data) in [
        (&primary, &first, b"old".as_slice()),
        (&sibling, &second, b"sibling".as_slice()),
    ] {
        let saved = slot.read_primary().await.unwrap();
        assert_eq!(saved.data, data);
        assert!(Arc::ptr_eq(saved.hints.as_ref().unwrap(), hints));
        assert_eq!(hints.operation("GET", "/read").get(), Some(U256::from(7)));
    }
}

#[tokio::test]
async fn failed_unchanged_commit_does_not_renew_live_metadata() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let slot = slot(&cfg);
    let original = entry(b"same");
    let expiry = original.metadata.expires;
    let hints = slot.write(Some(original.clone())).await.unwrap();
    let key = hints.operation("GET", "/read");
    key.learn(U256::from(7), received().unwrap());
    reject_commit(&slot, "UPDATE");
    let mut renewed = original;
    renewed.metadata.expires -= 100;
    assert!(slot.write(Some(renewed)).await.is_none());
    assert_eq!(hints.state.lock().unwrap().metadata.expires, expiry);
    assert_eq!(key.get(), Some(U256::from(7)));
    let saved = slot.read().await.unwrap();
    assert_eq!(saved.metadata.expires, expiry);
    assert!(Arc::ptr_eq(saved.hints.as_ref().unwrap(), &hints));
}

#[tokio::test]
async fn failed_revision_and_oversize_deletions_preserve_live_observations() {
    for revision in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let mut cfg = config(root.path());
        cfg.max_spec_bytes = 4;
        let original = slot(&cfg);
        let hints = original.write(Some(entry(b"old"))).await.unwrap();
        let key = hints.operation("GET", "/read");
        key.learn(U256::from(7), received().unwrap());
        reject_commit(&original, "DELETE");
        if revision {
            cfg.base_url = Some("https://changed.example".into());
            slot(&cfg).check_revision().await;
        } else {
            open(&original.directory)
                .unwrap()
                .execute(
                    "UPDATE entries SET data=?2 WHERE key=?1",
                    params![original.key, b"oversized".as_slice()],
                )
                .unwrap();
            assert!(original.read().await.is_none());
        }
        assert_eq!(key.get(), Some(U256::from(7)));
        assert!(Arc::ptr_eq(existing(&original).as_ref().unwrap(), &hints));
        assert_eq!(
            open(&original.directory)
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM entries WHERE key=?1",
                    [&original.key],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}
