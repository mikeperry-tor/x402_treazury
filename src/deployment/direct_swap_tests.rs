use super::*;

fn config() -> MetaConfig {
    toml::from_str(
        r#"
version=1
[treasury]
state_dir='state'
[funding]
base_rpc_url_env='not a Base environment name'
[servers.control]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
[servers.control.swap_control]
receive=true
"#,
    )
    .unwrap()
}

#[test]
fn direct_only_listener_requires_treasury_and_auth_but_no_base_wallet() {
    let mut config = config();
    assert!(config.validate().is_ok());
    config
        .servers
        .get_mut("control")
        .unwrap()
        .swap_control
        .as_mut()
        .unwrap()
        .receive = false;
    assert!(
        config.validate().is_ok(),
        "disabled admissions retain owned status"
    );
    let definitions = listener_management_tools(&config.servers["control"]);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name, "x402_treazury_swap_status");
    for serving in [false, true] {
        let inventory = listener_tool_inventory(
            "control",
            &config.servers["control"],
            &BTreeMap::new(),
            &BTreeMap::new(),
            serving,
        )
        .unwrap();
        assert!(
            inventory.tools.is_empty(),
            "owned status alone is a usable endpoint"
        );
    }
    config.servers.get_mut("control").unwrap().auth = false;
    config
        .servers
        .get_mut("control")
        .unwrap()
        .bearer_token_env
        .clear();
    assert!(format!("{:#}", config.validate().err().unwrap()).contains("requires authentication"));
    config.servers.get_mut("control").unwrap().auth = true;
    config.servers.get_mut("control").unwrap().bearer_token_env = "TOKEN".into();
    config.funding = None;
    assert!(
        config.validate().is_ok(),
        "disabled status needs no funding config"
    );
    config.treasury = None;
    assert!(format!("{:#}", config.validate().err().unwrap()).contains("requires [treasury]"));
}

#[test]
fn direct_only_inventory_and_validation_reject_unknown_authority() {
    let mut config = config();
    config.funding = None;
    assert!(
        config.validate().is_ok(),
        "native receiving needs no funding settings"
    );
    let listener = config.servers.get_mut("control").unwrap();
    listener.swap_control.as_mut().unwrap().send = true;
    assert!(
        config.validate().is_ok(),
        "sending is a separate opt-in and native sending requires no NEAR configuration"
    );
    config
        .servers
        .get_mut("control")
        .unwrap()
        .swap_control
        .as_mut()
        .unwrap()
        .receive = false;
    assert!(config.validate().is_ok(), "send-only listeners are valid");
    for cap in [None, Some("none"), Some("0"), Some("0.05")] {
        config
            .servers
            .get_mut("control")
            .unwrap()
            .swap_control
            .as_mut()
            .unwrap()
            .max_send_zec = cap.map(str::to_owned);
        assert!(
            config.validate().is_ok(),
            "valid optional total-debit preference: {cap:?}"
        );
    }
    for cap in ["", "-1", "NaN", "0.000000001"] {
        config
            .servers
            .get_mut("control")
            .unwrap()
            .swap_control
            .as_mut()
            .unwrap()
            .max_send_zec = Some(cap.into());
        assert!(
            config.validate().is_err(),
            "invalid total-debit preference: {cap}"
        );
    }
    assert!(
        toml::from_str::<ListenerConfig>(
            "listen='127.0.0.1:0'\n[swap_control]\nreceive=true\nowner='foreign'"
        )
        .is_err()
    );
}

#[tokio::test]
async fn disabled_swap_control_keeps_saved_status_without_sdk_base_or_near_credentials() {
    saved_status_with_unavailable_sdk(false).await;
    saved_status_with_unavailable_sdk(true).await;
}

