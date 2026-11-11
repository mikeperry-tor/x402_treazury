use std::collections::BTreeMap;
use x402_treazury::{
    deployment::Deployment,
    rotation::config::{WalletConfig, zatoshis},
};
fn config() -> String {
    r#"version = 1
[treasury]
id = "11111111-1111-4111-8111-111111111111"
state_dir = "state/treasury"
key_file = "secrets/treasury.key"
indexer_url_env = "INDEXER"
submission_url_env = "SUBMISSION"
[funding]
base_rpc_url_env = "BASE_RPC"
[wallets.research]
mode = "zcash_rotation"
max_api_payment_usdc = "1.00"
[wallets.research.reserves.small]
funding_amount_usdc = "2.00"
[sources.api]
spec = "spec.json"
[servers.main]
listen = "127.0.0.1:0"
bearer_token_env = "TOKEN"
wallet = "research"
sources = ["api"]
"#
    .into()
}
#[tokio::test]
async fn inspection_resolves_paths_and_defaults_without_state_or_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.toml");
    std::fs::write(&path, config()).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(
        shown["wallets"]["research"]["reserves"]["small"]["funding_amount_usdc"],
        "2.00"
    );
    assert_eq!(
        shown["treasury"]["state_dir"],
        dir.path().join("state/treasury").to_str().unwrap()
    );
    assert_eq!(shown["funding"]["base_rpc_url_env"], "BASE_RPC");
    assert_eq!(
        shown["base_rpc_policy"]["fallback_defaults"],
        serde_json::json!(["https://base.drpc.org", "https://mainnet.base.org"])
    );
    assert!(!dir.path().join("state").exists());
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"servers":[{"url":"https://example.invalid"}],"paths":{"/pay":{"get":{}}}}"#,
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    assert_eq!(deployment.inventory().len(), 1);
    assert!(!dir.path().join("state").exists());
    assert!(deployment.bind(&BTreeMap::new()).await.is_err());
    assert!(!dir.path().join("state").exists());
    for (from, to) in [
        (
            "mode = \"zcash_rotation\"",
            "mode = \"zcash_rotation\"\nmax_funding_spend_zec = \"0.02\"",
        ),
        (
            "mode = \"zcash_rotation\"",
            "mode = \"zcash_rotation\"\nprivate_key_env = \"KEY\"",
        ),
        ("[funding]", "[unknown]"),
        (
            "base_rpc_url_env = \"BASE_RPC\"",
            "base_rpc_url_env = \"BASE_RPC\"\nconfidentiality = \"invalid\"",
        ),
    ] {
        std::fs::write(&path, config().replace(from, to)).unwrap();
        assert!(Deployment::show_config(&path).await.is_err(), "{to}");
    }
}
#[test]
fn money_is_exact_and_tagged_profiles_reject_cross_mode_fields() {
    assert_eq!(zatoshis("0.00000001").unwrap(), 1);
    for bad in [
        "NaN",
        "-1",
        "0",
        "1e3",
        "21000001",
        "0.000000001",
        "99999999999999999999999",
    ] {
        assert!(zatoshis(bad).is_err());
    }
    assert!(
        toml::from_str::<WalletConfig>(
            "mode='static'\nprivate_key_env='KEY'\nfunding_amount_usdc='5'"
        )
        .is_err()
    );
    toml::from_str::<WalletConfig>("mode='zcash_rotation'\nmax_api_payment_usdc='1'\n[reserves.small]\nfunding_amount_usdc='2'")
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
fn rpc_fallback_configuration_is_explicit_bounded_and_unique() {
    use x402_treazury::rotation::config::FundingConfig;
    for (fallbacks, valid) in [
        ("[]", true),
        ("['SECOND','THIRD']", true),
        ("['BASE']", false),
        ("['SECOND','SECOND']", false),
        ("['SECOND','THIRD','FOURTH']", false),
        ("['not an env name']", false),
    ] {
        let f: FundingConfig = toml::from_str(&format!(
            "base_rpc_url_env='BASE'\nbase_rpc_fallback_url_envs={fallbacks}"
        ))
        .unwrap();
        assert_eq!(f.validate().is_ok(), valid, "{fallbacks}");
    }
}

#[test]
fn slippage_configuration_accepts_the_full_percentage_domain() {
    use x402_treazury::rotation::config::FundingConfig;
    let default: FundingConfig = toml::from_str("").unwrap();
    assert_eq!(default.slippage_bps, 500);
    for (slippage, valid) in [
        (0, true),
        (1001, true),
        (2500, true),
        (10000, true),
        (10001, false),
    ] {
        let funding: FundingConfig = toml::from_str(&format!("slippage_bps={slippage}")).unwrap();
        assert_eq!(funding.validate().is_ok(), valid, "{slippage}");
    }
}

#[test]
fn default_rpc_resolution_and_explicit_overrides_are_unambiguous() {
    use x402_treazury::rotation::config::{DEFAULT_BASE_RPC_URLS, FundingConfig};
    let parse = |text: &str| toml::from_str::<FundingConfig>(text).unwrap();
    assert_eq!(
        parse("").base_rpc_urls(|_| None).unwrap(),
        DEFAULT_BASE_RPC_URLS
    );
    assert_eq!(
        parse("base_rpc_fallback_url_envs=[]")
            .base_rpc_urls(|_| None)
            .unwrap(),
        vec![DEFAULT_BASE_RPC_URLS[0]]
    );
    let custom = parse("base_rpc_url_env='PRIVATE'\nbase_rpc_fallback_url_envs=['BACKUP']");
    assert_eq!(
        custom
            .base_rpc_urls(|name| Some(format!("https://{}.invalid", name.to_lowercase())))
            .unwrap(),
        vec!["https://private.invalid", "https://backup.invalid"]
    );
    assert!(custom.base_rpc_urls(|_| None).is_err());
    assert!(
        custom
            .base_rpc_urls(|_| Some("https://same.invalid".into()))
            .is_err()
    );
    assert!(parse("").base_rpc_urls(|_| Some("".into())).is_err());
    assert_eq!(
        parse("")
            .base_rpc_urls(|_| Some("https://base.drpc.org/".into()))
            .unwrap(),
        vec!["https://base.drpc.org/", "https://mainnet.base.org"]
    );
    assert!(
        parse("base_rpc_url_env='MISSING'")
            .base_rpc_urls(|_| None)
            .is_err()
    );
    assert!(
        parse("base_rpc_fallback_url_envs=['MISSING']")
            .base_rpc_urls(|_| None)
            .is_err()
    );
}

#[test]
fn public_funding_is_explicit_and_needs_no_near_credentials() {
    use x402_treazury::rotation::config::FundingConfig;
    for mode in ["public", "basic", "advanced"] {
        let f: FundingConfig = toml::from_str(&format!(
            "base_rpc_url_env='BASE'\nconfidentiality='{mode}'"
        ))
        .unwrap();
        f.validate().unwrap();
        assert!(f.near_api_key_env.is_none());
        assert!(f.near_user_session_env.is_none());
    }
    let f: FundingConfig = toml::from_str("base_rpc_url_env='BASE'").unwrap();
    assert_eq!(f.confidentiality, "basic"); // Existing configs never silently lose confidentiality.
}

#[tokio::test]
async fn ergonomic_treasury_defaults_remain_offline_and_funding_is_explicitly_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let text = config()
        .replace("id = \"11111111-1111-4111-8111-111111111111\"\n", "")
        .replace("key_file = \"secrets/treasury.key\"\n", "")
        .replace("indexer_url_env = \"INDEXER\"\n", "")
        .replace("submission_url_env = \"SUBMISSION\"\n", "");
    std::fs::write(&path, &text).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(shown["treasury_identity"], "from_wallet_state_at_runtime");
    assert!(shown["treasury"]["id"].is_null());
    assert_eq!(
        shown["treasury"]["key_file"],
        dir.path()
            .join("state/treasury/wallet.key")
            .to_str()
            .unwrap()
    );
    assert_eq!(shown["funding"]["auto_fund"], true);
    assert!(!dir.path().join("state").exists());
    std::fs::write(
        &path,
        text.replace("[funding]", "[funding]\nauto_fund=false"),
    )
    .unwrap();
    assert_eq!(
        Deployment::show_config(&path).await.unwrap()["funding"]["auto_fund"],
        false
    );
}

