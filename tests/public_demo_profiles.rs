//! Both demo paths are inspectable offline before supplying funds or secrets.
use std::path::Path;
use x402_treazury::deployment::Deployment;

#[tokio::test]
async fn public_demo_profiles_expose_one_bounded_tool_without_credentials() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["public-swap-demo.toml", "public-payment-demo.toml"] {
        let path = root.join("examples/deployments").join(name);
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
            assert_eq!(shown["wallets"]["demo"]["funding_amount_usdc"], "5.00");
            assert_eq!(shown["wallets"]["demo"]["max_funding_spend_zec"], "0.006");
            assert_eq!(shown["treasury"]["daily_treasury_spend_limit_zec"], "0.012");
            assert!(shown["funding"]["near_api_key_env"].is_null());
            assert!(shown["funding"]["near_user_session_env"].is_null());
        }
        let deployment = Deployment::load(&path).await.unwrap();
        let inventory = deployment.inventory();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].tools.len(), 1);
        let tool = &inventory[0].tools[0].tool;
        assert_eq!(tool.name, "socialfetch_twitter_profiles_handle");
        assert_eq!(tool.path, "/v1/twitter/profiles/{handle}");
    }
}
