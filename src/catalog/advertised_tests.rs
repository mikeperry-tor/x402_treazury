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
    assert_eq!(
        tool.description,
        "Read records. Listed: ~$0.015/call [spec]."
    );
    let serialized = serde_json::to_value(&tool).unwrap();
    assert!(serialized.get("advertised_pricing").is_none());
    let restored: ToolSpec = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored.advertised_pricing.amount(), None);
    let mut overrides = cfg.clone();
    overrides.overrides =
        json!({"api_read":{"description":"Authored.","pricing_description":"Cost: $9/record."}});
    let authored = build_tools(&overrides, &root, "api").unwrap().remove(0);
    assert_eq!(
        authored.description,
        "Authored. Listed: ~$0.015/call [spec]. Configured pricing: Cost: $9/record."
    );
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
        overrides: json!({"api_read":{"pricing_description":"Vendor lists $0.05 per record."}}),
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
        "Read records. Advertised exact offer: ~$0.01/request [external pricing]. Configured pricing: Vendor lists $0.05 per record."
    );
    assert_eq!(
        tool.render_description(Some(crate::pricing::ObservedPrice {
            amount: U256::from(5000)
        })),
        "Read records. Advertised exact offer: ~$0.01/request [external pricing]. Configured pricing: Vendor lists $0.05 per record. Sample offer: $0.005 for one request."
    );
    assert_eq!(
        tool.description, original,
        "canonical descriptions/search cursors stay stable"
    );
    tool.advertised_pricing.set_expires(0);
    assert_eq!(tool.render_description(None), original);
    assert!(tool.advertised_pricing.amount().is_none());
    tool.advertised_pricing.invalidate();
    assert_eq!(
        tool.render_description(Some(crate::pricing::ObservedPrice {
            amount: U256::from(5000)
        })),
        original
    );
}

#[test]
fn legacy_external_offer_alias_preserves_differing_and_sample_only_prices() {
    use crate::pricing::advertised::{NormalizedPrice, PricingConfig, PricingFormat};
    let pricing = PricingConfig {
        source: "https://api.example.com/prices".into(),
        format: PricingFormat::X402Resources,
        key: None,
        unit: None,
        usd_as_usdc: false,
    };
    let cfg = Config {
        pricing: Some(pricing.clone()),
        overrides: json!({"api_read":{"pricing_description":
            "Cost: ~$0.001/call [spec]. Billed on success."}}),
        ..Default::default()
    };
    let tool = build_tools(&cfg, &json!({"paths":{"/read":{"get":{}}}}), "api")
        .unwrap()
        .remove(0);
    for (amount, sample, keeps_configured_price) in [
        ("1000", false, false),
        ("2000", false, true),
        ("1000", true, true),
    ] {
        let mut record = json!({"accepts":[{"scheme":"exact","network":"eip155:8453",
            "asset":crate::payment::USDC,"amount":amount}]});
        if sample {
            record["unit"] = json!("record");
        }
        let price = NormalizedPrice::parse(&pricing, &record);
        tool.advertised_pricing
            .replace(price, pricing.source.clone(), None);
        let rendered = tool.render_description(None);
        assert_eq!(
            rendered.contains("Configured pricing:"),
            keeps_configured_price,
            "{rendered}"
        );
        assert!(rendered.ends_with("Billed on success."));
    }
}