#[test]
fn treasury_endpoint_defaults_overrides_and_missing_references() {
    use x402_treazury::{deployment::MetaConfig, rotation::config::TreasuryConfig};
    let mut t: TreasuryConfig = toml::from_str::<MetaConfig>(&config())
        .unwrap()
        .treasury
        .unwrap();
    assert!(t.indexer_endpoint(|_| None).is_err());
    assert!(t.submission_endpoint(|_| None).is_err());
    t.indexer_url_env.clear();
    t.submission_url_env.clear();
    assert_eq!(
        t.indexer_endpoint(|_| panic!("unexpected env lookup"))
            .unwrap(),
        "https://zec.rocks:443"
    );
    t.indexer_url = Some("https://indexer.example:443".into());
    assert_eq!(
        t.submission_endpoint(|_| None).unwrap(),
        "https://indexer.example:443"
    );
    t.submission_url = Some("https://submit.example:443".into());
    assert_eq!(
        t.submission_endpoint(|_| None).unwrap(),
        "https://submit.example:443"
    );
    t.indexer_url_env = "INDEXER".into();
    assert!(t.validate().is_err()); // Never silently pick between conflicting settings.
    t.indexer_url = None;
    assert!(t.indexer_endpoint(|_| Some("".into())).is_err());
    assert_eq!(
        t.indexer_endpoint(|_| Some("https://custom.example".into()))
            .unwrap(),
        "https://custom.example"
    );
}

