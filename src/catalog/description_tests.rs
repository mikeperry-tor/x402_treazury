//! Canonical pricing survives expiry while runtime rendering replaces owned fragments.
use super::*;
use crate::output::ObservedPrice;
use alloy_primitives::U256;

fn observed() -> Option<ObservedPrice> {
    Some(ObservedPrice {
        amount: U256::from(5_000),
    })
}

fn tool(cfg: Config, op: Value) -> ToolSpec {
    build_tools(&cfg, &json!({"paths":{"/read":{"get":op}}}), "api")
        .unwrap()
        .remove(0)
}

#[test]
fn observed_rendering_replaces_generated_fixed_range_and_unknown_prices() {
    for (pricing, suffix) in [
        (
            json!({"amount":"0.01","currency":"USDC"}),
            "Cost: ~$0.01/call [spec].",
        ),
        (
            json!({"min":"0.01","max":"0.02","currency":"USD"}),
            "Cost: ~$0.01–$0.02/call [spec].",
        ),
        (Value::Null, "Cost: unknown."),
    ] {
        let tool = tool(
            Config {
                pricing_key: Some("x-price".into()),
                ..Default::default()
            },
            json!({"description":"Search company records; a $9 returned value is data.","x-price":pricing}),
        );
        let fallback = tool.description.clone();
        assert_eq!(
            fallback,
            format!("Search company records; a $9 returned value is data. {suffix}")
        );
        let expected =
            "Search company records; a $9 returned value is data. Observed cost: ~$0.005/call.";
        assert_eq!(tool.render_description(observed()), expected);
        assert_eq!(tool.render_description(observed()), expected);
        assert_eq!(tool.render_description(None), fallback);
        assert_eq!(tool.description, fallback);
    }
}

#[test]
fn exact_generated_suffix_is_replaced_once_without_parsing_vendor_prose() {
    let tool = tool(
        Config {
            pricing_key: Some("x-price".into()),
            ..Default::default()
        },
        json!({"description":"Search. Cost: ~$0.01/call [spec].", "x-price":{"amount":"0.01","currency":"USD"}}),
    );
    assert_eq!(tool.description, "Search. Cost: ~$0.01/call [spec].");
    assert_eq!(
        tool.render_description(observed()),
        "Search. Observed cost: ~$0.005/call."
    );
}

#[test]
fn observed_rendering_replaces_the_whole_owned_credit_section() {
    let cfg = Config {
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "x-credit".into(),
            usdc_per_credit: "0.014".into(),
        }),
        pricing_key: Some("x-price".into()),
        ..Default::default()
    };
    let tool = tool(
        cfg,
        json!({"description":"Find posts.","x-price":"Original credit conditions.","x-credit":{
            "version":1,"baseCredits":1,"maxCredits":1,"normalizationFailureCredits":0,"surcharges":[]
        }}),
    );
    assert!(tool.description.contains("Original credit conditions."));
    assert!(tool.description.contains("0.014 USDC/request"));
    let bound = crate::catalog_state::bind(vec![(
        tool.clone(),
        crate::payment::PaidClient::unsigned(),
        "https://api.example.com".into(),
    )])
    .remove(0);
    assert_eq!(
        bound.description(),
        tool.description,
        "catalog metadata cannot erase the credit fallback"
    );
    assert_eq!(
        tool.render_description(observed()),
        "Find posts. Observed cost: ~$0.005/call."
    );
}

#[test]
fn authored_tariff_fragments_work_without_schema_pricing_and_opaque_overrides_stay_intact() {
    let cfg = Config {
        overrides: json!({"api_read":{
            "description":"Read records. Vendor bills successful calls.",
            "pricing_description":"Cost: $0.01 [vendor guide]."
        }}),
        ..Default::default()
    };
    let split = tool(
        cfg,
        json!({"description":"Generated text must stay replaced."}),
    );
    assert_eq!(
        split.description,
        "Read records. Vendor bills successful calls. Cost: $0.01 [vendor guide]."
    );
    assert_eq!(
        split.render_description(observed()),
        "Read records. Vendor bills successful calls. Observed cost: ~$0.005/call."
    );
    let opaque = tool(
        Config {
            overrides: json!({"api_read":{"description":"Opaque $0.01 authored prose."}}),
            ..Default::default()
        },
        json!({}),
    );
    assert_eq!(
        opaque.render_description(observed()),
        "Opaque $0.01 authored prose. Observed cost: ~$0.005/call."
    );
}