#[test]
fn legacy_external_tariffs_match_current_listed_pricing() {
    use crate::pricing::advertised::{NormalizedPrice, PricingConfig, PricingFormat};
    let pricing = PricingConfig {
        source: "https://api.example.com/prices".into(),
        format: PricingFormat::X402Resources,
        key: None,
        unit: None,
        usd_as_usdc: false,
    };
    for (metadata, legacy, listed) in [
        (
            json!({"price":{"min":"0.01","max":"0.10","currency":"USD","unit":"call","mode":"dynamic"}}),
            "~$0.01–$0.10/call [spec]",
            "~$0.01–$0.10/call [external pricing, dynamic]",
        ),
        (
            json!({"price":{"amount":"0.01","currency":"USD","unit":"call","mode":"metered"}}),
            "~$0.01/call [spec, dynamic]",
            "~$0.01/call [external pricing, metered]",
        ),
        (
            json!({"price":{"amount":"0.01","currency":"USD","unit":"record"}}),
            "~$0.01/record [spec]",
            "~$0.01/record [external pricing]",
        ),
    ] {
        let cfg = Config {
            pricing: Some(pricing.clone()),
            overrides: json!({"api_read":{"pricing_description":
                format!("Cost: {legacy}. Empty results are free.")}}),
            ..Default::default()
        };
        let tool = build_tools(
            &cfg,
            &json!({"paths":{"/read":{"get":{"summary":"Read."}}}}),
            "api",
        )
        .unwrap()
        .remove(0);
        let price = NormalizedPrice::parse(&pricing, &metadata);
        tool.advertised_pricing
            .replace(price, pricing.source.clone(), None);
        assert_eq!(
            tool.render_description(None),
            format!("Read. Listed: {listed}. Empty results are free.")
        );
    }
}