#[test]
fn reserve_funding_limits_are_exact() {
    for (target, max, valid) in [
        ("2", "3", true),
        ("2", "2", true),
        ("2", "1.99", false),
        ("2", "NaN", false),
    ] {
        let config: WalletConfig = toml::from_str(&format!(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\n[reserves.small]\nfunding_amount_usdc='{target}'\nmax_funding_amount_usdc='{max}'"
        )).unwrap();
        assert_eq!(config.validate().is_ok(), valid);
    }
}

#[tokio::test]
async fn comprehensive_example_exposes_reserves_without_obsolete_fee_preferences_offline() {
    let shown = Deployment::show_config(std::path::Path::new(
        "examples/deployments/comprehensive.toml",
    ))
    .await
    .unwrap();
    assert_eq!(shown["wallets"].as_object().unwrap().len(), 2);
    assert_eq!(
        shown["wallet_bootstrap"],
        serde_json::json!({"bootstrap_group":"web","deferred_groups":["company"]})
    );
    assert_eq!(shown["wallet_summary"]["bootstrap_setup"]["group"], "web");
    assert_eq!(
        shown["wallet_summary"]["bootstrap_setup"]["initial_inventory_target_atomic"],
        "2000000"
    );
    assert_eq!(
        shown["wallet_summary"]["bootstrap_setup"]["initial_inventory_maximum_atomic"],
        "3000000"
    );
    assert_eq!(shown["wallet_bindings"]["web"]["exa"]["wallet"], "web");
    assert_eq!(
        shown["wallet_bindings"]["social"]["social"]["wallet"],
        "company"
    );
    assert_eq!(
        shown["wallet_summary"]["initial_inventory_target_atomic"],
        "4000000"
    );
    assert_eq!(
        shown["wallet_summary"]["initial_inventory_maximum_atomic"],
        "6000000"
    );
    assert_eq!(shown["wallets"]["web"]["max_api_payment_usdc"], "0.10");
    assert_eq!(shown["wallets"]["company"]["max_api_payment_usdc"], "0.50");
    for name in ["web", "company"] {
        assert_eq!(
            shown["wallets"][name]["reserves"]["small"]["min_funding_amount_usdc"],
            "2.00"
        );
        assert_eq!(
            shown["wallets"][name]["reserves"]["small"]["max_funding_amount_usdc"],
            "3.00"
        );
        assert_eq!(
            shown["wallets"][name]["max_retirement_remainder_usdc"],
            "0.001"
        );
    }
    assert!(
        shown["treasury"]
            .get("max_funding_transaction_fee_zec")
            .is_none()
    );
    assert!(
        shown["wallets"]["web"]
            .get("max_conversion_overhead_percent")
            .is_none()
    );
    assert!(
        shown["treasury"]
            .get("max_refund_shielding_fee_zec")
            .is_none()
    );
}

