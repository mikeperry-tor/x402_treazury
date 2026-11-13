//! Both demo paths are inspectable offline before supplying funds or secrets.
use std::path::Path;
use x402_treazury::deployment::Deployment;

#[tokio::test]
async fn public_demo_profiles_expose_one_bounded_tool_without_credentials() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["public-swap-demo.toml", "public-payment-demo.toml"] {
        let template = root.join("tests/live/deployments").join(name);
        let dir = tempfile::tempdir().unwrap();
        let mut config: toml::Value =
            toml::from_str(&std::fs::read_to_string(&template).unwrap()).unwrap();
        let state = dir.path().join("state");
        if let Some(treasury) = config.get_mut("treasury") {
            treasury["state_dir"] = toml::Value::String(state.to_str().unwrap().into());
            std::fs::create_dir(&state).unwrap();
            // Inspection must not open even incompatible configured state.
            std::fs::write(state.join("state.sqlite"), b"not a SQLite wallet").unwrap();
        }
        for (_, source) in config["sources"].as_table_mut().unwrap().iter_mut() {
            for field in ["extends", "spec"] {
                if let Some(value) = source.get_mut(field) {
                    let absolute = template
                        .parent()
                        .unwrap()
                        .join(value.as_str().unwrap())
                        .canonicalize()
                        .unwrap();
                    *value = toml::Value::String(absolute.to_str().unwrap().into());
                }
            }
        }
        let path = dir.path().join(name);
        std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
        let shown = Deployment::show_config(&path).await.unwrap();
        assert_eq!(shown["wallets"]["demo"]["max_api_payment_usdc"], "0.05");
        if name == "public-swap-demo.toml" {
            assert_eq!(shown["treasury_identity"], "from_wallet_state_at_runtime");
            let state = Path::new(shown["treasury"]["state_dir"].as_str().unwrap());
            assert_eq!(
                Path::new(shown["treasury"]["key_file"].as_str().unwrap()),
                state.join("wallet.key")
            );
            assert_eq!(shown["funding"]["auto_fund"], false);
            assert_eq!(shown["funding"]["confidentiality"], "public");
            assert_eq!(
                shown["wallets"]["demo"]["max_retirement_remainder_usdc"],
                "0"
            );
            assert_eq!(
                shown["wallets"]["demo"]["reserves"]["small"]["min_funding_amount_usdc"],
                "2.00"
            );
            assert_eq!(
                shown["wallets"]["demo"]["reserves"]["small"]["max_funding_amount_usdc"],
                "3.00"
            );
            assert!(
                shown["treasury"]
                    .get("daily_treasury_spend_limit_zec")
                    .is_none()
            );
            assert!(shown["funding"]["near_api_key_env"].is_null());
            assert!(shown["funding"]["near_user_session_env"].is_null());
        }
        let deployment = Deployment::load(&path).await.unwrap();
        if state.exists() {
            assert_eq!(
                std::fs::read(state.join("state.sqlite")).unwrap(),
                b"not a SQLite wallet"
            );
        }
        let inventory = deployment.inventory();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].tools.len(), 1);
        let tool = &inventory[0].tools[0].tool;
        assert_eq!(tool.name, "socialfetch_twitter_profiles_handle");
        assert_eq!(tool.path, "/v1/twitter/profiles/{handle}");
    }
}
