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
    let bound = crate::catalog_state::bind(vec![(
        authored.clone(),
        crate::payment::PaidClient::unsigned(),
        "https://api.example.com".into(),
    )])
    .remove(0);
    assert_eq!(
        bound.description(),
        authored.description,
        "advertised catalog metadata cannot replace authored presentation"
    );
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

#[test]
fn external_pricing_enriches_live_descriptions_without_becoming_observed() {
    use crate::pricing::advertised::{NormalizedPrice, PricingConfig, PricingFormat};
    let cfg = Config {
        pricing: Some(PricingConfig {
            source: "https://api.example.com/.well-known/x402".into(),
            format: PricingFormat::X402Resources,
            key: None,
            unit: None,
            usd_as_usdc: false,
        }),
        ..Default::default()
    };
    let tool = build_tools(
        &cfg,
        &json!({"paths":{"/read":{"get":{"summary":"Read records."}}}}),
        "api",
    )
    .unwrap()
    .remove(0);
    let original = tool.description.clone();
    let price = NormalizedPrice::payment_requirements(&json!([{
        "scheme":"exact", "network":"eip155:8453", "asset":crate::payment::USDC, "amount":"10000"
    }]));
    tool.advertised_pricing.replace(
        price,
        "https://api.example.com/.well-known/x402".into(),
        None,
    );
    assert_eq!(
        tool.render_description(None),
        "Read records. Cost: ~$0.01/call [spec]."
    );
    assert_eq!(
        tool.render_description(Some(crate::pricing::ObservedPrice {
            amount: U256::from(5000)
        })),
        "Read records. Observed cost: ~$0.005/call."
    );
    assert_eq!(
        tool.description, original,
        "canonical descriptions/search cursors stay stable"
    );
    tool.advertised_pricing.set_expires(0);
    assert_eq!(tool.render_description(None), original);
}