#[test]
fn example_bootstrap_policies_preserve_destination_groups_and_opt_out() {
    use alloy_primitives::U256;
    use x402_treazury::{deployment::MetaConfig, rotation::assignment};
    for (text, bootstrap, participants, group_count) in [
        (
            include_str!("../examples/deployments/comprehensive.toml"),
            "web",
            vec!["company"],
            2u64,
        ),
        (
            include_str!("../examples/deployments/research_desk.toml"),
            "general",
            vec!["personal", "health", "financial"],
            4u64,
        ),
    ] {
        let mut config: MetaConfig = toml::from_str(text).unwrap();
        let resolution = assignment::resolve(&config).unwrap();
        let policy = resolution.bootstrap.as_ref().unwrap();
        assert_eq!(policy.bootstrap_group, bootstrap);
        assert_eq!(policy.deferred_groups, participants);
        let setup = resolution.summary.bootstrap_setup.as_ref().unwrap();
        assert_eq!(setup.initial_inventory_target_atomic, "2000000");
        assert_eq!(setup.initial_inventory_maximum_atomic, "3000000");
        assert_eq!(
            resolution.summary.initial_inventory_target_atomic,
            (U256::from(2_000_000u64) * U256::from(group_count)).to_string()
        );
        assert_eq!(
            resolution.summary.initial_inventory_maximum_atomic,
            (U256::from(3_000_000u64) * U256::from(group_count)).to_string()
        );
        if bootstrap == "general" {
            assert_eq!(config.servers.len(), 12);
            assert_eq!(
                resolution.bindings["health"]
                    .values()
                    .next()
                    .unwrap()
                    .wallet,
                "health"
            );
            let cap = |name: &str| {
                let WalletConfig::ZcashRotation {
                    max_api_payment_usdc,
                    ..
                } = &resolution.wallets[name]
                else {
                    panic!("managed example")
                };
                max_api_payment_usdc.as_str()
            };
            assert_eq!(cap("general"), "1.00");
            assert_eq!(cap("health"), "0.50");
        }
        let bindings = serde_json::to_value(&resolution.bindings).unwrap();
        config.wallet_bootstrap = None;
        let separate = assignment::resolve(&config).unwrap();
        assert!(separate.bootstrap.is_none());
        assert!(separate.summary.bootstrap_setup.is_none());
        assert_eq!(
            separate.summary.initial_inventory_target_atomic,
            resolution.summary.initial_inventory_target_atomic
        );
        assert_eq!(
            separate.summary.initial_inventory_maximum_atomic,
            resolution.summary.initial_inventory_maximum_atomic
        );
        assert_eq!(
            serde_json::to_value(separate.bindings).unwrap(),
            bindings,
            "bootstrap policy does not rewrite destination bindings"
        );
    }
}

