use serde_json::{Value, json};
use std::path::Path;
use x402_treazury::{
    catalog::{self, Config, CreditPricing},
    config,
};

fn tariff() -> Value {
    json!({"version":1,"baseCredits":1,"maxCredits":1,"normalizationFailureCredits":0,"surcharges":[]})
}
fn tools(metadata: Value) -> Vec<catalog::ToolSpec> {
    let cfg = Config {
        pricing_key: Some("credits_text".into()),
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "credit_tariff".into(),
            usdc_per_credit: "0.014".into(),
        }),
        ..Default::default()
    };
    catalog::build_tools(
        &cfg,
        &json!({"paths":{"/test":{"get":{
            "description":"Original description.",
            "credits_text":"Original credit conditions.",
            "credit_tariff": metadata
        }}}}),
        "api",
    )
    .unwrap()
}
#[test]
fn fixed_metered_batch_and_conditional_costs_preserve_credit_terms() {
    let fixed = tools(tariff());
    assert!(
        fixed[0]
            .description
            .contains("Estimated cost: 0.014 USDC/request")
    );
    assert!(fixed[0].description.contains("Original credit conditions."));
    let mut metered = tariff();
    metered["baseCredits"] = json!(2);
    metered["maxCredits"] = json!(2002);
    metered["metered"] = json!({"chargedUnits":"returned_records","creditsPerUnit":2,"defaultUnits":10,"maxUnits":1000,"queryParam":"limit","unit":"record"});
    let metered = tools(metered);
    assert!(
        metered[0]
            .description
            .contains("0.028 USDC base + 0.028 USDC per returned record")
    );
    assert!(
        metered[0]
            .description
            .contains("maximum 28.028 USDC/request")
    );
    let mut batch = tariff();
    batch["baseCredits"] = json!(3);
    batch["maxCredits"] = json!(150);
    batch["batch"] = json!({"unit":"url","maxUnits":50});
    assert!(
        tools(batch)[0]
            .description
            .contains("up to 0.042 USDC/URL; maximum 50 URLs and 2.1 USDC/request")
    );
    let mut media = tariff();
    media["maxCredits"] = json!(49);
    media["surcharges"] = json!([{"credits":48,"label":"+2 credits per hosted asset (up to 24)","queryParam":"hostMedia","when":"boolean_true"}]);
    let media = tools(media);
    assert!(media[0].description.contains("maximum 0.686 USDC/request"));
    assert!(
        media[0]
            .description
            .contains("up to 0.672 USDC extra when hostMedia is true")
    );
    assert!(!media[0].description.contains("0.672 USDC per"));
    let mut zero = tariff();
    zero["baseCredits"] = json!(0);
    zero["maxCredits"] = json!(0);
    assert!(tools(zero)[0].description.contains("0 USDC/request"));
}
#[test]
fn invalid_conversion_is_rejected_without_rounding() {
    for rate in [
        "0",
        "-1",
        "NaN",
        "1e-3",
        "0.0000001",
        "18446744073709551615",
        "",
    ] {
        let cfg = CreditPricing {
            credit_cost_key: "tariff".into(),
            usdc_per_credit: rate.into(),
        };
        assert!(cfg.validate().is_err(), "{rate}");
    }
    let cfg = CreditPricing {
        credit_cost_key: "".into(),
        usdc_per_credit: "0.014".into(),
    };
    assert!(cfg.validate().is_err());
}
#[test]
fn unsupported_metadata_has_visible_fallback_and_warning() {
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let writer = Writer(Arc::default());
    let sink = writer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut future = tariff();
    future["version"] = json!(2);
    let mut unknown = tariff();
    unknown["newBillingRule"] = json!("private-tariff-value");
    let mut negative = tariff();
    negative["baseCredits"] = json!(-1);
    let mut inconsistent = tariff();
    inconsistent["baseCredits"] = json!(2);
    let invalid = [Value::Null, future, unknown, negative, inconsistent];
    for metadata in &invalid {
        let tool = tools(metadata.clone()).remove(0);
        assert!(
            tool.description
                .contains("Per-call USDC estimate unavailable")
        );
        assert!(tool.description.contains("Original credit conditions."));
        assert!(tool.description.ends_with(" [conf]."));
        assert!(!tool.description.contains("Estimated cost:"));
    }
    let logs = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("credit_pricing_unrecognized"), "{logs}");

    let mut cfg = Config {
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "credit_tariff".into(),
            usdc_per_credit: "0.014".into(),
        }),
        ..Default::default()
    };
    let mut paths = serde_json::Map::new();
    for (i, metadata) in invalid.iter().enumerate() {
        paths.insert(
            format!("/bad{i}"),
            json!({"get":{"credit_tariff":metadata}}),
        );
    }
    paths.insert("/good".into(), json!({"get":{"credit_tariff":tariff()}}));
    let document = json!({"paths":paths});
    writer.0.lock().unwrap().clear();
    catalog::build_tools(&cfg, &document, "provider_a").unwrap();
    let logs = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches(" WARN ").count(), 1, "{logs}");
    assert_eq!(
        logs.matches("DEBUG x402_treazury::catalog::credit_pricing:")
            .count(),
        5,
        "{logs}"
    );
    for expected in [
        "provider=\"provider_a\"",
        "metadata_key=credit_tariff",
        "affected_operations=5",
        "\"missing_metadata\": 1",
        "\"unsupported_version\": 1",
        "\"malformed_or_unsupported_schema\": 2",
        "\"inconsistent_credit_bounds\": 1",
        "tool=provider_a_bad0",
        "method=GET",
    ] {
        assert!(logs.contains(expected), "missing {expected}: {logs}");
    }
    assert!(!logs.contains("private-tariff-value"));
    assert!(!logs.contains("newBillingRule"));

    // Name-filtered failures do not count or produce diagnostics.
    cfg.exclude_tools = vec!["provider_a_bad*".into()];
    writer.0.lock().unwrap().clear();
    let retained = catalog::build_tools(&cfg, &document, "provider_a").unwrap();
    assert_eq!(retained.len(), 1);
    assert!(retained[0].description.contains("Estimated cost:"));
    assert!(writer.0.lock().unwrap().is_empty());

    // Summaries are build-local, not globally suppressed across sources or reloads.
    cfg.exclude_tools.clear();
    catalog::build_tools(&cfg, &document, "provider_b").unwrap();
    catalog::build_tools(&cfg, &document, "provider_b").unwrap();
    let logs = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logs.matches(" WARN ").count(), 2, "{logs}");
    assert_eq!(logs.matches("affected_operations=5").count(), 2, "{logs}");
}
#[tokio::test]
async fn comprehensive_subsets_inherit_conversion_despite_source_and_endpoint_instruction_overrides()
 {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/deployments/comprehensive.toml");
    let deployment = config::read_table(&path).await.unwrap();
    let document: Value =
        serde_json::from_str(include_str!("fixtures/socialfetch_openapi.json")).unwrap();
    for (id, witnesses) in [
        (
            "social",
            &[
                "socialfetch_twitter_profiles_handle",
                "socialfetch_instagram_profiles_handle_posts",
            ][..],
        ),
        (
            "webinfo",
            &["socialfetch_web_search", "socialfetch_web_ask"][..],
        ),
    ] {
        let local = deployment["sources"][id].as_table().unwrap().clone();
        let mut cfg = config::resolve(local, &path).await.unwrap().settings;
        assert_eq!(
            cfg.credit_pricing.as_ref().unwrap().usdc_per_credit,
            "0.014"
        );
        // Social now places its tailored guidance on the endpoint; webinfo still
        // overrides provider instructions at the source. Neither controls pricing.
        let instructions = if id == "social" {
            deployment["servers"]["social"]["instructions_text"]
                .as_str()
                .unwrap()
        } else {
            cfg.instructions_text.as_deref().unwrap()
        };
        assert!(!instructions.contains("0.014"));
        let selected = catalog::build_tools(&cfg, &document, "socialfetch").unwrap();
        assert!(!selected.is_empty());
        for name in witnesses {
            assert!(
                selected.iter().any(|tool| tool.name == *name),
                "{id}: {name}"
            );
        }
        for tool in &selected {
            assert!(
                tool.description.contains("Estimated cost:"),
                "{}",
                tool.name
            );
            assert!(tool.description.ends_with(" [conf]."));
            assert!(
                !tool
                    .description
                    .contains("Per-call USDC estimate unavailable"),
                "{}",
                tool.name
            );
            let tags = document["paths"][&tool.path][tool.method.to_lowercase()]["tags"]
                .as_array()
                .unwrap();
            assert!(
                tags.iter()
                    .any(|t| cfg.tags.iter().any(|wanted| t == wanted))
            );
        }
        let name = selected[0].name.clone();
        cfg.overrides = json!({name.clone():{"description":"Operator description."}});
        let overridden = catalog::build_tools(&cfg, &document, "socialfetch").unwrap();
        assert_eq!(
            overridden
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .description,
            "Operator description."
        );
    }
}

