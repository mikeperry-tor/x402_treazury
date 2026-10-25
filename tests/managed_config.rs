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
max_refund_shielding_fee_zec = "0.001"
[funding]
base_rpc_url_env = "BASE_RPC"
[wallets.research]
mode = "zcash_rotation"
max_funding_spend_zec = "0.02"
max_conversion_overhead_percent = 5
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
    assert!(shown["wallets"]["research"].get("wait_seconds").is_none());
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
            "max_conversion_overhead_percent = 5",
            "max_conversion_overhead_percent = 101",
        ),
        (
            "max_funding_spend_zec = \"0.02\"",
            "max_funding_spend_zec = \"0.000000001\"",
        ),
        (
            "max_funding_spend_zec = \"0.02\"",
            "max_funding_spend_zec = \"0\"",
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
        (
            "max_conversion_overhead_percent = 5",
            "max_conversion_overhead_percent = 5\nfunding_amount_usdc = \"0.0000001\"",
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
    toml::from_str::<WalletConfig>("mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5\n[reserves.small]\nfunding_amount_usdc='2'")
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
fn funding_limits_are_exact_and_old_names_are_rejected() {
    use x402_treazury::rotation::config::FundingConfig;
    for field in ["daily_funding_limit_usdc", "total_funding_limit_usdc"] {
        let error = toml::from_str::<FundingConfig>(&format!("{field}='20'"))
            .err()
            .expect("removed budget must fail")
            .to_string();
        assert!(
            error.contains("was removed") && error.contains("spendable treasury"),
            "{error}"
        );
    }
    for (target, max, valid) in [
        ("2", "3", true),
        ("2", "2", true),
        ("2", "1.99", false),
        ("2", "NaN", false),
    ] {
        let config: WalletConfig = toml::from_str(&format!(
            "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5\n[reserves.small]\nfunding_amount_usdc='{target}'\nmax_funding_amount_usdc='{max}'"
        )).unwrap();
        assert_eq!(config.validate().is_ok(), valid);
    }
    for obsolete in [
        "max_input_zec='0.01'",
        "deposit_size='2'",
        "max_price_usd='0.1'",
        "max_fee_bps=500",
    ] {
        assert!(
            toml::from_str::<WalletConfig>(&format!(
                "mode='zcash_rotation'\nmax_conversion_overhead_percent=5\n{obsolete}"
            ))
            .is_err()
        );
    }
    for obsolete in ["daily_input_zec='0.01'", "shield_max_fee_zec='0.001'"] {
        assert!(
            toml::from_str::<x402_treazury::rotation::config::TreasuryConfig>(&format!(
                "state_dir='state'\n{obsolete}"
            ))
            .is_err()
        );
    }
}

#[tokio::test]
async fn comprehensive_example_exposes_reserves_and_per_operation_fee_defaults_offline() {
    let shown = Deployment::show_config(std::path::Path::new(
        "examples/deployments/comprehensive.toml",
    ))
    .await
    .unwrap();
    assert!(shown["funding"].get("daily_funding_limit_usdc").is_none());
    assert!(shown["funding"].get("total_funding_limit_usdc").is_none());
    assert_eq!(shown["wallets"].as_object().unwrap().len(), 2);
    assert_eq!(
        shown["wallet_summary"]["initial_inventory_target_atomic"],
        "4000000"
    );
    assert_eq!(shown["wallets"]["web"]["max_api_payment_usdc"], "0.10");
    assert_eq!(shown["wallets"]["company"]["max_api_payment_usdc"], "0.50");
    for name in ["web", "company"] {
        assert_eq!(
            shown["wallets"][name]["reserves"]["small"]["funding_amount_usdc"],
            "2.00"
        );
        assert_eq!(
            shown["wallets"][name]["reserves"]["small"]["max_funding_amount_usdc"],
            "3.00"
        );
        assert_eq!(shown["wallets"][name]["max_funding_spend_zec"], "0.006");
    }
    assert!(
        shown["treasury"]
            .get("daily_treasury_spend_limit_zec")
            .is_none()
    );
    assert_eq!(
        shown["treasury"]["max_funding_transaction_fee_zec"],
        "0.0003"
    );
    assert_eq!(shown["treasury"]["max_refund_shielding_fee_zec"], "0.0003");
}

#[tokio::test]
async fn managed_funding_uses_spendable_treasury_without_aggregate_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, config()).unwrap();
    Deployment::show_config(&path).await.unwrap();
    for (table, setting) in [
        ("funding", "daily_funding_limit_usdc"),
        ("funding", "total_funding_limit_usdc"),
        ("treasury", "daily_treasury_spend_limit_zec"),
    ] {
        std::fs::write(
            &path,
            config().replace(&format!("[{table}]"), &format!("[{table}]\n{setting}='20'")),
        )
        .unwrap();
        let error = Deployment::show_config(&path)
            .await
            .expect_err("removed setting must fail");
        let error = format!("{error:#}");
        assert!(error.contains("was removed"), "{error}");
        assert!(error.contains(setting), "{error}");
    }
    assert!(!dir.path().join("state").exists());
}