async fn saved_status_with_unavailable_sdk(receive: bool) {
    use crate::rotation::store::Store;
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"invalid SDK snapshot fixture",
    )
    .unwrap();
    let operation = store
        .reserve_direct(
            "one",
            "receive",
            "saved",
            serde_json::json!({"original":true}),
        )
        .unwrap()
        .0;
    drop(store);
    super::initialization_tests::write_spec(dir.path());
    let mut text = super::initialization_tests::managed_config();
    if !receive {
        text = text.replace("[funding]", "[funding]\nnear_api_key_env='MISSING_NEAR'");
    }
    text.push_str(&format!(
        "\n[servers.one.swap_control]\nreceive={receive}\n"
    ));
    text.push_str(STATIC_LISTENER);
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, text).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let mut env = super::initialization_tests::env();
    if !receive {
        env.remove("BASE");
    }
    env.insert("STATIC".into(), format!("{:064x}", 1));
    let initialized = deployment
        .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
        .await
        .unwrap();
    assert!(matches!(initialized.runtime, ManagedRuntime::StoreOnly(_)));
    assert!(initialized.wallets.contains_key("static"));
    let service = initialized
        .runtime
        .direct_service()
        .expect("saved status remains available");
    let config = crate::direct_swap::Config::default();
    let status = service
        .invoke(
            "one",
            &config,
            "x402_treazury_swap_status",
            serde_json::json!({"operation_id":operation.id}),
        )
        .await
        .unwrap();
    assert_eq!(status["operation"]["terms"]["original"], true);
    let error = service.invoke("one", &crate::direct_swap::Config {receive:true,send:false,..Default::default()}, "x402_treazury_payment_request_create", serde_json::json!({"asset_id":"source","chain":"source_chain","amount":"1","refund_address":"refund","request_id":"denied"})).await.unwrap_err();
    assert!(error.to_string().contains("unavailable"));
    let store = initialized.runtime.store().unwrap();
    let created = store
        .call(|s| s.reserve_direct("one", "receive", "denied", serde_json::json!({})))
        .await
        .unwrap()
        .1;
    assert!(
        created,
        "unavailable admission must not reserve an operation"
    );
    drop(store);
    drop(service);
    initialized.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_connection_warnings_report_configuration_causes_without_values() {
    if crate::test_log_capture::run_in_child(
        "deployment::direct_swap_tests::local_connection_warnings_report_configuration_causes_without_values",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let store = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"unavailable SDK fixture",
    )
    .unwrap();
    drop(store);
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        super::initialization_tests::managed_config().replace(
            "auto_fund=false",
            "auto_fund=false\nnear_api_key_env='NEAR'",
        ),
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    for (key, value, category, reason) in [
        (
            "BASE",
            None,
            "base_configuration_unavailable",
            "configured Base RPC environment variable is missing or empty",
        ),
        (
            "BASE",
            Some("http://user:configuration-secret@provider.invalid/path?key=configuration-secret"),
            "base_configuration_unavailable",
            "endpoint requires TLS",
        ),
        (
            "NEAR",
            None,
            "funding_configuration_unavailable",
            "required environment variable NEAR is missing or empty",
        ),
        (
            "NEAR",
            Some("configuration-secret\ninvalid"),
            "funding_configuration_unavailable",
            "invalid NEAR API key",
        ),
        (
            "INDEXER",
            None,
            "funding_configuration_unavailable",
            "required endpoint environment variable INDEXER is missing or empty",
        ),
        (
            "INDEXER",
            Some("http://user:configuration-secret@provider.invalid/path?key=configuration-secret"),
            "funding_configuration_unavailable",
            "endpoint requires TLS",
        ),
    ] {
        let before = std::fs::metadata(log.path()).unwrap().len() as usize;
        let mut env = super::initialization_tests::env();
        env.insert("NEAR".into(), "fixture".into());
        if let Some(value) = value {
            env.insert(key.into(), value.into());
        } else {
            env.remove(key);
        }
        let setup = deployment.wallet_setup(&env, true, true).await.unwrap();
        assert!(matches!(setup, ManagedSetup::StoreOnly { .. }));
        assert!(!setup.funding_available());
        assert!(
            setup
                .store()
                .call(|s| s.funding_jobs())
                .await
                .unwrap()
                .is_empty()
        );
        setup.close().await.unwrap();
        let output = std::fs::read_to_string(log.path()).unwrap();
        assert!(
            output[before..].contains(category),
            "{key}: expected category"
        );
        assert!(output[before..].contains(reason), "{key}: expected cause");
    }
    let output = std::fs::read_to_string(log.path()).unwrap();
    assert!(output.contains("WARN"));
    for secret in ["configuration-secret", "provider.invalid", "user:"] {
        assert!(!output.contains(secret), "preflight logs must omit values");
    }
}