#[test]
fn reserve_shape_resolves_fixed_ranges_and_slot_defaults() {
    use x402_treazury::rotation::config::ReservesConfig;
    for (text, minimum, maximum) in [
        ("[small]\nfunding_amount_usdc='2'", "2000000", "2000000"),
        (
            "[small]\nfunding_amount_usdc='2'\n[large]\nfunding_amount_usdc='10'",
            "12000000",
            "12000000",
        ),
        (
            "[small]\nfunding_amount_usdc='2'\n[large]\nfunding_amount_usdc='2'",
            "4000000",
            "4000000",
        ),
        (
            "[small]\nfunding_amount_usdc='5'\n[large]\nfunding_amount_usdc='2'",
            "7000000",
            "7000000",
        ),
        (
            "[small]\nmin_funding_amount_usdc='2'\nmax_funding_amount_usdc='5'",
            "2000000",
            "5000000",
        ),
        (
            "[small]\nfunding_amount_usdc='2'\nmax_funding_amount_usdc='3'",
            "2000000",
            "3000000",
        ),
        (
            "[small]\nmin_funding_amount_usdc='2'\nmax_funding_amount_usdc='2'",
            "2000000",
            "2000000",
        ),
        ("[large]\nfunding_amount_usdc='10'", "12000000", "13000000"),
        ("[small]", "2000000", "3000000"),
        ("[large]", "12000000", "28000000"),
    ] {
        let reserves: ReservesConfig = toml::from_str(text).unwrap();
        reserves.validate().unwrap();
        assert_eq!(
            reserves
                .initial_inventory_minimum_atomic()
                .unwrap()
                .to_string(),
            minimum,
            "{text}"
        );
        assert_eq!(
            reserves
                .initial_inventory_maximum_atomic()
                .unwrap()
                .to_string(),
            maximum,
            "{text}"
        );
    }
    for text in [
        "[small]\nmin_funding_amount_usdc='2'",
        "[small]\nmax_funding_amount_usdc='2'",
        "[small]\nfunding_amount_usdc='2'\nmin_funding_amount_usdc='1'\nmax_funding_amount_usdc='3'",
        "[small]\nfunding_amount_usdc='2'\nmax_api_payment_usdc='1'",
        "[small]\nfunding_amount_usdc='2'\n[medium]\nfunding_amount_usdc='5'",
    ] {
        let parsed = toml::from_str::<ReservesConfig>(text);
        assert!(
            parsed.is_err() || parsed.unwrap().validate().is_err(),
            "{text}"
        );
    }
    for text in [
        "[small]\nfunding_amount_usdc='0'",
        "[small]\nfunding_amount_usdc='2'\nmax_funding_amount_usdc='1.99'",
        "[small]\nmin_funding_amount_usdc='3'\nmax_funding_amount_usdc='2'",
        "[small]\nmin_funding_amount_usdc='0'\nmax_funding_amount_usdc='3'",
        "[small]\nfunding_amount_usdc='0.0000001'",
        "[small]\nfunding_amount_usdc='9223372036854.775808'",
    ] {
        assert!(
            toml::from_str::<ReservesConfig>(text)
                .unwrap()
                .validate()
                .is_err(),
            "{text}"
        );
    }
}

#[test]
fn retirement_tolerance_is_exact_finite_nonnegative_and_bounded() {
    use x402_treazury::rotation::config::retirement_remainder;
    for (text, amount) in [
        ("0", 0),
        ("0.000000", 0),
        ("0.000001", 1),
        ("0.001", 1_000),
        ("1.23", 1_230_000),
        ("9223372036854.775807", i64::MAX as u64),
    ] {
        assert_eq!(retirement_remainder(text).unwrap(), amount);
    }
    for text in [
        "",
        "none",
        "off",
        "NaN",
        "-1",
        "1e3",
        "0.0000001",
        "9223372036854.775808",
    ] {
        assert!(retirement_remainder(text).is_err(), "{text}");
    }
}

