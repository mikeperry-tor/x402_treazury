//! Named provider-contract drift must not invalidate unrelated endpoint tools.
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};
use x402_treazury::deployment::{Deployment, Inventory};

async fn research_source(source: &str, schema: &str) -> Vec<Inventory> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/deployments/research_desk.toml");
    let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut settings = config["sources"][source].clone();
    let provider = settings["extends"].as_str().unwrap();
    settings["extends"] = root
        .join("examples/deployments")
        .join(provider)
        .to_string_lossy()
        .into_owned()
        .into();
    let mut destinations = BTreeSet::new();
    for rule in settings["send_tools"].as_array().unwrap() {
        match &rule["to"] {
            toml::Value::String(name) if name != "exclude" => {
                destinations.insert(name.clone());
            }
            toml::Value::Array(names) => {
                destinations.extend(names.iter().map(|v| v.as_str().unwrap().to_owned()))
            }
            _ => {}
        }
    }
    config["servers"]
        .as_table_mut()
        .unwrap()
        .retain(|name, _| destinations.contains(name));
    config["sources"].as_table_mut().unwrap().clear();
    config["network"] = toml::from_str::<toml::Value>("mode='direct'").unwrap();
    config.as_table_mut().unwrap().remove("relay_fallback");
    let temp = tempfile::tempdir().unwrap();
    let spec = temp.path().join("source.json");
    std::fs::write(&spec, schema).unwrap();
    settings
        .as_table_mut()
        .unwrap()
        .insert("spec".into(), spec.to_string_lossy().into_owned().into());
    config["sources"]
        .as_table_mut()
        .unwrap()
        .insert(source.to_owned(), settings);
    let config_path = temp.path().join("research.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    // Inspection builds catalogs/routing only: no socket binding, wallet secrets,
    // funding, API calls or private state opens.
    Deployment::load(&config_path).await.unwrap().inventory()
}

#[tokio::test]
async fn otto_removed_generative_routes_preserve_remaining_research_destinations() {
    let doc = include_str!("fixtures/otto_routing_openapi.json");
    let parsed: Value = serde_json::from_str(doc).unwrap();
    assert!(parsed["paths"].get("/generate-meme").is_none());
    assert!(parsed["paths"].get("/video-gen").is_none());
    let inventory = research_source("otto", doc).await;
    let names: BTreeSet<_> = inventory
        .iter()
        .map(|server| server.server.as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["business", "finance", "security", "social", "world"])
    );
    for (server, tool) in [
        ("world", "otto_weather"),
        ("finance", "otto_funding_rates"),
        ("security", "otto_dns_lookup"),
    ] {
        assert!(
            inventory
                .iter()
                .find(|entry| entry.server == server)
                .unwrap()
                .tools
                .iter()
                .any(|entry| entry.tool.name == tool),
            "{server}: {tool}"
        );
    }
}

#[tokio::test]
async fn agproxy_selects_only_raw_fetch_on_web() {
    let inventory =
        research_source("agproxy", include_str!("../providers/agproxy-openapi.json")).await;
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].server, "web");
    let names: BTreeSet<_> = inventory[0]
        .tools
        .iter()
        .map(|entry| entry.tool.name.as_str())
        .collect();
    assert_eq!(names, BTreeSet::from(["agproxy_fetch"]));
}

#[tokio::test]
async fn repaired_csv_weather_and_post_utilities_preserve_research_desk_distribution() {
    let twos = research_source(
        "twos",
        x402_treazury::providers::get("2s")
            .unwrap()
            .local_schema
            .unwrap(),
    )
    .await;
    let world = twos.iter().find(|server| server.server == "world").unwrap();
    let history = world
        .tools
        .iter()
        .find(|entry| entry.tool.name == "twos_climate_station_history")
        .unwrap();
    assert_eq!(history.tool.method, "GET");
    assert_eq!(
        history.tool.input_schema["properties"]["dataTypes"]["type"],
        "string"
    );
    for server in &twos {
        assert!(server.tools.iter().all(|entry| !matches!(
            entry.tool.path.as_str(),
            "/api/crypto/token-price" | "/api/crypto/kimchi-premium" | "/api/gov/osha-violations"
        )));
    }
    let strale = research_source(
        "strale",
        x402_treazury::providers::get("strale")
            .unwrap()
            .local_schema
            .unwrap(),
    )
    .await;
    assert!(strale.iter().any(|server| {
        server.server == "business"
            && server
                .tools
                .iter()
                .any(|entry| entry.tool.name == "strale_polish_company_data")
    }));
    // The reusable provider restores utilities; this deployment still deliberately
    // excludes them in its final rule because parsing/validation belongs locally.
    for server in &strale {
        assert!(server.tools.iter().all(|entry| !matches!(
            entry.tool.path.as_str(),
            "/x402/v2/http-to-curl"
                | "/x402/v2/json-schema-validate"
                | "/x402/v2/tool-call-validate"
                | "/x402/v2/html-to-pdf"
        )));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("examples/deployments/research_desk.toml")).unwrap(),
    )
    .unwrap();
    assert!(
        config["servers"]["world"]["discovery_search_tool_instructions_text"]
            .as_str()
            .unwrap()
            .contains("GHCN")
    );
}

#[tokio::test]
async fn authored_market_groundwater_and_history_additions_have_intended_destinations() {
    let apex = research_source(
        "apexfaucet",
        include_str!("../providers/apexfaucet-openapi.json"),
    )
    .await;
    let finance = apex
        .iter()
        .find(|server| server.server == "finance")
        .unwrap();
    assert!(finance.discover_on_demand);
    for name in [
        "apexfaucet_funding_rates",
        "apexfaucet_stablecoin_pegs",
        "apexfaucet_prediction_odds",
    ] {
        assert!(
            finance.tools.iter().any(|entry| entry.tool.name == name),
            "{name}"
        );
    }
    assert!(
        !apex
            .iter()
            .flat_map(|server| &server.tools)
            .any(|entry| entry.tool.path == "/api/x402/pay-safe-credits")
    );
    let locus = research_source("locus", include_str!("../providers/locus/openapi.json")).await;
    let property = locus
        .iter()
        .find(|server| server.server == "property")
        .unwrap();
    assert!(
        property
            .tools
            .iter()
            .any(|entry| entry.tool.name == "locus_locus_groundwater_drawdown")
    );
    assert!(
        !property
            .tools
            .iter()
            .any(|entry| entry.tool.name == "locus_locus_before_you_sign_dossier")
    );
    let straits =
        research_source("straits", include_str!("../providers/straits/openapi.json")).await;
    let world = straits
        .iter()
        .find(|server| server.server == "world")
        .unwrap();
    let history = world
        .tools
        .iter()
        .find(|entry| entry.tool.name == "straits_history_feed")
        .unwrap();
    assert!(world.discover_on_demand);
    assert_eq!(
        history.tool.input_schema["properties"]["days"]["maximum"],
        7
    );
    assert!(
        history.tool.input_schema["properties"]
            .get("before")
            .is_some()
    );
    assert!(
        !world
            .tools
            .iter()
            .any(|entry| entry.tool.name == "straits_export")
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("examples/deployments/research_desk.toml")).unwrap(),
    )
    .unwrap();
    for (endpoint, term) in [
        ("finance", "funding intervals"),
        ("property", "groundwater drawdown"),
        ("world", "tick history"),
    ] {
        assert!(
            config["servers"][endpoint]["discovery_search_tool_instructions_text"]
                .as_str()
                .unwrap()
                .contains(term)
        );
    }
}
