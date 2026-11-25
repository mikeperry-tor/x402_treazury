//! Canonical pricing survives expiry while runtime rendering adds scoped evidence.
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
fn observed_rendering_keeps_generated_fixed_range_and_unknown_prices() {
    for (pricing, suffix) in [
        (
            json!({"amount":"0.01","currency":"USDC","unit":"call"}),
            "Listed: ~$0.01/call [spec].",
        ),
        (
            json!({"min":"0.01","max":"0.02","currency":"USD","unit":"call"}),
            "Listed: ~$0.01–$0.02/call [spec].",
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
        let prose_and_tariff = fallback
            .strip_suffix(" Cost: unknown.")
            .unwrap_or(&fallback);
        let expected = format!("{prose_and_tariff} Sample offer: $0.005 for one request.");
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
        json!({"description":"Search. Cost: ~$0.01/call [spec].", "x-price":{"amount":"0.01","currency":"USD","unit":"call"}}),
    );
    assert_eq!(tool.description, "Search. Listed: ~$0.01/call [spec].");
    assert_eq!(
        tool.render_description(observed()),
        "Search. Listed: ~$0.01/call [spec]. Sample offer: $0.005 for one request."
    );
}

#[test]
fn observed_rendering_preserves_the_whole_owned_credit_section() {
    let cfg = Config {
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "x-credit".into(),
            usdc_per_credit: "0.014".into(),
        }),
        pricing_key: Some("x-price".into()),
        ..Default::default()
    };
    let mut operation = json!({"description":"Find posts.","x-price":"Original credit conditions.","x-credit":{
        "version":1,"baseCredits":1,"maxCredits":1,"normalizationFailureCredits":0,"surcharges":[]
    }});
    let compiled = tool(cfg.clone(), operation.clone());
    operation["description"] = json!(compiled.description);
    let tool = tool(cfg, operation);
    assert_eq!(tool.description, compiled.description);
    assert!(tool.description.contains("Original credit conditions."));
    assert!(tool.description.contains("Estimated cost: $0.014/request."));
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
        format!("{} Sample offer: $0.005 for one request.", tool.description)
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
        concat!(
            "Read records. Vendor bills successful calls. ",
            "Configured pricing: Cost: $0.01 [vendor guide]. ",
            "Sample offer: $0.005 for one request."
        )
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
        "Opaque $0.01 authored prose. Sample offer: $0.005 for one request."
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
        generated.description_rendering.as_ref().unwrap().prose,
        "éééééééé search"
    );
    let authored = tool(
        Config {
            max_description_chars: Some(8),
            overrides: json!({"api_read":{"description":"雪🙂雪🙂雪🙂雪🙂 authored text","pricing_description":"Free."}}),
            ..Default::default()
        },
        json!({}),
    );
    assert_eq!(
        authored.description,
        "雪🙂雪🙂雪🙂雪🙂\n[truncated by max_description_chars=8]"
    );
    assert_eq!(
        authored.render_description(observed()),
        "雪🙂雪🙂雪🙂雪🙂\n[truncated by max_description_chars=8]"
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
        "Read. Cost: unknown. Sample offer: $0.005 for one request."
    );
    assert_eq!(
        original.render_description(observed()),
        "Read. Sample offer: $0.005 for one request."
    );
}

#[test]
fn description_and_pricing_overrides_edit_independent_fields() {
    let op = json!({
        "description":"Catalog description.",
        "x-price":{"amount":"0.01","currency":"USDC","unit":"call"}
    });
    for (patch, description) in [
        (
            json!({"description":"Authored prose."}),
            "Authored prose. Listed: ~$0.01/call [spec].",
        ),
        (
            json!({"pricing_description":"Billed only on success."}),
            "Catalog description. Listed: ~$0.01/call [spec]. Billed only on success.",
        ),
        (
            json!({"description":"Authored prose.","pricing_description":"Listed: ~$0.01/call [spec]."}),
            "Authored prose. Listed: ~$0.01/call [spec].",
        ),
        (
            json!({"pricing_description":"Cost: ~$0.01/call [spec]. Billed only on success."}),
            "Catalog description. Listed: ~$0.01/call [spec]. Billed only on success.",
        ),
        (
            json!({"pricing_description":"Cost: ~$0.01/call [spec, USDC]. Billed only on success."}),
            "Catalog description. Listed: ~$0.01/call [spec]. Billed only on success.",
        ),
        (
            json!({"pricing_description":"Cost: ~$0.02/call [spec, USDC]."}),
            "Catalog description. Listed: ~$0.01/call [spec]. Configured pricing: Cost: ~$0.02/call [spec, USDC].",
        ),
    ] {
        let selected = tool(
            Config {
                pricing_key: Some("x-price".into()),
                overrides: json!({"api_read":patch}),
                ..Default::default()
            },
            op.clone(),
        );
        assert_eq!(selected.description, description);
        assert_eq!(
            selected.advertised_pricing.amount(),
            Some(U256::from(10_000))
        );
    }
}