#[test]
fn structured_credit_prices_remain_authored_and_support_digests() {
    let cfg = Config {
        credit_pricing: Some(CreditPricing {
            credit_cost_key: "credit_tariff".into(),
            usdc_per_credit: "0.000001".into(),
        }),
        ..Default::default()
    };
    let mut metadata = tariff();
    metadata["baseCredits"] = json!(25);
    metadata["maxCredits"] = json!(25);
    let documents = [
        json!({"paths":{"/test":{"get":{"credit_tariff":metadata}}}}),
        json!({"operations":[{"path":"/test","method":"get","credit_tariff":metadata}]}),
    ];
    for document in documents {
        let tools = catalog::build_tools(&cfg, &document, "api").unwrap();
        assert!(tools[0].description.contains("0.000025 USDC/request"));
        assert!(!tools[0].description.contains("Cost: unknown"));
    }
}

fn expect_estimate(metadata: Value, estimate: &str) {
    assert_eq!(
        tools(metadata).remove(0).description,
        format!(
            "Original description. Pricing: \"Original credit conditions.\" [spec]. Estimated cost: {estimate} [conf]."
        )
    );
}

#[test]
fn tiktok_transcript_billable_domain_error_preserves_all_charges() {
    // Faithful pricing metadata from SocialFetch GET /v1/tiktok/videos/transcript
    // (2026-10-22): this new field previously rejected the whole tariff.
    expect_estimate(
        json!({"version":1,"baseCredits":1,"maxCredits":601,
        "normalizationFailureCredits":0,"settlement":"actual_credits",
        "x402ExactPolicy":"always","surcharges":[
            {"queryParam":"useAiFallback","credits":10,"when":"boolean_true","label":"+10 credits when AI transcribes the audio (useAiFallback)"},
            {"queryParam":"allowLongTranscription","credits":590,"when":"boolean_true","label":"+10 credits per started minute of AI transcript, replacing the flat +10 (allowLongTranscription)","badge":"+10 / started minute"}
        ],"billableDomainErrors":[
            {"code":"transcript_target_not_video","httpStatus":400,"credits":1}
        ]}),
        "0.014 USDC base; maximum 8.414 USDC/request; up to 0.14 USDC extra when useAiFallback is true (+10 credits when AI transcribes the audio (useAiFallback)); up to 8.26 USDC extra when allowLongTranscription is true (+10 credits per started minute of AI transcript, replacing the flat +10 (allowLongTranscription)); 0.014 USDC for transcript_target_not_video (HTTP 400)",
    );
}

