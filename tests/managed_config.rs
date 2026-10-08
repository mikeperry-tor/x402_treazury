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
daily_treasury_spend_limit_zec = "0.1"
max_refund_shielding_fee_zec = "0.001"
[funding]
base_rpc_url_env = "BASE_RPC"
[wallets.research]
mode = "zcash_rotation"
max_funding_spend_zec = "0.02"
max_conversion_overhead_percent = 5
[sources.api]
spec = "spec.json"
probe_pricing = false
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
    assert_eq!(shown["wallets"]["research"]["funding_amount_usdc"], "2.00");
    assert_eq!(shown["wallets"]["research"]["wait_seconds"], 30);
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
    #[cfg(not(feature = "zcash"))]
    {
        let env = BTreeMap::from([("TOKEN".into(), "secret".into())]);
        let error = match Deployment::load(&path).await.unwrap().bind(&env).await {
            Ok(_) => panic!("managed serving accepted without zcash feature"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains("--features zcash"), "{error}");
    }
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
            "max_conversion_overhead_percent = 5\nwait_seconds = 0",
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
    toml::from_str::<WalletConfig>("mode='zcash_rotation'\nmax_conversion_overhead_percent=5")
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
    for (daily, total, valid) in [
        ("20.00", "50", true),
        ("0", "50", false),
        ("20", "0.0000001", false),
        ("NaN", "50", false),
        ("20", "9223372036854.775808", false),
    ] {
        let config: FundingConfig = toml::from_str(&format!(
            "daily_funding_limit_usdc='{daily}'\ntotal_funding_limit_usdc='{total}'"
        ))
        .unwrap();
        assert_eq!(config.validate().is_ok(), valid);
    }
    for (target, max, valid) in [
        ("2", "3", true),
        ("2", "2", true),
        ("2", "1.99", false),
        ("2", "NaN", false),
    ] {
        let config: WalletConfig = toml::from_str(&format!(
            "mode='zcash_rotation'\nfunding_amount_usdc='{target}'\nmax_funding_amount_usdc='{max}'\nmax_conversion_overhead_percent=5"
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
async fn privacy_example_exposes_usdc_budgets_and_advanced_fee_defaults_offline() {
    let shown = Deployment::show_config(std::path::Path::new("examples/deployments/privacy.toml"))
        .await
        .unwrap();
    assert_eq!(shown["funding"]["daily_funding_limit_usdc"], "20.00");
    assert!(shown["funding"]["total_funding_limit_usdc"].is_null());
    for name in ["web", "social", "company"] {
        assert_eq!(shown["wallets"][name]["funding_amount_usdc"], "2.00");
        assert_eq!(shown["wallets"][name]["max_funding_amount_usdc"], "3.00");
        assert_eq!(shown["wallets"][name]["max_funding_spend_zec"], "0.006");
    }
    assert_eq!(shown["treasury"]["daily_treasury_spend_limit_zec"], "0.012");
    assert_eq!(
        shown["treasury"]["max_funding_transaction_fee_zec"],
        "0.0003"
    );
    assert_eq!(shown["treasury"]["max_refund_shielding_fee_zec"], "0.0003");
}

#[tokio::test]
async fn managed_profiles_require_at_least_one_aggregate_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = config().replace("daily_treasury_spend_limit_zec = \"0.1\"\n", "");
    std::fs::write(&path, &text).unwrap();
    let error = Deployment::show_config(&path)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("managed funding requires"), "{error}");
    for setting in ["daily_funding_limit_usdc", "total_funding_limit_usdc"] {
        std::fs::write(
            &path,
            text.replace("[funding]", &format!("[funding]\n{setting}='20'")),
        )
        .unwrap();
        Deployment::show_config(&path).await.unwrap();
    }
}