#[tokio::test]
async fn direct_receive_retains_sdk_while_missing_base_only_disables_managed_clients() {
    let dir = tempfile::tempdir().unwrap();
    let owner = crate::treasury::Treasury::create(dir.path().join("state"), dir.path().join("key"), 2_000_000, Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    owner.close().await.unwrap();
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        super::initialization_tests::managed_config()
            + "\n[servers.one.swap_control]\nreceive=true\n"
            + STATIC_LISTENER,
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let mut env = super::initialization_tests::env();
    env.remove("BASE");
    env.insert("STATIC".into(), format!("{:064x}", 1));
    let initialized = deployment
        .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
        .await
        .unwrap();
    assert!(matches!(initialized.runtime, ManagedRuntime::Direct(_)));
    assert!(!initialized.runtime.funding_available());
    assert!(initialized.managed_pools.is_empty());
    assert_eq!(initialized.wallets.len(), 3);
    assert!(initialized.wallets.contains_key("static"));
    let store = initialized.runtime.store().unwrap();
    let jobs = store.call(|s| s.funding_jobs()).await.unwrap();
    assert!(
        jobs.is_empty(),
        "startup must not allocate or fund Base wallets"
    );
    drop(store);
    initialized.shutdown().await.unwrap();
}

const STATIC_LISTENER: &str = "\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[servers.static]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n";

#[tokio::test]
async fn removed_table_and_funding_retain_only_authenticated_owned_status() {
    for (listener_name, auth, inherits) in [
        ("owner", true, true),
        ("foreign", true, false),
        ("owner", false, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::rotation::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"unavailable SDK fixture",
        )
        .unwrap();
        let operation = store
            .reserve_direct(
                "owner",
                "receive",
                "saved",
                serde_json::json!({"original":true}),
            )
            .unwrap()
            .0;
        drop(store);
        super::initialization_tests::write_spec(dir.path());
        let text = recovery_config(listener_name, auth, false);
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, &text).unwrap();
        let shown = Deployment::show_config(&path).await.unwrap();
        assert!(
            shown["servers"][listener_name]["swap_control"].is_null(),
            "config show must not infer wallet state"
        );
        let env = BTreeMap::from([
            ("TOKEN".into(), "fixture".into()),
            ("STATIC".into(), format!("{:064x}", 1)),
        ]);
        let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .unwrap();
        assert_eq!(deployment.direct_history, Some(true));
        assert_eq!(
            deployment.listeners[listener_name].swap_control.is_some(),
            inherits
        );
        assert_eq!(
            deployment.config.servers[listener_name]
                .swap_control
                .is_some(),
            inherits
        );
        let effective = deployment
            .wallet_resolution
            .source_bindings
            .listeners(&deployment.config);
        assert_eq!(effective[listener_name].swap_control.is_some(), inherits);
        let inventory = deployment.inventory();
        assert_eq!(
            inventory[0]
                .management_tools
                .iter()
                .any(|t| t.name == "x402_treazury_swap_status"),
            inherits
        );
        assert!(
            inventory[0]
                .management_tools
                .iter()
                .all(|t| t.name != "x402_treazury_payment_request_create")
        );
        let initialized = deployment
            .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
            .await
            .unwrap();
        assert!(matches!(initialized.runtime, ManagedRuntime::StoreOnly(_)));
        let service = initialized.runtime.direct_service().unwrap();
        let status = service
            .invoke(
                "owner",
                &crate::direct_swap::Config::default(),
                "x402_treazury_swap_status",
                serde_json::json!({"operation_id":operation.id}),
            )
            .await
            .unwrap();
        assert_eq!(status["operation"]["terms"]["original"], true);
        assert!(
            service
                .invoke(
                    "foreign",
                    &crate::direct_swap::Config::default(),
                    "x402_treazury_swap_status",
                    serde_json::json!({"operation_id":operation.id})
                )
                .await
                .is_err()
        );
        drop(service);
        initialized.shutdown().await.unwrap();
        if inherits {
            std::fs::write(&path,"version=1\n[treasury]\nstate_dir='state'\nkey_file='key'\n[servers.owner]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\n").unwrap();
            let direct_only =
                Deployment::load_for_serving_with_relay(&path, &env, Default::default())
                    .await
                    .unwrap();
            assert!(
                direct_only.listeners["owner"].swap_control.is_some(),
                "recover before empty-source validation"
            );
            assert!(direct_only.wallet_resolution.wallets.is_empty());
            let initialized = direct_only
                .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
                .await
                .unwrap();
            assert!(matches!(initialized.runtime, ManagedRuntime::StoreOnly(_)));
            initialized.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn removed_listener_retains_background_sdk_owner_without_restoring_tools() {
    let dir = tempfile::tempdir().unwrap();
    let owner = crate::treasury::Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    let store = owner.store_handle();
    store
        .call(|s| {
            s.reserve_direct(
                "removed",
                "receive",
                "saved",
                serde_json::json!({"original":true}),
            )
        })
        .await
        .unwrap();
    drop(store);
    owner.close().await.unwrap();
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, recovery_config("remaining", true, true)).unwrap();
    let env = BTreeMap::from([
        ("TOKEN".into(), "fixture".into()),
        ("STATIC".into(), format!("{:064x}", 1)),
        ("INDEXER".into(), "http://127.0.0.1:1".into()),
        ("SUBMIT".into(), "http://127.0.0.1:1".into()),
    ]);
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    assert_eq!(deployment.direct_history, Some(true));
    assert!(
        deployment
            .listeners
            .values()
            .all(|s| s.swap_control.is_none())
    );
    let initialized = deployment
        .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
        .await
        .unwrap();
    assert!(matches!(initialized.runtime, ManagedRuntime::Direct(_)));
    assert!(initialized.runtime.direct_service().is_some());
    assert!(!initialized.runtime.funding_available());
    initialized.shutdown().await.unwrap();
}

fn recovery_config(listener: &str, auth: bool, funding: bool) -> String {
    format!(
        "version=1\n[treasury]\nstate_dir='state'\nkey_file='key'\nindexer_url_env='INDEXER'\nsubmission_url_env='SUBMIT'\n{}\n[wallets.static]\nmode='static'\nprivate_key_env='STATIC'\n[sources.api]\nspec='spec.json'\nbase_url='https://127.0.0.1:1'\n[servers.{listener}]\nlisten='127.0.0.1:0'\nauth={auth}\n{}\nwallet='static'\nsources=['api']\n",
        if funding {
            "[funding]\nauto_fund=false"
        } else {
            ""
        },
        if auth { "bearer_token_env='TOKEN'" } else { "" }
    )
}

#[tokio::test]
async fn native_sdk_remains_available_without_usable_near_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let owner = crate::treasury::Treasury::create(dir.path().join("state"),dir.path().join("key"),2_000_000,Some(zeroize::Zeroizing::new("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".into()))).await.unwrap();
    owner.close().await.unwrap();
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    let mut env = BTreeMap::from([
        ("TOKEN".into(), "fixture".into()),
        ("STATIC".into(), format!("{:064x}", 1)),
        ("INDEXER".into(), "http://127.0.0.1:1".into()),
        ("SUBMIT".into(), "http://127.0.0.1:1".into()),
    ]);
    for near in [
        None,
        Some("near_api_key_env='MISSING'"),
        Some("near_api_key_env='BAD'"),
        Some("slippage_bps=10001"),
        Some("poll_seconds=99"),
        Some("poll_seconds=0"),
    ] {
        let mut text = recovery_config("native", true, near.is_some());
        if let Some(settings) = near {
            text = text.replace("auto_fund=false", &format!("auto_fund=false\n{settings}"));
        }
        text.push_str("\n[servers.native.swap_control]\nreceive=true\nsend=true\n");
        env.insert("BAD".into(), "malformed\nheader".into());
        std::fs::write(&path, text).unwrap();
        let deployment = Deployment::load(&path).await.unwrap();
        let initialized = deployment
            .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
            .await
            .unwrap();
        assert!(matches!(initialized.runtime, ManagedRuntime::Direct(_)));
        assert!(!initialized.runtime.funding_available());
        let service = initialized.runtime.direct_service().unwrap();
        let assets = service
            .invoke(
                "native",
                &crate::direct_swap::Config {
                    receive: true,
                    ..Default::default()
                },
                "x402_treazury_swap_assets",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        assert_eq!(assets["complete"], false);
        assert_eq!(assets["foreign_assets_available"], false);
        assert!(
            assets["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|asset| asset["assetId"] == crate::direct_swap::route::ZEC)
        );
        let store = initialized.runtime.store().unwrap();
        assert!(store.call(|s| s.funding_jobs()).await.unwrap().is_empty());
        drop(store);
        drop(service);
        initialized.shutdown().await.unwrap();
    }
    // Ordinary serving may retain mixed managed declarations without their Base
    // funding configuration; native capability does not grant funding authority.
    let mut text = super::initialization_tests::managed_config();
    text = text.replace(
        "[funding]\nauto_fund=false\nbase_rpc_url_env='BASE'\nbase_rpc_fallback_url_envs=[]\n",
        "",
    );
    text.push_str(STATIC_LISTENER);
    text.push_str("\n[servers.one.swap_control]\nreceive=false\nsend=true\n");
    std::fs::write(&path, text).unwrap();
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let initialized = deployment
        .initialize_wallets_mode(&env, Default::default(), None, &BTreeSet::new(), true)
        .await
        .unwrap();
    assert!(matches!(initialized.runtime, ManagedRuntime::Direct(_)));
    assert!(initialized.wallets.contains_key("static"));
    assert!(initialized.managed_pools.is_empty());
    assert!(!initialized.runtime.funding_available());
    let store = initialized.runtime.store().unwrap();
    assert!(store.call(|s| s.funding_jobs()).await.unwrap().is_empty());
    drop(store);
    initialized.shutdown().await.unwrap();
}

#[tokio::test]
async fn catalog_loaders_do_not_inspect_configured_treasury_state() {
    for fault in ["schema", "identity", "corrupt"] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let store =
            crate::rotation::store::Store::create(&state, &dir.path().join("key"), 1, b"fixture")
                .unwrap();
        drop(store);
        let mut text = recovery_config("owner", true, false);
        match fault {
            "schema" => rusqlite::Connection::open(state.join("state.sqlite"))
                .unwrap()
                .execute_batch("PRAGMA user_version=22")
                .unwrap(),
            "identity" => {
                text = text.replace(
                    "[treasury]",
                    "[treasury]\nid='11111111-1111-4111-8111-111111111111'",
                )
            }
            _ => std::fs::write(state.join("state.sqlite"), b"invalid SQLite fixture").unwrap(),
        }
        super::initialization_tests::write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, text).unwrap();
        let before = std::fs::read(state.join("state.sqlite")).unwrap();
        for deployment in [
            Deployment::load(&path).await.unwrap(),
            Deployment::load_for_inspection(&path, false).await.unwrap(),
            Deployment::load_for_unsigned_serving(&path).await.unwrap(),
        ] {
            assert_eq!(deployment.direct_history, None);
            assert!(deployment.listeners["owner"].swap_control.is_none());
            assert!(!deployment.inventory()[0].tools.is_empty());
        }
        assert_eq!(
            std::fs::read(state.join("state.sqlite")).unwrap(),
            before,
            "inspection cannot repair or migrate wallet state"
        );
        let env = BTreeMap::from([
            ("TOKEN".into(), "fixture".into()),
            ("STATIC".into(), format!("{:064x}", 1)),
        ]);
        let unsigned = Deployment::load_for_unsigned_serving(&path)
            .await
            .unwrap()
            .bind_unsigned(&env)
            .await
            .unwrap();
        assert!(matches!(unsigned.runtime, ManagedRuntime::None));
        drop(unsigned);
        assert!(
            Deployment::load_for_serving_with_relay(&path, &env, Default::default())
                .await
                .is_err(),
            "signed serving preserves state identity/schema checks: {fault}"
        );
        assert!(
            Deployment::load(&path)
                .await
                .unwrap()
                .bind(&env)
                .await
                .is_err(),
            "late binding checks actual state before financial runtime: {fault}"
        );
    }
}

#[tokio::test]
async fn inspection_then_bind_restores_only_authenticated_owned_status_over_mcp() {
    use crate::mcp_wire::McpResponse;
    use serde_json::{Value, json};
    let dir = tempfile::tempdir().unwrap();
    let mut store = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"unavailable SDK fixture",
    )
    .unwrap();
    for owner in ["owner", "public"] {
        store
            .reserve_direct(owner, "receive", "saved", json!({"original":owner}))
            .unwrap();
    }
    drop(store);
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    let text = recovery_config("owner", true, false)
        + "\n[servers.foreign]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='static'\nsources=['api']\n[servers.public]\nlisten='127.0.0.1:0'\nauth=false\nwallet='static'\nsources=['api']\n";
    std::fs::write(&path, text).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    assert_eq!(deployment.direct_history, None);
    assert!(
        deployment
            .listeners
            .values()
            .all(|s| s.swap_control.is_none())
    );
    let env = BTreeMap::from([
        ("TOKEN".into(), "fixture".into()),
        ("STATIC".into(), format!("{:064x}", 1)),
    ]);
    let running = deployment.bind(&env).await.unwrap();
    assert!(matches!(running.runtime, ManagedRuntime::StoreOnly(_)));
    let addresses = running.addresses();
    let stop = CancellationToken::new();
    let task = tokio::spawn(running.serve(stop.clone()));
    let client =
        crate::network::discovery("https://recovery-fixture.invalid", Duration::from_secs(5))
            .unwrap();
    for (name, address) in addresses {
        let listed: Value = client
            .post(format!("http://{address}/mcp"))
            .bearer_auth("fixture")
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap()
            .mcp_json()
            .await
            .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools
                .iter()
                .any(|t| t["name"] == "x402_treazury_swap_status"),
            name == "owner"
        );
        assert!(tools.iter().all(|t| t["name"] != "x402_treazury_swap_send"
            && t["name"] != "x402_treazury_payment_request_create"));
        if name == "owner" {
            let status:Value=client.post(format!("http://{address}/mcp")).bearer_auth("fixture").header("accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"x402_treazury_swap_status","arguments":{"kind":"receive","request_id":"saved"}}})).send().await.unwrap().mcp_json().await.unwrap();
            assert_eq!(
                status["result"]["structuredContent"]["operation"]["terms"]["original"],
                "owner"
            );
        }
    }
    stop.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn late_history_recovery_rechecks_static_namespace_before_wallet_initialization() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"unavailable SDK fixture",
    )
    .unwrap();
    store
        .reserve_direct(
            "owner",
            "receive",
            "saved",
            serde_json::json!({"original":true}),
        )
        .unwrap();
    drop(store);
    super::initialization_tests::write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    let text = recovery_config("owner", true, false)
        .replace("[sources.api]", "[sources.api]\nprefix='x402_treazury'");
    std::fs::write(&path, text).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    assert_eq!(deployment.direct_history, None);
    let env = BTreeMap::from([("TOKEN".into(), "fixture".into())]);
    // STATIC is deliberately absent: the namespace conflict must win before
    // signer loading or any financial owner is constructed.
    let error = deployment.bind(&env).await.err().unwrap();
    assert!(
        format!("{error:#}").contains("reserved namespace"),
        "{error:#}"
    );
}