#[tokio::test]
async fn retirement_defaults_and_overrides_resolve_independently_for_groups_and_templates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let managed = "mode='zcash_rotation'\nmax_api_payment_usdc='1'\n";
    let text = format!(
        "{}\n[wallets.exact]\n{managed}max_retirement_remainder_usdc='0'\n[wallets.exact.reserves.small]\nfunding_amount_usdc='2'\n[wallets.custom]\n{managed}max_retirement_remainder_usdc='0.002'\n[wallets.custom.reserves.small]\nfunding_amount_usdc='2'\n[wallet_templates.generated]\n{managed}[wallet_templates.generated.reserves.small]\nfunding_amount_usdc='2'\n[wallet_assignment]\nscope='server'\ntemplate='generated'\n[servers.generated]\nlisten='127.0.0.1:0'\nbearer_token_env='GENERATED_TOKEN'\nsources=['api']\n",
        config()
    );
    std::fs::write(&path, text).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    for (name, expected) in [
        ("research", "0.001"),
        ("exact", "0"),
        ("custom", "0.002"),
        ("auto_v1_server_generated", "0.001"),
    ] {
        assert_eq!(
            shown["resolved_wallets"][name]["max_retirement_remainder_usdc"],
            expected
        );
    }
    assert!(!dir.path().join("state").exists());
}

#[test]
fn retirement_configuration_reopen_preserves_values_until_selected_groups_are_applied() {
    use x402_treazury::rotation::{config::retirement_remainder, store::Store};
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let mut store = Store::create(&state, &key, 1, b"snapshot").unwrap();
    let id = store.id().to_owned();
    let profile = |setting: &str| -> WalletConfig {
        toml::from_str(&format!(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\n{setting}\n[reserves.small]\nfunding_amount_usdc='2'"
        )).unwrap()
    };
    let apply = |store: &mut Store, name: &str, wallet: WalletConfig| {
        let WalletConfig::ZcashRotation {
            reserves,
            max_retirement_remainder_usdc,
            ..
        } = wallet
        else {
            panic!("managed profile required")
        };
        store
            .ensure_capacity_group(
                name,
                &reserves,
                None,
                retirement_remainder(&max_retirement_remainder_usdc).unwrap(),
            )
            .unwrap()
    };
    let exact = profile("max_retirement_remainder_usdc='0'");
    let serialized = serde_json::to_value(&exact).unwrap();
    assert_eq!(serialized["max_retirement_remainder_usdc"], "0");
    let selected = apply(
        &mut store,
        "selected",
        serde_json::from_value(serialized).unwrap(),
    );
    let other = apply(
        &mut store,
        "other",
        profile("max_retirement_remainder_usdc='0.002'"),
    );
    drop(store);
    let mut store = Store::open(&state, &key, &id).unwrap();
    let before = store.status().unwrap();
    assert_eq!(
        before
            .pools
            .iter()
            .find(|p| p.id == selected)
            .unwrap()
            .retirement_tolerance_atomic
            .as_deref(),
        Some("0")
    );
    assert_eq!(apply(&mut store, "selected", profile("")), selected);
    let after = store.status().unwrap();
    assert_eq!(
        after
            .pools
            .iter()
            .find(|p| p.id == selected)
            .unwrap()
            .retirement_tolerance_atomic
            .as_deref(),
        Some("1000")
    );
    assert_eq!(
        after
            .pools
            .iter()
            .find(|p| p.id == other)
            .unwrap()
            .retirement_tolerance_atomic
            .as_deref(),
        Some("2000")
    );
    assert!(after.pools.iter().all(|p| p.addresses.is_empty()));
}

