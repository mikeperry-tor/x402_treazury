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
daily_input_zec = "0.1"
shield_max_fee_zec = "0.001"
[funding]
base_rpc_url_env = "BASE_RPC"
[wallets.research]
mode = "zcash_rotation"
max_input_zec = "0.02"
max_fee_bps = 500
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
    assert_eq!(shown["wallets"]["research"]["deposit_size"], "2.00");
    assert_eq!(shown["wallets"]["research"]["wait_seconds"], 30);
    assert_eq!(
        shown["treasury"]["state_dir"],
        dir.path().join("state/treasury").to_str().unwrap()
    );
    assert_eq!(shown["funding"]["base_rpc_url_env"], "BASE_RPC");
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
        ("max_fee_bps = 500", "max_fee_bps = 10001"),
        (
            "max_input_zec = \"0.02\"",
            "max_input_zec = \"0.000000001\"",
        ),
        ("max_input_zec = \"0.02\"", "max_input_zec = \"0\""),
        (
            "mode = \"zcash_rotation\"",
            "mode = \"zcash_rotation\"\nprivate_key_env = \"KEY\"",
        ),
        ("[funding]", "[unknown]"),
        (
            "base_rpc_url_env = \"BASE_RPC\"",
            "base_rpc_url_env = \"BASE_RPC\"\nconfidentiality = \"invalid\"",
        ),
        ("max_fee_bps = 500", "max_fee_bps = 500\nwait_seconds = 0"),
        (
            "max_fee_bps = 500",
            "max_fee_bps = 500\ndeposit_size = \"0.0000001\"",
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
        toml::from_str::<WalletConfig>("mode='static'\nprivate_key_env='KEY'\ndeposit_size='5'")
            .is_err()
    );
    assert!(toml::from_str::<WalletConfig>("mode='zcash_rotation'\nmax_fee_bps=500").is_err());
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