#[tokio::test]
async fn obsolete_admission_timeout_is_accepted_but_not_exposed_as_effective_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.toml");
    for value in [0, 1, 30, 7200] {
        std::fs::write(
            &path,
            config().replace(
                "max_conversion_overhead_percent = 5",
                &format!("max_conversion_overhead_percent = 5\nwait_seconds = {value}"),
            ),
        )
        .unwrap();
        let shown = Deployment::show_config(&path).await.unwrap();
        assert!(shown["wallets"]["research"].get("wait_seconds").is_none());
    }
}

#[test]
fn reserve_shape_has_required_small_optional_large_and_explicit_output_ceilings() {
    use x402_treazury::rotation::config::ReservesConfig;
    for (text, expected) in [
        ("[small]\nfunding_amount_usdc='2'", "2000000"),
        (
            "[small]\nfunding_amount_usdc='2'\n[large]\nfunding_amount_usdc='10'",
            "12000000",
        ),
        (
            "[small]\nfunding_amount_usdc='2'\n[large]\nfunding_amount_usdc='2'",
            "4000000",
        ),
    ] {
        let reserves: ReservesConfig = toml::from_str(text).unwrap();
        reserves.validate().unwrap();
        assert_eq!(
            reserves
                .initial_inventory_target_atomic()
                .unwrap()
                .to_string(),
            expected
        );
        assert_eq!(reserves.small.maximum_atomic().unwrap(), 2_000_000);
    }
    for text in [
        "",
        "[large]\nfunding_amount_usdc='10'",
        "[small]\nmax_funding_amount_usdc='2'",
        "[small]\nfunding_amount_usdc='2'\nmax_api_payment_usdc='1'",
        "[small]\nfunding_amount_usdc='2'\n[medium]\nfunding_amount_usdc='5'",
    ] {
        assert!(toml::from_str::<ReservesConfig>(text).is_err(), "{text}");
    }
    for text in [
        "[small]\nfunding_amount_usdc='0'",
        "[small]\nfunding_amount_usdc='2'\nmax_funding_amount_usdc='1.99'",
        "[small]\nfunding_amount_usdc='2'\n[large]\nfunding_amount_usdc='1.99'",
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

#[test]
fn managed_cap_is_explicit_and_independent_of_reserve_targets() {
    let base = "mode='zcash_rotation'\nmax_conversion_overhead_percent=5\n[reserves.small]\nfunding_amount_usdc='2'";
    assert!(toml::from_str::<WalletConfig>(base).is_err());
    for cap in ["0", "0.01", "10", "none", "off", ""] {
        let config: WalletConfig =
            toml::from_str(&format!("max_api_payment_usdc='{cap}'\n{base}")).unwrap();
        config.validate().unwrap();
        let shown = serde_json::to_value(config).unwrap();
        assert_eq!(shown["max_retirement_remainder_usdc"], "0");
        assert!(shown.get("funding_amount_usdc").is_none());
        assert!(shown.get("limit_payments_to_funding_target").is_none());
    }
    for setting in [
        "funding_amount_usdc='2'",
        "max_funding_amount_usdc='3'",
        "limit_payments_to_funding_target=false",
    ] {
        let error =
            toml::from_str::<WalletConfig>(&format!("{setting}\nmax_api_payment_usdc='1'\n{base}"))
                .err()
                .expect("replaced setting must fail")
                .to_string();
        assert!(
            error.contains("moved to reserves.small") || error.contains("was removed"),
            "{error}"
        );
    }
    for setting in [
        "max_live_wallets=3",
        "retention=true",
        "tiers={small='2'}",
        "small_funding_amount_usdc='2'",
    ] {
        assert!(
            toml::from_str::<WalletConfig>(&format!("{setting}\nmax_api_payment_usdc='1'\n{base}"))
                .is_err()
        );
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
        json
    }
    let wallet = roundtrip::<WalletConfig>("mode='static'\nprivate_key_env='KEY'");
    assert_eq!(wallet["max_api_payment_usdc"], "1.00");
    let wallet = roundtrip::<WalletConfig>(
        "mode='zcash_rotation'\nmax_api_payment_usdc='1'\nmax_conversion_overhead_percent=5\nwait_seconds=123\n[reserves.small]\nfunding_amount_usdc='2'",
    );
    assert_eq!(wallet["max_attempts"], 3);
    assert_eq!(wallet["max_retirement_remainder_usdc"], "0");
    assert!(wallet.get("wait_seconds").is_none());
    let treasury = roundtrip::<TreasuryConfig>("state_dir='state'");
    assert_eq!(treasury["confirmations"], 3);
    assert_eq!(treasury["max_funding_transaction_fee_zec"], "0.0003");
    assert!(treasury.get("id").is_none());
    assert!(treasury.get("key_file").is_none());
    let funding = roundtrip::<FundingConfig>("");
    assert_eq!(funding["auto_fund"], true);
    assert_eq!(funding["base_confirmations"], 12);
    assert_eq!(funding["quote_deadline_seconds"], 7200);
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