#[test]
fn billable_domain_error_charges_preserve_order_zero_and_maximum() {
    let mut p = tariff();
    p["billableDomainErrors"] = json!([]);
    assert_eq!(
        tools(p.clone())[0].description,
        tools(tariff())[0].description
    );
    p["normalizationFailureCredits"] = json!(1);
    p["billableDomainErrors"] = json!([
        {"code":"free_error","httpStatus":400,"credits":0},
        {"code":"charged_error","httpStatus":422,"credits":1}
    ]);
    expect_estimate(
        p.clone(),
        "0.014 USDC/request; normalization-failure charge 0.014 USDC; 0 USDC for free_error (HTTP 400); 0.014 USDC for charged_error (HTTP 422)",
    );
    p["baseCredits"] = json!(u64::MAX);
    p["maxCredits"] = json!(u64::MAX);
    p["normalizationFailureCredits"] = json!(0);
    p["billableDomainErrors"] = json!([
        {"code":"maximum","httpStatus":400,"credits":u64::MAX}
    ]);
    expect_estimate(
        p,
        "258254417031933722.61 USDC/request; 258254417031933722.61 USDC for maximum (HTTP 400)",
    );
}

#[test]
fn malformed_billable_domain_errors_keep_original_terms_and_unknown_estimate() {
    let error = json!({"code":"charged_error","httpStatus":400,"credits":1});
    let mut invalid = vec![Value::Null, json!({}), json!([null]), json!([{}])];
    for field in ["code", "httpStatus", "credits"] {
        let mut entry = error.clone();
        entry.as_object_mut().unwrap().remove(field);
        invalid.push(json!([entry]));
    }
    for (field, value) in [
        ("code", json!(400)),
        ("httpStatus", json!("400")),
        ("httpStatus", json!(-1)),
        ("credits", json!(-1)),
        ("credits", json!(1.5)),
        ("credits", json!(2)), // Exceeds maxCredits.
        ("unknown", json!(true)),
    ] {
        let mut entry = error.clone();
        entry[field] = value;
        invalid.push(json!([entry]));
    }
    // A later invalid entry must not expose a partial estimate.
    invalid.push(json!([error, {"code":"missing_charge","httpStatus":400}]));
    for errors in invalid {
        let mut p = tariff();
        p["billableDomainErrors"] = errors;
        assert_eq!(
            tools(p.clone()).remove(0).description,
            "Original description. Pricing: \"Original credit conditions.\" [spec]. Per-call USDC estimate unavailable [conf].",
            "{p}"
        );
    }
}