#[test]
fn managed_cap_is_explicit_and_independent_of_reserve_targets() {
    let base = "mode='zcash_rotation'\n[reserves.small]\nfunding_amount_usdc='2'";
    assert!(toml::from_str::<WalletConfig>(base).is_err());
    for cap in ["0", "0.01", "10", "none", "off"] {
        let config: WalletConfig =
            toml::from_str(&format!("max_api_payment_usdc='{cap}'\n{base}")).unwrap();
        config.validate().unwrap();
        let shown = serde_json::to_value(config).unwrap();
        assert_eq!(shown["max_retirement_remainder_usdc"], "0.001");
    }
    for cap in ["", " "] {
        for wallet in [
            format!("max_api_payment_usdc='{cap}'\n{base}"),
            format!("mode='static'\nprivate_key_env='KEY'\nmax_api_payment_usdc='{cap}'"),
        ] {
            assert!(
                toml::from_str::<WalletConfig>(&wallet)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
    let static_wallet: WalletConfig =
        toml::from_str("mode='static'\nprivate_key_env='KEY'").unwrap();
    assert_eq!(
        serde_json::to_value(static_wallet).unwrap()["max_api_payment_usdc"],
        "1.00"
    );
}

#[test]
fn config_serde_roundtrips_one_schema_and_rejects_unknown_fields() {
    use serde::{Serialize, de::DeserializeOwned};
    use x402_treazury::rotation::config::{FundingConfig, TreasuryConfig};
    fn roundtrip<T: Serialize + DeserializeOwned>(text: &str) -> serde_json::Value {
        let config: T = toml::from_str(text).unwrap();
        let json = serde_json::to_value(&config).unwrap();
        let decoded: T = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), json);
        let decoded: T = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), json);
        let mut unknown = json.clone();
        unknown["misspelled_setting"] = serde_json::json!(true);
        assert!(serde_json::from_value::<T>(unknown).is_err());
        let unknown = format!("misspelled_setting=true\n{text}");
        assert!(toml::from_str::<T>(&unknown).is_err());
        json
    }
    let wallet = roundtrip::<WalletConfig>("mode='static'\nprivate_key_env='KEY'");
    assert_eq!(wallet["max_api_payment_usdc"], "1.00");
    let wallet = roundtrip::<WalletConfig>(
        "mode='zcash_rotation'\nmax_api_payment_usdc='1'\n[reserves.small]\nfunding_amount_usdc='2'",
    );
    assert!(wallet.get("max_attempts").is_none());
    assert!(
        toml::from_str::<WalletConfig>(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_attempts=3"
        )
        .is_err()
    );
    assert_eq!(wallet["max_retirement_remainder_usdc"], "0.001");
    let treasury = roundtrip::<TreasuryConfig>("state_dir='state'");
    assert_eq!(
        treasury["confirmations"],
        u64::from(
            zcash_client_backend::data_api::wallet::ConfirmationsPolicy::default()
                .trusted()
                .get()
        )
    );
    let overridden = roundtrip::<TreasuryConfig>("state_dir='state'\nconfirmations=7");
    assert_eq!(overridden["confirmations"], 7);
    assert!(treasury.get("max_funding_transaction_fee_zec").is_none());
    let removed = "state_dir='state'\nmax_funding_transaction_fee_zec='0.0003'";
    assert!(toml::from_str::<TreasuryConfig>(removed).is_err());
    assert!(
        toml::from_str::<WalletConfig>(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5"
        )
        .is_err()
    );
    assert!(treasury.get("id").is_none());
    assert!(treasury.get("key_file").is_none());
    // Explicit optional values must not become empty identity/path references.
    for field in ["id", "key_file", "indexer_url_env", "submission_url_env"] {
        assert!(
            toml::from_str::<TreasuryConfig>(&format!("state_dir='state'\n{field}='  '")).is_err(),
            "{field}"
        );
    }
    let funding = roundtrip::<FundingConfig>("");
    assert_eq!(funding["auto_fund"], true);
    assert!(funding.get("base_confirmations").is_none());
    assert!(toml::from_str::<FundingConfig>("base_confirmations=12").is_err());
    assert_eq!(funding["quote_deadline_seconds"], 1800);
    let overridden = roundtrip::<FundingConfig>("quote_deadline_seconds=7200");
    assert_eq!(overridden["quote_deadline_seconds"], 7200);
}

