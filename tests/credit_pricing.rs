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
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut future = tariff();
    future["version"] = json!(2);
    let mut unknown = tariff();
    unknown["newBillingRule"] = json!(1);
    let mut negative = tariff();
    negative["baseCredits"] = json!(-1);
    let mut inconsistent = tariff();
    inconsistent["baseCredits"] = json!(2);
    for metadata in [Value::Null, future, unknown, negative, inconsistent] {
        let tool = tools(metadata).remove(0);
        assert!(
            tool.description
                .contains("Per-call USDC estimate unavailable")
        );
        assert!(tool.description.contains("Original credit conditions."));
        assert!(tool.description.contains("configured 0.014 USDC/credit"));
        assert!(!tool.description.contains("Estimated cost:"));
    }
    let logs = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("credit_pricing_unrecognized"), "{logs}");
}
#[tokio::test]
async fn privacy_subsets_inherit_conversion_despite_instruction_overrides() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/deployments/privacy.toml");
    let deployment = config::read_table(&path).await.unwrap();
    let document: Value =
        serde_json::from_str(include_str!("fixtures/socialfetch_openapi.json")).unwrap();
    for id in ["social", "webinfo"] {
        let local = deployment["sources"][id].as_table().unwrap().clone();
        let mut cfg = config::resolve(local, &path).await.unwrap().settings;
        assert_eq!(
            cfg.credit_pricing.as_ref().unwrap().usdc_per_credit,
            "0.014"
        );
        assert!(!cfg.instructions_text.as_ref().unwrap().contains("0.014"));
        let selected = catalog::build_tools(&cfg, &document, "socialfetch").unwrap();
        assert!(!selected.is_empty() && selected.len() < 240);
        for tool in &selected {
            assert!(
                tool.description.contains("Estimated cost:"),
                "{}",
                tool.name
            );
            assert!(
                tool.description
                    .contains("payment challenge is authoritative")
            );
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

#[tokio::test]
async fn structured_credit_prices_skip_probes_and_support_digests() {
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
        let discovered = x402_treazury::pricing::PricingCache::default()
            .discover_observed(&cfg, &document, &tools, "http://127.0.0.1:1")
            .await
            .unwrap();
        assert!(discovered.prices.is_empty());
        assert_eq!(discovered.evidence.skipped_embedded, 1);
        assert_eq!(discovered.evidence.eligible, 0);
    }
}