#[test]
fn exact_policy_hints_and_instagram_include_surcharges_preserve_estimates() {
    for policy in ["always", "fixed_price_only"] {
        let mut p = tariff();
        p["x402ExactPolicy"] = json!(policy);
        assert_eq!(tools(p)[0].description, tools(tariff())[0].description);
    }
    // Live Instagram v2 profile and media tariffs (2026-10-19).
    expect_estimate(
        json!({"version":1,"baseCredits":1,"maxCredits":3,
        "normalizationFailureCredits":0,"settlement":"actual_credits",
        "x402ExactPolicy":"fixed_price_only","surcharges":[
            {"queryParam":"include","when":"includes","value":"recentMedia","credits":1,"label":"+1 credit for recentMedia"},
            {"queryParam":"include","when":"includes","value":"about","credits":1,"label":"+1 credit for about"}
        ]}),
        "0.014 USDC base; maximum 0.042 USDC/request; up to 0.014 USDC extra when include includes recentMedia (+1 credit for recentMedia); up to 0.014 USDC extra when include includes about (+1 credit for about)",
    );
    expect_estimate(
        json!({"version":1,"baseCredits":1,"maxCredits":61,
        "normalizationFailureCredits":0,"settlement":"actual_credits",
        "x402ExactPolicy":"fixed_price_only","surcharges":[
            {"queryParam":"include","when":"includes","value":"files","credits":60,"label":"+10 credits per 16 MB of files, max 60","badge":"+10 / 16 MB"}
        ]}),
        "0.014 USDC base; maximum 0.854 USDC/request; up to 0.84 USDC extra when include includes files (+10 credits per 16 MB of files, max 60)",
    );
}

