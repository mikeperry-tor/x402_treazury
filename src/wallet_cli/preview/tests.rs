//! Public metadata fixtures only: no SDK wallet decoding, credentials or I/O clients.
use super::*;
use crate::rotation::store::{AllocationKind, Store, test_support::reserves};
use serde_json::{Value, json};
use std::collections::BTreeSet;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("wallet.key"),
            1,
            b"public synthetic snapshot bytes",
        )
        .unwrap();
        Self { dir, store }
    }
    fn config(&self) -> Value {
        json!({
            "version": 1,
            "treasury": {"state_dir":"state", "key_file":"missing.key", "indexer_url_env":"PREVIEW_ABSENT_INDEXER"},
            "funding": {"auto_fund":false,"near_user_session_env":"PREVIEW_ABSENT_NEAR","base_rpc_url_env":"PREVIEW_ABSENT_BASE"},
            "wallets": {"current":{"mode":"zcash_rotation","max_api_payment_usdc":"1", "reserves":{"small":{"funding_amount_usdc":"2"}}}},
            "sources": {"one":{"spec":"never-read.json","wallet":"current", "send_tools":[{"to":["first","second"]}]}},
            "servers": {
                "first":{"listen":"127.0.0.1:0","bearer_token_env":"PREVIEW_ABSENT_TOKEN","discover_on_demand":true,"wallet":"current"},
                "second":{"listen":"127.0.0.1:0","bearer_token_env":"PREVIEW_ABSENT_TOKEN","discover_on_demand":true,"wallet":"current"}
            }
        })
    }
    fn report(&self, config: &Value) -> Result<Report> {
        let parsed: MetaConfig = serde_json::from_value(config.clone()).unwrap();
        let path = self.dir.path().join("preview.toml");
        std::fs::write(&path, toml::to_string(&parsed).unwrap()).unwrap();
        report(&path, None)
    }
    fn declare(&mut self, name: &str) -> String {
        self.store
            .ensure_capacity_group(name, &reserves("2"), Some(U256::from(1_000_000)), 1000)
            .unwrap()
    }
    fn logical_state(&self) -> Vec<(String, Vec<String>)> {
        let db = rusqlite::Connection::open_with_flags(
            self.dir.path().join("state/state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let tables = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                let mut stmt = db
                    .prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
                    .unwrap();
                let columns = stmt.column_count();
                let mut rows = stmt
                    .query_map([], |row| {
                        (0..columns)
                            .map(|column| row.get::<_, rusqlite::types::Value>(column))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .map(|row| format!("{:?}", row.unwrap()))
                    .collect::<Vec<_>>();
                rows.sort();
                (table, rows)
            })
            .collect()
    }
}

#[test]
fn bootstrap_preview_separates_setup_from_eventual_inventory_without_demand() {
    let mut fixture = Fixture::new();
    fixture.declare("current");
    fixture.declare("participant");
    let mut config = fixture.config();
    config["wallets"]["participant"] = config["wallets"]["current"].clone();
    config["wallet_bootstrap"] =
        json!({"bootstrap_group":"current", "deferred_groups":["participant"]});
    let before = fixture.logical_state();
    let report = fixture.report(&config).unwrap();
    let setup = report.summary.bootstrap_setup.as_ref().unwrap();
    assert_eq!(setup.group, "current");
    assert_eq!(setup.initial_inventory_target_atomic, "2000000");
    assert_eq!(setup.initial_inventory_maximum_atomic, "2000000");
    assert_eq!(report.summary.initial_inventory_target_atomic, "4000000");
    assert_eq!(report.summary.initial_inventory_maximum_atomic, "4000000");
    assert_eq!(
        report.wallet_bootstrap.as_ref().unwrap().deferred_groups,
        ["participant"]
    );
    let human = report.human();
    assert!(human.contains("Eventual nominal initial reserve sizing: 4.000000–4.000000"));
    assert!(human.contains("Explicit bootstrap setup: group current, 2.000000–2.000000"));
    assert!(human.contains("first positive initial credit"));
    assert_eq!(before, fixture.logical_state());
}

#[test]
fn inventory_classification_is_read_only_without_keys_and_while_owned() {
    let mut fixture = Fixture::new();
    let current = fixture.declare("current");
    fixture
        .store
        .allocate_capacity_kind(&current, AllocationKind::Small, true)
        .unwrap();
    let job = fixture.store.funding_jobs().unwrap().remove(0);
    let quote =
        serde_json::to_vec(&json!({"request":{"amount":job.target,"recipient":job.recipient}}))
            .unwrap();
    fixture.store.save_funding_quote(&job.id, &quote).unwrap();
    fixture
        .store
        .accept_preparation(&job.operation_id, Some(&current), 10, None)
        .unwrap();
    fixture
        .store
        .prepare(
            &job.operation_id,
            fixture.store.snapshot().unwrap().0,
            b"accepted snapshot",
            b"exact pending bytes",
        )
        .unwrap();
    fixture.declare("disabled");
    fixture.store.disable_pool("disabled").unwrap();
    fixture.declare("omitted");
    fixture.declare("already_disabled");
    fixture.store.disable_pool("already_disabled").unwrap();
    // An owner remains alive, but neither the configured key nor the original key
    // exists on disk. Public preview must not authenticate/decrypt SDK bytes.
    std::fs::remove_file(fixture.dir.path().join("wallet.key")).unwrap();
    let mut config = fixture.config();
    config["wallets"]["disabled"] = config["wallets"]["current"].clone();
    config["wallets"]["fresh_unused"] = config["wallets"]["current"].clone();
    config["wallets"]["current"]["max_api_payment_usdc"] = json!("3");
    config["wallets"]["current"]["reserves"]["small"]["funding_amount_usdc"] = json!("4");
    let before = fixture.logical_state();
    let report = fixture.report(&config).unwrap();
    assert_eq!(report.managed["current"].action, "reuse");
    assert_eq!(
        report.managed["current"].saved_id.as_deref(),
        Some(current.as_str())
    );
    assert_eq!(
        report.managed["current"].changed_saved_policy_fields,
        ["payment_limit_atomic", "reserves.small"]
    );
    assert_eq!(report.managed["disabled"].action, "reactivation");
    assert!(
        report.managed["disabled"]
            .changed_saved_policy_fields
            .is_empty()
    );
    assert_eq!(report.managed["fresh_unused"].action, "new");
    assert_eq!(report.omitted["omitted"].action, "would_disable");
    assert_eq!(
        report.omitted["already_disabled"].action,
        "retained_disabled"
    );
    assert!(report.startup_conflicts.is_empty());
    assert_eq!(report.saved.funding_jobs.len(), 1);
    assert_eq!(
        report
            .saved
            .pools
            .iter()
            .find(|p| p.id == current)
            .unwrap()
            .addresses
            .len(),
        1
    );
    assert_eq!(report.bindings["first"]["one"].wallet, "current");
    assert_eq!(report.bindings["second"]["one"].wallet, "current");
    let human = report.human();
    assert!(human.contains("advisory observations"));
    assert!(human.contains("fresh_unused: new"));
    assert!(human.contains("Saved inventory and pending work"));
    let json = serde_json::to_string(&report).unwrap();
    let again = fixture.report(&config).unwrap();
    assert_eq!(human, again.human());
    assert_eq!(json, serde_json::to_string(&again).unwrap());
    assert_eq!(before, fixture.logical_state());
    assert!(!fixture.dir.path().join("missing.key").exists());
    assert!(!fixture.dir.path().join("never-read.json").exists());
}

#[test]
fn static_only_retains_enabled_state_but_disabled_names_still_conflict() {
    let mut fixture = Fixture::new();
    fixture.declare("current");
    fixture.declare("disabled");
    fixture.store.disable_pool("disabled").unwrap();
    let mut config = fixture.config();
    config["wallets"] =
        json!({"static_payer":{"mode":"static","private_key_env":"PREVIEW_ABSENT_KEY"}});
    config["servers"]["first"]["wallet"] = json!("static_payer");
    config["servers"]["second"]["wallet"] = json!("static_payer");
    config["sources"]["one"]["wallet"] = json!("static_payer");
    let before = fixture.logical_state();
    let report = fixture.report(&config).unwrap();
    assert!(!report.configure_managed_profiles);
    assert!(report.startup_conflicts.is_empty());
    assert_eq!(
        report.omitted["current"].action,
        "retained_unchanged_static_only"
    );
    assert_eq!(
        report.omitted["disabled"].action,
        "retained_unchanged_static_only"
    );
    config["wallets"]["disabled"] = config["wallets"]["static_payer"].clone();
    let conflict = fixture.report(&config).unwrap();
    assert_eq!(conflict.startup_conflicts.len(), 1);
    assert!(conflict.startup_conflicts[0].contains("disabled"));
    assert!(conflict.human().contains("Startup conflict:"));
    assert_eq!(before, fixture.logical_state());
}

#[test]
fn generated_bindings_follow_existing_resolution_and_explicit_overrides() {
    let mut fixture = Fixture::new();
    fixture.declare("auto_v1_server_old");
    let mut config = fixture.config();
    config["wallet_templates"] = json!({"reserve":config["wallets"]["current"].clone()});
    config["wallet_assignment"] = json!({"scope":"server","template":"reserve"});
    config["servers"]["first"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    config["servers"]["second"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    config["sources"]["one"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    config["sources"]["override"] =
        json!({"wallet":"current","spec":"also-never-read.json","send_tools":[{"to":"first"}]});
    let report = fixture.report(&config).unwrap();
    assert_eq!(
        report.bindings["first"]["one"].wallet,
        "auto_v1_server_first"
    );
    assert_eq!(
        report.bindings["second"]["one"].wallet,
        "auto_v1_server_second"
    );
    assert_eq!(report.bindings["first"]["override"].wallet, "current");
    assert_eq!(report.generated.len(), 2);
    assert_eq!(report.summary.managed_pool_count, 3);
    assert_eq!(report.omitted["auto_v1_server_old"].action, "would_disable");
    assert_eq!(report.managed["auto_v1_server_first"].action, "new");
}

#[test]
fn preview_of_renamed_groups_agrees_with_normal_profile_configuration() {
    let mut fixture = Fixture::new();
    let original = fixture.declare("old");
    fixture
        .store
        .rename_capacity_group("old", "current")
        .unwrap();
    let config = fixture.config();
    let preview = fixture.report(&config).unwrap();
    assert_eq!(
        preview.managed["current"].saved_id.as_deref(),
        Some(original.as_str())
    );
    fixture
        .store
        .configure_profiles(&BTreeSet::from(["current".into()]), &BTreeSet::new())
        .unwrap();
    assert_eq!(fixture.declare("current"), original);
    fixture.declare("occupied");
    assert!(
        fixture
            .store
            .rename_capacity_group("current", "occupied")
            .is_err()
    );
    let preview = fixture.report(&config).unwrap();
    assert_eq!(
        preview.managed["current"].saved_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(preview.omitted["occupied"].action, "would_disable");
}

#[test]
fn missing_unsupported_or_mismatched_state_is_rejected_without_initializing() {
    let fixture = Fixture::new();
    let mut config = fixture.config();
    config["treasury"]["state_dir"] = json!("uninitialized");
    let err = fixture.report(&config).err().unwrap();
    assert!(format!("{err:#}").contains("cannot inspect initialized treasury"));
    assert!(!fixture.dir.path().join("uninitialized").exists());
    config["treasury"]["state_dir"] = json!("state");
    config["treasury"]["id"] = json!(uuid::Uuid::new_v4().to_string());
    let before = fixture.logical_state();
    assert!(format!("{:#}", fixture.report(&config).err().unwrap()).contains("identity mismatch"));
    assert_eq!(before, fixture.logical_state());
    config["treasury"].as_object_mut().unwrap().remove("id");
    let db = rusqlite::Connection::open(fixture.dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=999").unwrap();
    assert!(
        format!("{:#}", fixture.report(&config).err().unwrap())
            .contains("unsupported state schema")
    );
    db.execute_batch("PRAGMA user_version=23; PRAGMA ignore_check_constraints=ON; UPDATE instance SET network='unsupported'").unwrap();
    assert!(
        format!("{:#}", fixture.report(&config).err().unwrap())
            .contains("identity/network/schema mismatch")
    );
    db.execute_batch("DELETE FROM instance").unwrap();
    assert!(
        format!("{:#}", fixture.report(&config).err().unwrap()).contains("missing state identity")
    );
}
