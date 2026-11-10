//! Catalog pricing is available on the first local build without observed-cache state.
use super::*;
use alloy_primitives::U256;
#[test]
fn local_catalog_normalizes_once_for_description_and_initial_hint() {
    let cfg = Config {
        spec: "authored.json".into(),
        pricing: Some(crate::pricing::advertised::PricingConfig {
            source: "catalog".into(),
            format: crate::pricing::advertised::PricingFormat::PaymentInfo,
            key: None,
            unit: Some("call".into()),
            usd_as_usdc: true,
        }),
        ..Default::default()
    };
    let root = json!({"paths":{"/read":{"get":{"description":"Read records.","x-payment-info":{"price":{"currency":"USD","amount":"0.015000"}}}}}});
    let tool = build_tools(&cfg, &root, "api").unwrap().remove(0);
    assert!(tool.catalog_hints.is_none());
    assert_eq!(tool.advertised_pricing.amount(), Some(U256::from(15_000)));
    assert_eq!(tool.description, "Read records. Cost: ~$0.015/call [spec].");
    let serialized = serde_json::to_value(&tool).unwrap();
    assert!(serialized.get("advertised_pricing").is_none());
    let restored: ToolSpec = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored.advertised_pricing.amount(), None);
    let mut overrides = cfg.clone();
    overrides.overrides =
        json!({"api_read":{"description":"Authored.","pricing_description":"Cost: $9/record."}});
    let authored = build_tools(&overrides, &root, "api").unwrap().remove(0);
    assert_eq!(authored.description, "Authored. Cost: $9/record.");
    assert_eq!(
        authored.advertised_pricing.amount(),
        Some(U256::from(15_000))
    );
}
#[test]
fn remote_build_requires_acquired_freshness_before_hint_use() {
    let cfg = Config {
        spec: "https://api.example.com/spec".into(),
        pricing: Some(crate::pricing::advertised::PricingConfig {
            source: "catalog".into(),
            format: crate::pricing::advertised::PricingFormat::PaymentInfo,
            key: None,
            unit: Some("call".into()),
            usd_as_usdc: true,
        }),
        ..Default::default()
    };
    let root = json!({"operations":[{"path":"/read","method":"GET","pricing":{"amount":"0.01","currency":"USD"}}]});
    let tool = build_tools(&cfg, &root, "api").unwrap().remove(0);
    assert_eq!(tool.advertised_pricing.amount(), None);
    tool.advertised_pricing.set_fresh_until(Some(
        std::time::Instant::now() + std::time::Duration::from_secs(60),
    ));
    assert_eq!(tool.advertised_pricing.amount(), Some(U256::from(10_000)));
}
