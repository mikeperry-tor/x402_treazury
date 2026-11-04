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
async fn otto_removed_generative_routes_preserve_all_research_destinations() {
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
        BTreeSet::from(["business", "finance", "security", "social", "web", "world"])
    );
    for (server, tool) in [
        ("world", "otto_weather"),
        ("web", "otto_web_extract"),
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
async fn horizon_company_facts_use_reviewed_post_on_dynamic_company_endpoints() {
    let inventory = research_source(
        "horizonpulse",
        include_str!("../providers/horizonpulse-openapi.json"),
    )
    .await;
    for endpoint in ["business", "finance"] {
        let server = inventory
            .iter()
            .find(|server| server.server == endpoint)
            .unwrap();
        let tool = server
            .tools
            .iter()
            .find(|entry| entry.tool.name == "horizonpulse_sec_company_facts")
            .unwrap();
        assert!(server.discover_on_demand);
        assert_eq!(tool.tool.method, "POST");
        assert_eq!(tool.tool.path, "/api/sec/company-facts");
        assert!(
            !server
                .tools
                .iter()
                .any(|entry| entry.tool.path == "/api/screenshot")
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("examples/deployments/research_desk.toml")).unwrap(),
    )
    .unwrap();
    for endpoint in ["business", "finance"] {
        let hints = config["servers"][endpoint]["discovery_search_tool_instructions_text"]
            .as_str()
            .unwrap();
        assert!(
            hints.contains("Horizon Pulse")
                && hints.contains("SEC company facts")
                && hints.contains("XBRL")
        );
    }
}
