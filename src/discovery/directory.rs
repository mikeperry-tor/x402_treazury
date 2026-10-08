//! Built-in directory search; schema loading is offline, calls use ordinary PaidClient.
use crate::catalog::{Config, ToolSpec};
use std::sync::OnceLock;

pub const BASE: &str = "https://x402-list.com/api/v1";

pub fn tool() -> &'static ToolSpec {
    static TOOL: OnceLock<ToolSpec> = OnceLock::new();
    TOOL.get_or_init(|| {
        let mut config: Config = toml::from_str(include_str!("../../providers/x402-list.toml"))
            .expect("bundled directory provider is valid");
        config.help_url = None;
        config.include_operations = vec!["GET /services".into()];
        let document = serde_json::from_str(include_str!(
            "../../providers/x402-list/search.openapi.json"
        ))
        .expect("bundled directory search schema is valid");
        crate::catalog::build_tools(&config, &document, "x402_list")
            .expect("bundled directory search builds")
            .remove(0)
    })
}
