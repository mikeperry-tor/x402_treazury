//! Retired relay configuration remains readable only through the offline archive.
use super::*;
use crate::{
    planner,
    preparation::CatalogPins,
    registry::{Authorization, Pins},
};
use std::{collections::BTreeMap, io::Write};
use x402_treazury::{
    deployment::MetaConfig,
    network::{Mode, NetworkPolicy},
    rotation::{assignment, store::Store},
};

#[test]
fn old_relay_config_preserves_offline_managed_identity_audit_without_live_authority() {
    const CHILD: &str = "TREAZURY_ARCHIVE_CATEGORY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .env(CHILD, "1")
            .args(["--exact", "tor::archive::category_tests::old_relay_config_preserves_offline_managed_identity_audit_without_live_authority", "--nocapture"])
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    let store = Store::create(
        &state,
        &temporary.path().join("key"),
        1,
        b"offline-archive-fixture",
    )
    .unwrap();
    let treasury_id = store.id().to_owned();
    drop(store);
    let mut manifest = crate::tests::manifest();
    manifest.treasury_id = treasury_id.clone();
    manifest.evidence_dir = temporary.path().join("evidence");
    manifest.network.tor_mode = crate::manifest::TorMode::Owned;
    manifest.network.require_isolation_evidence = true;
    manifest.network.tor_binary = Some("synthetic-tor-not-executed".into());
    let network = NetworkPolicy {
        mode: Mode::Tor,
        socks_endpoint: Some("127.0.0.1:1".parse().unwrap()),
        ..Default::default()
    };
    let config: MetaConfig = serde_json::from_value(json!({
        "version":1,"network":network,
        "treasury":{"id":treasury_id,"state_dir":state,"indexer_url":"https://indexer.example.com","submission_url":"https://submit.example.com"},
        "funding":{"base_rpc_fallback_url_envs":[]},
        "wallets":{"pool":{"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"2"}},"max_api_payment_usdc":"0.02","max_funding_spend_zec":"0.02","max_conversion_overhead_percent":5}},
        "sources":{"api":{"spec":"local.json"}},
        "servers":{"main":{"listen":"127.0.0.1:0","bearer_token_env":"TOKEN","wallet":"pool","sources":["api"]}}
    })).unwrap();
    let resolution = assignment::resolve(&config).unwrap();
    let mut snapshot = json!({"version":1,"deployment":config,
        "sources":{"api":{"settings":{"spec":"local.json"},"base_url":"https://api.example.com"}},
        "inventory":[{"server":"main","wallet_bindings":resolution.bindings["main"],"tools":[{"source":"api","name":"api_read","path":"/read","input_schema":{"type":"object"}}]}]});
    // Derive current identity permissions before adding any obsolete fields.
    let expected =
        identities::for_interval(&manifest, &snapshot, &state, |_| None, &[], false).unwrap();
    let directory = manifest
        .evidence_dir
        .join(format!("{}-tor-run", manifest.run_id));
    files::create_dir(&manifest.evidence_dir).unwrap();
    files::create_dir(&directory).unwrap();
    let publish = |name: &str, value: &Value| {
        files::create_file(&directory.join(name))
            .unwrap()
            .write_all(&serde_json::to_vec(value).unwrap())
            .unwrap();
    };
    publish(
        "tor.process.json",
        &json!({"success":true,"valid_output":true,"forced_kill":false}),
    );
    publish("confinement.json", &json!({"status":"qualified"}));
    publish("identities.json", &serde_json::to_value(&expected).unwrap());
    let mut events = Vec::new();
    for (index, identity) in expected
        .values()
        .filter(|identity| identity.required)
        .enumerate()
    {
        let target = identity.targets.iter().next().unwrap();
        events.push(format!(
            "650 STREAM {} NEW 0 {} SOCKS_USERNAME=\"{}\" SOCKS_PASSWORD=\"{}\"",
            index + 1,
            target,
            identity.user,
            identity.password
        ));
        events.push(format!(
            "650 STREAM {} SUCCEEDED {} 192.0.2.1:443",
            index + 1,
            index + 1
        ));
    }
    let event_bytes = format!("{}\n", events.join("\n")).into_bytes();
    files::create_file(&directory.join("tor-events.log"))
        .unwrap()
        .write_all(&event_bytes)
        .unwrap();
    publish(
        "control-completion.json",
        &json!({"version":1,"complete":true,"event_count":events.len(),"events_sha256":files::hash(&event_bytes)}),
    );
    let artifact_directory = manifest.evidence_dir.join(&manifest.run_id);
    files::create_dir(&artifact_directory).unwrap();
    let mut artifact_hashes = BTreeMap::new();
    for (name, bytes) in [
        ("snapshot.json", serde_json::to_vec(&snapshot).unwrap()),
        (
            "frozen-snapshot.json",
            serde_json::to_vec(&snapshot).unwrap(),
        ),
        ("deployment.toml", b"retained offline fixture".to_vec()),
        ("build.json", b"{}".to_vec()),
    ] {
        files::create_file(&artifact_directory.join(name))
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        artifact_hashes.insert(name.to_owned(), files::hash(&bytes));
    }
    let mut resolved = json!({"treasury":{"id":treasury_id},"funding":{"confidentiality":"public"},"network":{"mode":"tor"},
        "resolved_wallets":{"pool":{"mode":"zcash_rotation","max_api_payment_usdc":"0.02"}},"wallet_bindings":{"main":{"api":{"wallet":"pool"}}}});
    resolved["sources"] = json!({"api":{}});
    let mut pins = Pins {
        binary_sha256: files::hash(b"fixture binary"),
        cargo_lock_sha256: files::hash(b"fixture lock"),
        source_revision: "archive fixture".into(),
        source_dirty: false,
        resolved_config: resolved,
        qualification: "configuration_only".into(),
        catalogs: None,
    };
    let authorization = Authorization {
        version: 1,
        id: manifest.registry_authorization.clone(),
        treasury_id: treasury_id.clone(),
        cumulative_api_usdc: "0.04".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    let mut registry = Registry::authorize(&state, &authorization, 2).unwrap();
    registry
        .prepare(
            &planner::build(&manifest, &pins.resolved_config).unwrap(),
            &pins,
            2,
        )
        .unwrap();
    drop(registry);
    // Install synthetic retained artifacts after config-only preparation. No
    // executable, Tor daemon or funded runtime is started by this archive test.
    pins.qualification = "catalogs_prepared".into();
    pins.catalogs = Some(CatalogPins {
        build: x402_treazury::build_identity::current(),
        directory: artifact_directory.clone(),
        files: artifact_hashes,
    });
    let raw_pins = serde_json::to_string(&pins).unwrap();
    let database =
        rusqlite::Connection::open(state.join("live-integration/registry.sqlite")).unwrap();
    database
        .execute(
            "UPDATE pins SET payload=?1,digest=?2 WHERE run=?3",
            rusqlite::params![raw_pins, files::hash(raw_pins.as_bytes()), manifest.run_id],
        )
        .unwrap();
    drop(database);
    let baseline_output = manifest.evidence_dir.join("current-audit.json");
    assert_eq!(
        audit_run(&state, &manifest.run_id, &baseline_output).unwrap(),
        0
    );
    let baseline: Value = serde_json::from_slice(&std::fs::read(baseline_output).unwrap()).unwrap();

    let mut retired = vec![
        json!({"provider":"missing-legacy-provider.toml"}),
        json!({"fallbacks":{"GET":["missing-provider.toml"],"POST":[]}}),
        json!({"fallbacks":{},"source_discovery":false,"api_requests":true,"provider_metadata":false}),
    ];
    for old_key in ["serve", "warm", "on_demand", "search"] {
        let mut relay = json!({"fallbacks":{}});
        relay[old_key] = json!(true);
        retired.push(relay);
    }
    for (ordinal, relay) in retired.into_iter().enumerate() {
        snapshot["deployment"]["discovery_relay"] = relay;
        assert!(
            serde_json::from_value::<MetaConfig>(snapshot["deployment"].clone()).is_err(),
            "retired relay configuration {ordinal}"
        );
        assert!(
            identities::for_interval(&manifest, &snapshot, &state, |_| None, &[], false).is_err(),
            "live identity path must stay strict for retired relay configuration {ordinal}"
        );
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(artifact_directory.join("snapshot.json"), &bytes).unwrap();
        pins.catalogs
            .as_mut()
            .unwrap()
            .files
            .insert("snapshot.json".into(), files::hash(&bytes));
        let raw_pins = serde_json::to_string(&pins).unwrap();
        let database =
            rusqlite::Connection::open(state.join("live-integration/registry.sqlite")).unwrap();
        database
            .execute(
                "UPDATE pins SET payload=?1,digest=?2 WHERE run=?3",
                rusqlite::params![raw_pins, files::hash(raw_pins.as_bytes()), manifest.run_id],
            )
            .unwrap();
        let raw_manifest: String = database
            .query_row(
                "SELECT manifest FROM runs WHERE id=?1",
                [&manifest.run_id],
                |row| row.get(0),
            )
            .unwrap();
        let event_count: i64 = database
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let output = manifest
            .evidence_dir
            .join(format!("historical-audit-{ordinal}.json"));
        assert_eq!(audit_run(&state, &manifest.run_id, &output).unwrap(), 0);
        let observed: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(observed["stream_checks"], baseline["stream_checks"]);
        assert_eq!(
            observed["initial_identities_sha256"],
            baseline["initial_identities_sha256"]
        );
        assert_eq!(observed["identity_export_corrections"], json!([]));
        assert_eq!(
            std::fs::read(artifact_directory.join("snapshot.json")).unwrap(),
            bytes
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT manifest FROM runs WHERE id=?1",
                    [&manifest.run_id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            raw_manifest
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT payload FROM pins WHERE run=?1",
                    [&manifest.run_id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            raw_pins
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            event_count
        );
        assert_eq!(database.query_row("SELECT COUNT(*) FROM events WHERE kind IN ('execution_started','application_claim','application_payment','funding_execution')", [], |row| row.get::<_,i64>(0)).unwrap(), 0);
    }
}