#[test]
fn complete_tariff_descriptions_preserve_modes_and_ordered_suffixes() {
    expect_estimate(tariff(), "0.014 USDC/request");
    let mut bounded = tariff();
    bounded["maxCredits"] = json!(4);
    bounded["normalizationFailureCredits"] = json!(2);
    bounded["settlement"] = json!("actual_credits");
    bounded["surcharges"] = json!([
        {"credits":3,"queryParam":"media","when":"boolean_true","label":"three-credit cap","badge":"optional"},
        {"credits":1,"queryParam":"details","when":"present","label":"one-credit cap"}
    ]);
    expect_estimate(
        bounded,
        "0.014 USDC base; maximum 0.056 USDC/request; up to 0.042 USDC extra when media is true (three-credit cap); up to 0.014 USDC extra when details is supplied (one-credit cap); normalization-failure charge 0.028 USDC",
    );
    let mut metered = tariff();
    metered["baseCredits"] = json!(2);
    metered["maxCredits"] = json!(8);
    metered["normalizationFailureCredits"] = json!(1);
    metered["metered"] = json!({"chargedUnits":"returned_records","creditsPerUnit":2,"defaultUnits":3,"maxUnits":3,"queryParam":"limit","unit":"record"});
    expect_estimate(
        metered,
        "0.028 USDC base + 0.028 USDC per returned record (limit defaults to 3, maximum 3 records); maximum 0.112 USDC/request; normalization-failure charge 0.014 USDC",
    );
    let mut batch = tariff();
    batch["baseCredits"] = json!(3);
    batch["maxCredits"] = json!(150);
    batch["batch"] = json!({"unit":"url","maxUnits":50});
    expect_estimate(
        batch,
        "up to 0.042 USDC/URL; maximum 50 URLs and 2.1 USDC/request",
    );
}

#[test]
fn unsupported_tariff_rules_and_arithmetic_overflow_keep_complete_unknown_description() {
    let metered = json!({"chargedUnits":"returned_records","creditsPerUnit":1,"defaultUnits":1,"maxUnits":1,"queryParam":"limit","unit":"record"});
    let batch = json!({"unit":"url","maxUnits":1});
    let surcharge = json!({"credits":1,"queryParam":"extra","when":"present","label":"cap"});
    let mut invalid = vec![Value::Null, json!([]), json!({})];
    for (key, value) in [
        ("version", json!(2)),
        ("version", json!("1")),
        ("unknown", json!(0)),
        ("baseCredits", json!(-1)),
        ("baseCredits", json!(2)),
        ("maxCredits", json!(1.5)),
        ("normalizationFailureCredits", json!(2)),
        ("settlement", json!("estimated_credits")),
        ("x402ExactPolicy", json!("unknown")),
        ("x402ExactPolicy", json!(true)),
        ("surcharges", Value::Null),
    ] {
        let mut p = tariff();
        p[key] = value;
        invalid.push(p);
    }
    for field in [
        "version",
        "baseCredits",
        "maxCredits",
        "normalizationFailureCredits",
        "surcharges",
    ] {
        let mut p = tariff();
        p.as_object_mut().unwrap().remove(field);
        invalid.push(p);
    }
    let mut both = tariff();
    both["metered"] = metered.clone();
    both["batch"] = batch.clone();
    invalid.push(both);
    for (key, value) in [
        ("chargedUnits", json!("requested_records")),
        ("unit", json!("url")),
        ("defaultUnits", json!(2)),
        ("maxUnits", json!(2)),
        ("creditsPerUnit", json!(-1)),
        ("unknown", json!(true)),
    ] {
        let mut p = tariff();
        p["baseCredits"] = json!(0);
        p["metered"] = metered.clone();
        p["metered"][key] = value;
        invalid.push(p);
    }
    for (base, units, per_unit) in [(0, u64::MAX, 2), (1, u64::MAX, 1)] {
        let mut p = tariff();
        p["baseCredits"] = json!(base);
        p["maxCredits"] = json!(u64::MAX);
        p["metered"] = metered.clone();
        p["metered"]["maxUnits"] = json!(units);
        p["metered"]["creditsPerUnit"] = json!(per_unit);
        invalid.push(p);
    }
    for (key, value) in [
        ("unit", json!("record")),
        ("maxUnits", json!(0)),
        ("maxUnits", json!(2)),
        ("unknown", json!(true)),
    ] {
        let mut p = tariff();
        p["batch"] = batch.clone();
        p["batch"][key] = value;
        invalid.push(p);
    }
    let mut overflow = tariff();
    overflow["baseCredits"] = json!(u64::MAX);
    overflow["maxCredits"] = json!(u64::MAX);
    overflow["batch"] = json!({"unit":"url","maxUnits":2});
    invalid.push(overflow);
    for (key, value) in [
        ("when", json!("false")),
        ("when", json!("includes")), // Missing the membership value.
        ("value", json!("files")),   // Conflicts with the `present` condition.
        ("value", json!(1)),
        ("credits", json!(2)),
        ("credits", json!(-1)),
        ("unknown", json!(true)),
        ("badge", json!(false)),
    ] {
        let mut p = tariff();
        p["surcharges"] = json!([surcharge.clone()]);
        p["surcharges"][0][key] = value;
        invalid.push(p);
    }
    for (mode, value, field) in [
        ("metered", metered, "queryParam"),
        ("batch", batch, "unit"),
        ("surcharges", surcharge, "label"),
    ] {
        let mut value = value;
        value.as_object_mut().unwrap().remove(field);
        let mut p = tariff();
        p[mode] = if mode == "surcharges" {
            json!([value])
        } else {
            value
        };
        invalid.push(p);
    }
    for metadata in invalid {
        assert_eq!(
            tools(metadata.clone()).remove(0).description,
            "Original description. Pricing: \"Original credit conditions.\" [spec]. Per-call USDC estimate unavailable [conf].",
            "{metadata}"
        );
    }
}