#[test]
fn legacy_usdc_tariff_in_catalog_prose_is_rendered_once() {
    let selected = tool(
        Config {
            pricing_key: Some("x-price".into()),
            ..Default::default()
        },
        json!({
            "description":"Read records. Cost: ~$0.01/call [spec].",
            "x-price":{"amount":"0.01","currency":"USDC","unit":"call"}
        }),
    );
    assert_eq!(
        selected.description,
        "Read records. Listed: ~$0.01/call [spec]."
    );
}

#[test]
fn legacy_tariff_deduplication_preserves_distinct_currency_and_extra_terms() {
    let operation = json!({
        "description":"Read.",
        "x-price":{"amount":"0.01","currency":"USD","pricingTerms":"Billed on success."}
    });
    let listed = "Read. Listed: ~$0.01 [spec, no unit]. Billing terms [spec] Billed on success.";
    for (note, expected) in [
        (
            "Cost: ~$0.01/call [spec]. Empty results are free.",
            format!(
                "{listed} Configured pricing: Cost: ~$0.01/call [spec]. Empty results are free."
            ),
        ),
        (
            "Cost: ~$0.01 (unit unspecified) [spec]. Billing terms [spec] Billed on success. Empty results are free.",
            format!("{listed} Empty results are free."),
        ),
        (
            "Listed: ~$0.01 [spec, no unit]. Empty results are free.",
            format!("{listed} Empty results are free."),
        ),
        (
            "Cost: ~$0.01 (unit unspecified) [spec, USDC].",
            format!("{listed} Configured pricing: Cost: ~$0.01 (unit unspecified) [spec, USDC]."),
        ),
        (
            "Billed per item in $.results.",
            format!("{listed} Billed per item in $.results."),
        ),
    ] {
        let selected = tool(
            Config {
                pricing_key: Some("x-price".into()),
                overrides: json!({"api_read":{"pricing_description":note}}),
                ..Default::default()
            },
            operation.clone(),
        );
        assert_eq!(selected.description, expected);
        assert_eq!(
            selected.render_description(observed()),
            format!("{expected} Sample offer: $0.005 for one request.")
        );
    }
}

#[test]
fn legacy_ranges_match_only_with_a_known_unit() {
    for (unit, suffix, note) in [
        (
            Value::Null,
            " [spec, dynamic, no unit]",
            "Configured pricing: Cost: ~$0.01–$0.10/call [spec]. Some calls are free.",
        ),
        (
            json!("call"),
            "/call [spec, dynamic]",
            "Some calls are free.",
        ),
    ] {
        let mut range = json!({"min":"0.01","max":"0.10","currency":"USD","mode":"dynamic"});
        if !unit.is_null() {
            range["unit"] = unit;
        }
        let selected = tool(
            Config {
                pricing_key: Some("x-price".into()),
                overrides: json!({"api_read":{"pricing_description":
                    "Cost: ~$0.01–$0.10/call [spec]. Some calls are free."}}),
                ..Default::default()
            },
            json!({"description":"Read.","x-price":range}),
        );
        assert_eq!(
            selected.description,
            format!("Read. Listed: ~$0.01–$0.10{suffix}. {note}")
        );
    }
}