#[test]
fn external_pricing_collapses_a_copied_legacy_tariff_and_exact_offer() {
    use crate::pricing::advertised::{NormalizedPrice, PricingConfig, PricingFormat};
    let price_config = PricingConfig {
        source: "https://api.example.com/.well-known/x402".into(),
        format: PricingFormat::X402Resources,
        key: None,
        unit: None,
        usd_as_usdc: false,
    };
    let price = NormalizedPrice::parse(
        &price_config,
        &json!({
            "price":{"amount":"0.01","currency":"USD","unit":"call",
                "conditions":"Downloads are charged separately."},
            "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]
        }),
    );
    let legacy_offer = "Advertised exact offer: ~$0.001/call [external pricing, USDC].";
    for (older_terms, expected_note) in [
        ("", "Billed on success.".to_owned()),
        (
            "Billing terms [external pricing] Includes email delivery. ",
            format!(
                "Configured pricing: Billing terms [external pricing] Includes email delivery. {legacy_offer} Billed on success."
            ),
        ),
        (
            "Billing terms [external pricing] Never rely on ",
            format!(
                "Configured pricing: Billing terms [external pricing] Never rely on {legacy_offer} Billed on success."
            ),
        ),
    ] {
        let note = format!(
            "Cost: ~$0.01/call [external pricing]. {older_terms}{legacy_offer} Billed on success."
        );
        let cfg = Config {
            pricing: Some(price_config.clone()),
            overrides: json!({"api_read":{"pricing_description":note}}),
            ..Default::default()
        };
        let tool = build_tools(
            &cfg,
            &json!({"paths":{"/read":{"get":{"summary":"Read records."}}}}),
            "api",
        )
        .unwrap()
        .remove(0);
        tool.advertised_pricing
            .replace(price.clone(), price_config.source.clone(), None);
        assert_eq!(
            tool.render_description(None),
            format!(
                "Read records. Listed: ~$0.01/call [external pricing]. Billing conditions [external pricing] Downloads are charged separately. Advertised exact offer: ~$0.001/request [external pricing]. {expected_note}"
            )
        );
    }

    let cfg = Config {
        pricing: Some(price_config.clone()),
        overrides: json!({"api_read":{"pricing_description":format!("{legacy_offer} Billed on success.")}}),
        ..Default::default()
    };
    let tool = build_tools(
        &cfg,
        &json!({"paths":{"/read":{"get":{"summary":"Read records."}}}}),
        "api",
    )
    .unwrap()
    .remove(0);
    let conflict = NormalizedPrice::parse(
        &price_config,
        &json!({
            "authMode":"free", "conditions":"Public records only.",
            "accepts":[{"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"1000"}]
        }),
    );
    tool.advertised_pricing
        .replace(conflict, price_config.source, None);
    assert!(tool.advertised_pricing.amount().is_none());
    assert_eq!(
        tool.render_description(None),
        format!(
            "Read records. Advertised sample offer: ~$0.001/request [external pricing]. Conflicting free and paid metadata. Billing conditions [external pricing] Public records only. Configured pricing: {legacy_offer} Billed on success."
        )
    );
}

#[test]
fn digest_tariffs_work_by_default_and_defer_to_external_pricing() {
    let root = json!({"operations":[{"path":"/read","method":"GET",
        "pricing":{"amount":"0.004","currency":"USDC","unit":"call"}}]});
    let external = Config {
        pricing: Some(crate::pricing::advertised::PricingConfig {
            source: "https://api.example.com/prices".into(),
            format: crate::pricing::advertised::PricingFormat::X402Discovery,
            key: None,
            unit: None,
            usd_as_usdc: false,
        }),
        ..Default::default()
    };
    let tool = build_tools(&external, &root, "api").unwrap().remove(0);
    assert!(tool.description.ends_with("Cost: unknown."));
    assert!(tool.advertised_pricing.amount().is_none());
    let cfg = Config {
        pricing_key: Some("x-declared".into()),
        ..Default::default()
    };
    for cfg in [Config::default(), cfg] {
        let tool = build_tools(&cfg, &root, "api").unwrap().remove(0);
        assert_eq!(tool.advertised_pricing.amount(), Some(U256::from(4000)));
        tool.advertised_pricing.set_expires(0);
        assert!(tool.description.contains("$0.004/call"));
        assert!(tool.render_description(None).ends_with("Cost: unknown."));
        assert!(tool.advertised_pricing.amount().is_none());
    }
}

#[test]
fn a_partial_metered_range_remains_visible_during_an_observed_call() {
    let cfg = Config {
        pricing_key: Some("x-price".into()),
        ..Default::default()
    };
    let root = json!({"paths":{"/read":{"get":{"summary":"Read records.",
        "x-price":{"min":"0.05","currency":"USD","unit":"record","mode":"metered",
            "price":{"fixed":{"amount":"0.05","currency":"USD","unit":"call",
                "conditions":"A download incurs a separate fee."}}}}}}});
    let tool = build_tools(&cfg, &root, "api").unwrap().remove(0);
    assert!(tool.advertised_pricing.amount().is_none());
    let canonical = "Read records. Listed: from ~$0.05/record [spec, metered]. Billing conditions [spec] A download incurs a separate fee.";
    assert_eq!(tool.description, canonical);
    assert_eq!(
        tool.render_description(Some(crate::pricing::ObservedPrice {
            amount: U256::from(500_000)
        })),
        format!("{canonical} Sample offer: $0.5 for one request.")
    );
}

#[test]
fn expiry_of_external_pricing_keeps_local_credit_terms() {
    use crate::pricing::advertised::{NormalizedPrice, PricingConfig, PricingFormat};
    let cfg = Config {
        spec: "local.json".into(),
        pricing: Some(PricingConfig {
            source: "https://api.example.com/prices".into(),
            format: PricingFormat::X402Resources,
            key: None,
            unit: None,
            usd_as_usdc: false,
        }),
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "x-credit".into(),
            usdc_per_credit: "0.014".into(),
        }),
        ..Default::default()
    };
    let root = json!({"paths":{"/read":{"get":{"x-credit":{
        "version":1,"baseCredits":1,"maxCredits":1,"normalizationFailureCredits":0,"surcharges":[]
    }}}}});
    let tool = build_tools(&cfg, &root, "api").unwrap().remove(0);
    let offer = NormalizedPrice::payment_requirements(&json!([
        {"scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,"amount":"10000"}
    ]));
    tool.advertised_pricing
        .replace(offer, cfg.pricing.unwrap().source, None);
    assert!(tool.render_description(None).contains("[external pricing]"));
    tool.advertised_pricing.set_expires(0);
    assert_eq!(tool.render_description(None), tool.description);
    assert!(tool.description.contains("Estimated cost: $0.014/request."));
    tool.advertised_pricing.invalidate();
    assert!(tool.render_description(None).ends_with("Cost: unknown."));
}
