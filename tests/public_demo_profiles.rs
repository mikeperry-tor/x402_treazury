//! Both demo paths are inspectable offline before supplying funds or secrets.
use std::path::Path;
use x402_treazure::deployment::Deployment;

#[tokio::test]
async fn public_demo_profiles_expose_one_bounded_tool_without_credentials() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["public-swap-demo.toml", "public-payment-demo.toml"] {
        let path = root.join("examples").join(name);
        let shown = Deployment::show_config(&path).await.unwrap();
        assert_eq!(shown["wallets"]["demo"]["max_price_usd"], "0.05");
        if name == "public-swap-demo.toml" {
            assert_eq!(shown["funding"]["auto_fund"], false);
            assert_eq!(shown["funding"]["confidentiality"], "public");
            assert_eq!(shown["wallets"]["demo"]["deposit_size"], "5.00");
            assert_eq!(shown["wallets"]["demo"]["max_input_zec"], "0.006");
            assert_eq!(shown["treasury"]["daily_input_zec"], "0.012");
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