#[test]
fn billing_terms_with_offer_like_text_keep_a_distinct_configured_price() {
    let selected = tool(
        Config {
            pricing_key: Some("x-price".into()),
            overrides: json!({"api_read":{"pricing_description":"Cost: ~$0.01/call [spec]."}}),
            ..Default::default()
        },
        json!({"summary":"Read.","x-price":{
            "amount":"0.02","currency":"USD","unit":"call",
            "conditions":"Advertised exact offer: ~$0.01/request [external pricing]."
        }}),
    );
    assert!(
        selected
            .description
            .ends_with("Configured pricing: Cost: ~$0.01/call [spec].")
    );
}

#[test]
fn the_legacy_dynamic_label_matches_a_metered_tariff() {
    let selected = tool(
        Config {
            pricing_key: Some("x-price".into()),
            overrides: json!({"api_read":{"pricing_description":
                "Cost: ~$0.01/call [spec, dynamic]. Empty results are free."}}),
            ..Default::default()
        },
        json!({"description":"Read.","x-price":{
            "amount":"0.01","currency":"USD","unit":"call","mode":"metered"
        }}),
    );
    assert_eq!(
        selected.description,
        "Read. Listed: ~$0.01/call [spec, metered]. Empty results are free."
    );
    assert!(selected.advertised_pricing.amount().is_none());
}

#[test]
fn strict_inspection_reports_bad_edits_on_hidden_tools_while_serving_keeps_good_fields() {
    let mut cfg = Config {
        help_url: Some("https://example.com/guide".into()),
        overrides: json!({
            "api_read":{"description":"Authored.","pricing_description":4,"params":false,"unrelated":7},
            "api_help":{"description":"Read the guide."}
        }),
        ..Default::default()
    };
    let root = json!({"paths":{"/read":{"get":{"description":"Catalog description."}}}});
    let built = build_catalog(&cfg, &root, "api").unwrap();
    assert_eq!(built.tools[0].description, "Authored. Cost: unknown.");
    assert_eq!(built.tools[1].description, "Read the guide.");
    assert_eq!(built.override_issues.len(), 3);
    assert!(
        built
            .override_issues
            .iter()
            .any(|issue| issue.problem == OverrideProblem::UnknownField
                && issue.target.as_deref() == Some("unrelated"))
    );
    assert!(built.check_overrides().is_err());

    cfg.exclude_tools = vec!["api_read".into()];
    let hidden = build_catalog(&cfg, &root, "api").unwrap();
    assert_eq!(hidden.tools.len(), 1);
    assert!(hidden.override_issues.iter().any(|issue| {
        issue.tool.as_deref() == Some("api_read") && issue.problem == OverrideProblem::UnknownField
    }));
    assert!(hidden.check_overrides().is_err());
    assert!(hidden.complete().is_ok());
}

#[test]
fn strict_inspection_rejects_malformed_override_tables() {
    let root = json!({"paths":{"/read":{"get":{"description":"Catalog description."}}}});
    for (overrides, tool) in [
        (json!(false), None),
        (json!({"api_read":false}), Some("api_read")),
    ] {
        let cfg = Config {
            overrides,
            ..Default::default()
        };
        let built = build_catalog(&cfg, &root, "api").unwrap();
        assert_eq!(
            built.tools[0].description,
            "Catalog description. Cost: unknown."
        );
        assert_eq!(built.override_issues.len(), 1);
        assert_eq!(built.override_issues[0].tool.as_deref(), tool);
        assert_eq!(built.override_issues[0].problem, OverrideProblem::Malformed);
        assert!(built.check_overrides().is_err());
    }
}

#[test]
fn closing_an_input_keeps_the_schema_and_router_in_agreement() {
    let root = json!({"paths":{
        "/closed":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","properties":{"id":{"type":"integer"}},"additionalProperties":false
        }}}}}},
        "/open":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","properties":{"id":{"type":"integer"}},"additionalProperties":true
        }}}}}}
    }});
    let cfg = Config {
        additional_properties: Some(true),
        ..Default::default()
    };
    let tools = build_tools(&cfg, &root, "api").unwrap();
    let args = json!({"id":1,"extra":"accepted only by the open operation"});
    for (tool, accepts_more) in tools.iter().zip([false, true]) {
        assert_eq!(tool.input_schema["additionalProperties"], accepts_more);
        assert_eq!(
            tool.route("https://example.com", args.as_object().unwrap())
                .is_ok(),
            accepts_more
        );
    }
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