#[test]
fn explicit_limits_preserve_fallback_contract_and_visibly_truncate_observed_overlays() {
    let generated = tool(
        Config {
            max_description_chars: Some(8),
            ..Default::default()
        },
        json!({"description":"éééééééé search"}),
    );
    assert_eq!(
        generated.description,
        "éééééééé\n[truncated by max_description_chars=8]"
    );
    assert_eq!(
        generated.render_description(observed()),
        generated.description
    );
    assert_eq!(
        generated.description_rendering.as_ref().unwrap().base,
        "éééééééé search"
    );
    let authored = tool(
        Config {
            max_description_chars: Some(8),
            overrides: json!({"api_read":{"description":"Read original authored description.","pricing_description":"Free."}}),
            ..Default::default()
        },
        json!({}),
    );
    assert_eq!(
        authored.description,
        "Read original authored description. Free."
    );
    assert_eq!(
        authored.render_description(observed()),
        "Read ori\n[truncated by max_description_chars=8]"
    );
    assert_eq!(authored.render_description(None), authored.description);
}

#[test]
fn runtime_rendering_metadata_is_not_serialized_and_restored_descriptions_are_opaque() {
    let original = tool(Config::default(), json!({"description":"Read."}));
    let serialized = serde_json::to_value(&original).unwrap();
    assert!(serialized.get("description_rendering").is_none());
    let restored: ToolSpec = serde_json::from_value(serialized).unwrap();
    assert!(restored.description_rendering.is_none());
    assert_eq!(
        restored.render_description(observed()),
        "Read. Cost: unknown. Observed cost: ~$0.005/call."
    );
    assert_eq!(
        original.render_description(observed()),
        "Read. Observed cost: ~$0.005/call."
    );
}

#[test]
fn only_new_pricing_fragment_opt_in_requires_a_string_and_paired_description() {
    for patch in [
        json!({"pricing_description":4,"description":"Read."}),
        json!({"pricing_description":"Free."}),
        json!({"pricing_description":null,"description":"Read."}),
    ] {
        let cfg = Config {
            overrides: json!({"api_read":patch}),
            ..Default::default()
        };
        assert!(
            build_tools(&cfg, &json!({"paths":{"/read":{"get":{}}}}), "api")
                .unwrap_err()
                .to_string()
                .contains("pricing_description")
        );
    }
    let legacy = tool(
        Config {
            overrides: json!({"api_read":{"description":4,"params":false,"unrelated":7}}),
            ..Default::default()
        },
        json!({"description":"Read."}),
    );
    assert_eq!(legacy.description, "Read. Cost: unknown.");
}

#[test]
fn naming_and_display_keep_unicode_lowercase_and_value_kind_boundaries() {
    assert_eq!(slug("Straße İSTANBUL café"), "stra_e_i_stanbul_caf");
    assert_eq!(slug("__A---B...09__"), "a_b_09");
    // Display pricing retains the existing Unicode regex and textual formatting;
    // it is not financial amount validation or signing authority.
    for (value, expected) in [
        (json!(" 001.23000 "), Some("001.23")),
        (json!("１２.３０"), Some("１２.３０")),
        (json!(u64::MAX), Some("18446744073709551615")),
        (json!(-1), None),
        (json!(1.0), None),
        (json!(true), None),
        (json!("1e3"), None),
        (json!(".50"), None),
    ] {
        assert_eq!(
            crate::pricing::advertised::money(&value).as_deref(),
            expected
        );
    }
}