#[test]
fn zero_and_maximum_units_preserve_existing_permissive_tariffs() {
    let mut zero = tariff();
    zero["baseCredits"] = json!(0);
    zero["maxCredits"] = json!(0);
    expect_estimate(zero.clone(), "0 USDC/request");
    zero["metered"] = json!({"chargedUnits":"returned_records","creditsPerUnit":0,"defaultUnits":0,"maxUnits":0,"queryParam":"","unit":"record"});
    expect_estimate(
        zero.clone(),
        "0 USDC base + 0 USDC per returned record ( defaults to 0, maximum 0 records); maximum 0 USDC/request",
    );
    zero["metered"]["maxUnits"] = json!(u64::MAX);
    zero["metered"]["defaultUnits"] = json!(u64::MAX);
    expect_estimate(
        zero.clone(),
        "0 USDC base + 0 USDC per returned record ( defaults to 18446744073709551615, maximum 18446744073709551615 records); maximum 0 USDC/request",
    );
    zero["metered"] = Value::Null;
    zero["batch"] = json!({"unit":"url","maxUnits":u64::MAX});
    expect_estimate(
        zero,
        "up to 0 USDC/URL; maximum 18446744073709551615 URLs and 0 USDC/request",
    );
    let mut maximum = tariff();
    maximum["baseCredits"] = json!(u64::MAX);
    maximum["maxCredits"] = json!(u64::MAX);
    expect_estimate(maximum, "258254417031933722.61 USDC/request");
    let mut unusual = tariff();
    unusual["surcharges"] = json!([
        {"credits":1,"queryParam":"","when":"present","label":""},
        {"credits":1,"queryParam":"","when":"present","label":"","badge":null}
    ]);
    unusual["normalizationFailureCredits"] = json!(1);
    // Individual surcharges are capped, but their sum plus base is not validated.
    expect_estimate(
        unusual,
        "0.014 USDC base; maximum 0.014 USDC/request; up to 0.014 USDC extra when  is supplied (); up to 0.014 USDC extra when  is supplied (); normalization-failure charge 0.014 USDC",
    );
}