#[tokio::test]
async fn managed_starter_exposes_exa_and_provider_capabilities_without_financial_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = tempfile::tempdir().unwrap();
    let mut config: toml::Value =
        toml::from_str(include_str!("../examples/deployments/managed.toml")).unwrap();
    config["treasury"]["state_dir"] =
        toml::Value::String(dir.path().join("state").to_str().unwrap().into());
    config["sources"]["exa"]["extends"] =
        toml::Value::String(root.join("providers/exa.toml").to_str().unwrap().into());
    let path = dir.path().join("managed.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(shown["wallets"].as_object().unwrap().len(), 1);
    assert_eq!(shown["servers"].as_object().unwrap().len(), 1);
    assert_eq!(shown["sources"].as_object().unwrap().len(), 1);
    assert_eq!(shown["wallets"]["research"]["mode"], "zcash_rotation");
    assert_eq!(shown["wallets"]["research"]["max_api_payment_usdc"], "0.50");
    assert_eq!(
        shown["wallet_bindings"]["research"]["exa"]["wallet"],
        "research"
    );
    assert_eq!(
        shown["sources"]["exa"]["settings"]["spec"],
        "https://api.exa.ai/openapi.json"
    );
    for capability in [
        "provider_management_local",
        "provider_management_remote",
        "raw_url_calls",
    ] {
        assert_eq!(shown["servers"]["research"][capability], true);
    }
    assert_eq!(
        shown["servers"]["research"]["remote_provider_directories"],
        serde_json::json!(["coinbase", "x402-list"])
    );
    assert!(include_str!("../examples/deployments/managed.toml").contains("coinbase_curated"));
    assert_eq!(shown["servers"]["research"]["wallet"], "research");
    assert!(shown["servers"]["research"]["source_registry_file"].is_null());
    assert!(shown.get("provider_management_remote").is_none());
    assert!(!dir.path().join("state").exists());

    // Exercise deployment composition offline; provider-specific schema contracts
    // are covered separately by the Exa regression in provider_catalogs.
    let spec = serde_json::json!({
        "openapi": "3.0.3", "info": {"title": "Search fixture", "version": "1"},
        "paths": {
            "/search": {"post": {"summary": "Search", "requestBody": {"content": {
                "application/json": {"schema": {"type": "object", "properties": {
                    "query": {"type": "string"}, "stream": {"type": "boolean"}
                }}}
            }}}},
            "/contents": {"post": {"summary": "Contents", "requestBody": {"content": {
                "application/json": {"schema": {"type": "object", "properties": {
                    "urls": {"type": "array", "items": {"type": "string"}},
                    "ids": {"type": "array", "items": {"type": "string"}}
                }}}
            }}}}
        }
    });
    std::fs::write(
        dir.path().join("spec.json"),
        serde_json::to_vec(&spec).unwrap(),
    )
    .unwrap();
    config["sources"]["exa"]
        .as_table_mut()
        .unwrap()
        .insert("spec".into(), toml::Value::String("spec.json".into()));
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory.len(), 1);
    assert_eq!(
        inventory[0]
            .tools
            .iter()
            .map(|entry| entry.tool.name.as_str())
            .collect::<Vec<_>>(),
        ["exa_contents", "exa_help", "exa_search"]
    );
    assert!(
        inventory[0]
            .tools
            .iter()
            .filter(|entry| entry.tool.name != "exa_help")
            .all(|entry| entry.tool.method == "POST")
    );
    assert_eq!(inventory[0].management_tools.len(), 8);
    assert!(
        inventory[0]
            .management_tools
            .iter()
            .any(|tool| tool.name == "x402_treazury_providers_list_local")
    );
    assert!(
        inventory[0]
            .management_tools
            .iter()
            .any(|tool| tool.name == "x402_treazury_providers_add_remote")
    );
    assert!(!dir.path().join("state").exists());
}
