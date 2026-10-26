//! Bundled compatibility choices, independently of vendor catalog inventory size.
use serde_json::json;
use std::path::Path;
use x402_treazury::{catalog, config};

#[tokio::test]
async fn ordinary_providers_admit_new_reads_inside_and_outside_naming_anchors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for provider in [
        "agentfund",
        "arkham",
        "botsmith",
        "brazilayer",
        "datakoot",
        "deepline",
        "genuinegood",
        "google-trends",
        "ipintel",
        "kronos",
        "lonestar",
        "pdl",
        "x402stock",
    ] {
        let cfg = config::load(&root.join(format!("providers/{provider}.toml")))
            .await
            .unwrap()
            .settings;
        let doc = json!({"paths":{
            "/public-status":{"get":{"summary":"New public diagnostic"}},
            "/api/v1/future-research":{"post":{"requestBody":{"content":{
                "application/json":{"schema":{"type":"object","properties":{
                    "query":{"type":"string"}},"required":["query"]}}
            }}}}
        }});
        let tools = catalog::build_tools(&cfg, &doc, cfg.prefix.as_deref().unwrap())
            .unwrap_or_else(|e| panic!("{provider}: {e:#}"));
        assert!(
            tools.iter().any(|t| t.path == "/public-status"),
            "{provider}"
        );
        let tool = tools
            .iter()
            .find(|t| t.path == "/api/v1/future-research")
            .unwrap_or_else(|| panic!("{provider}"));
        let args = json!({"query":"public records"});
        let route = tool
            .route("https://example.com", args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.method, "POST");
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
    }
}

#[tokio::test]
async fn namespace_and_identity_restrictions_leave_public_research_available() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (provider, doc, present, absent) in [
        (
            "agosint",
            json!({"paths":{
                "/x402/new-registry":{"post":{}}, "/mpp/company":{"post":{}},
            "/endpoints":{"get":{}}, "/mppish":{"get":{}}
            }}),
            "/x402/new-registry",
            vec!["/mpp/company"],
        ),
        (
            "arkham",
            json!({"paths":{
                "/new-intelligence":{"post":{}}, "/chains":{"get":{}},
                "/networks/status":{"get":{}}, "/arkm/circulating":{"get":{}}
            }}),
            "/new-intelligence",
            vec!["/chains", "/networks/status", "/arkm/circulating"],
        ),
    ] {
        let cfg = config::load(&root.join(format!("providers/{provider}.toml")))
            .await
            .unwrap()
            .settings;
        let tools = catalog::build_tools(&cfg, &doc, cfg.prefix.as_deref().unwrap()).unwrap();
        assert!(tools.iter().any(|t| t.path == present), "{provider}");
        if provider == "agosint" {
            assert!(tools.iter().any(|t| t.name == "agosint_new_registry"));
            assert!(tools.iter().any(|t| t.path == "/endpoints"));
            assert!(tools.iter().any(|t| t.path == "/mppish"));
        }
        assert!(
            !tools.iter().any(|t| absent.contains(&t.path.as_str())),
            "{provider}"
        );
    }
}
