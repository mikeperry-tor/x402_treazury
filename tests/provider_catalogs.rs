//! Focused captured provider regressions; no live vendor access.
use serde_json::Value;
use std::path::Path;
#[test]
fn live_provider_scope_accounts_for_every_bundled_definition() {
    use std::collections::BTreeSet;
    fn definitions(root: &Path, dir: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                definitions(root, &path, found);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                found.insert(path.strip_prefix(root).unwrap().to_str().unwrap().into());
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bundled = BTreeSet::new();
    definitions(root, &root.join("providers"), &mut bundled);
    let coverage: Vec<Value> = serde_json::from_str(include_str!("live/coverage.json")).unwrap();
    let exclusions: Vec<Value> =
        serde_json::from_str(include_str!("live/coverage_exclusions.json")).unwrap();
    let preset: toml::Value =
        toml::from_str(include_str!("live/integration/providers.example.toml")).unwrap();
    let cases: BTreeSet<_> = preset["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect();
    let mut accounted = BTreeSet::new();
    let mut paid = BTreeSet::new();
    for row in coverage.iter().chain(&exclusions) {
        assert!(!row["reason"].as_str().unwrap().trim().is_empty());
        assert!(accounted.insert(row["config"].as_str().unwrap().to_owned()));
        if let Some(case) = row["case"].as_str() {
            assert!(
                cases.contains(case),
                "missing reviewed provider case {case}"
            );
            assert!(paid.insert(case));
        }
    }
    assert_eq!(
        bundled, accounted,
        "new providers require explicit scope review"
    );
    let selected: BTreeSet<_> = cases
        .into_iter()
        .filter(|id| id.starts_with("sweep_"))
        .collect();
    assert_eq!(
        paid, selected,
        "provider scope must match the paid preset exactly"
    );
    assert_eq!(paid.len(), 22);
}

#[tokio::test]
async fn directory_exact_allowlist_excludes_new_write_and_subroutes() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/x402-list.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/x402_list_openapi.json")).unwrap();
    doc["paths"]["/services"]["post"] = serde_json::json!({"description":"new write"});
    doc["paths"]["/services/new-admin-action"] =
        serde_json::json!({"get":{"description":"new route"}});
    let tools = catalog::build_tools(&cfg, &doc, "x402_list").unwrap();
    assert_eq!(tools.len(), 6);
    assert!(tools.iter().all(|t| t.method == "GET"));
    let details = tools.iter().find(|t| t.path == "/services/{slug}").unwrap();
    let route = details
        .route(
            cfg.base_url.as_ref().unwrap(),
            serde_json::json!({"slug":"vendor/test"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        route.url,
        "https://x402-list.com/api/v1/services/vendor%2Ftest"
    );
    let search = tools.iter().find(|t| t.path == "/services").unwrap();
    let route = search
        .route(
            cfg.base_url.as_ref().unwrap(),
            serde_json::json!({"network":"BSE","limit":2})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(route.query["network"], "BSE");
    assert_eq!(route.query["limit"], 2);
}

#[tokio::test]
async fn exa_curates_paid_operations_and_preserves_request_contracts() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/exa.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value = serde_json::from_str(include_str!("fixtures/exa_openapi.json")).unwrap();
    // New diagnostic methods and same-prefix subroutes must not enlarge the default catalog.
    doc["paths"]["/search"]["get"] = json!({"summary":"Diagnostic"});
    doc["paths"]["/search/debug"] = json!({"post":{"summary":"Diagnostic"}});
    doc["paths"]["/health"] = json!({"get":{"summary":"Health"}});
    let tools = catalog::build_tools(&cfg, &doc, "exa").unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["exa_contents", "exa_search", "exa_help"]
    );
    let contents = &tools[0];
    assert_eq!(
        contents.input_schema["oneOf"],
        json!([{"required":["ids"]},{"required":["urls"]}])
    );
    assert_eq!(contents.input_schema["properties"]["urls"]["minItems"], 1);
    assert!(contents.description.contains("exactly one"));
    let args = json!({"urls":["https://example.com/a?b=1"],"highlights":{"query":"context"},"maxAgeHours":0});
    let routed = contents
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(routed.url, "https://api.exa.ai/contents");
    assert_eq!(routed.method, "POST");
    assert_eq!(routed.body, Some(args));
    assert!(routed.query.is_empty());
    let search = &tools[1];
    assert_eq!(search.input_schema["required"], json!(["query"]));
    assert!(
        search.input_schema["properties"]["stream"]["description"]
            .as_str()
            .unwrap()
            .contains("false")
    );
    let args = json!({"query":"rust","numResults":3,"contents":{"highlights":true},"stream":false});
    assert_eq!(
        search
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap()
            .body,
        Some(args)
    );
    assert_eq!(
        tools[2].help_url.as_deref(),
        Some("https://exa.ai/docs/integrations/payments/x402/quickstart.md")
    );
    assert!(
        tools
            .iter()
            .all(|t| !t.input_schema.to_string().contains("$ref"))
    );
}

#[test]
fn body_presence_alternatives_follow_collision_renaming() {
    use serde_json::json;
    use x402_treazury::catalog::{self, Config};
    for keyword in ["oneOf", "anyOf", "allOf"] {
        let mut schema = json!({"type":"object","properties":{"ids":{"type":"array","items":{"type":"string"}},"urls":{"type":"array","items":{"type":"string"}}}});
        schema[keyword] = json!([{"required":["ids"]},{"required":["urls"]}]);
        let doc = json!({"openapi":"3.1.0","paths":{"/contents":{"post":{
            "parameters":[{"name":"ids","in":"query","schema":{"type":"string"}}],
            "requestBody":{"content":{"application/json":{"schema":schema}}}
        }}}});
        let tools = catalog::build_tools(&Config::default(), &doc, "test").unwrap();
        let tool = &tools[0];
        assert_eq!(
            tool.input_schema[keyword],
            json!([{"required":["ids_body"]},{"required":["urls"]}])
        );
        let args = json!({"ids":"query value","ids_body":["document"]});
        let route = tool
            .route("https://example.com", args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.query["ids"], "query value");
        assert_eq!(route.body, Some(json!({"ids":["document"]})));
    }
}

#[tokio::test]
async fn oneshot_keeps_synchronous_search_without_authenticated_job_workflows() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/oneshot.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/oneshot_openapi.json")).unwrap();
    // A new diagnostic method or nested paid route must not expand the catalog.
    doc["paths"]["/v1/tools/search"]["get"] = json!({"summary":"Search diagnostics"});
    doc["paths"]["/v1/tools/search/jobs"] = json!({"post":{"summary":"Async search"}});
    let tools = catalog::build_tools(&cfg, &doc, "oneshot").unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["oneshot_search", "oneshot_help"]
    );
    let search = &tools[0];
    assert_eq!(search.input_schema["required"], json!(["query"]));
    assert_eq!(search.input_schema["properties"]["query"]["minLength"], 1);
    assert_eq!(search.input_schema["properties"]["query"]["maxLength"], 500);
    assert_eq!(
        search.input_schema["properties"]["max_results"]["maximum"],
        20
    );
    assert_eq!(
        search.input_schema["properties"]["max_results"]["default"],
        5
    );
    assert_eq!(search.param_routes.len(), 2);
    assert!(search.param_routes.values().all(|v| v == "body"));
    let args = json!({"query":"Rust MCP integration","max_results":3});
    let routed = search
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(routed.url, "https://win.oneshotagent.com/v1/tools/search");
    assert_eq!(routed.method, "POST");
    assert_eq!(routed.body, Some(args));
    assert!(routed.query.is_empty());
    assert!(search.description.contains("$0.001"));
    assert!(!search.description.contains("securitySchemes"));
    assert_eq!(
        tools[1].help_url.as_deref(),
        Some("https://docs.oneshotagent.com/api-reference/web-search.md")
    );
}

#[tokio::test]
async fn stableenrich_keeps_sync_images_and_json_without_async_workflows() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let mut cfg =
        config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/stableenrich.toml"))
            .await
            .unwrap()
            .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/stableenrich_openapi.json")).unwrap();
    doc["paths"]["/api/health"] = json!({"get":{"summary":"Health"}});
    doc["paths"]["/api/exa/search"]["get"] = json!({"summary":"Search diagnostics"});
    doc["paths"]["/api/exa/search/jobs"] = json!({"post":{"summary":"Async search"}});
    let tools = catalog::build_tools(&cfg, &doc, "stableenrich").unwrap();
    assert_eq!(tools.len(), 33);
    assert!(tools.iter().all(|t| !t.path.contains("/hunter/")
        && !t.path.contains("/cloudflare/")
        && !t.path.contains("/aerial-view/")
        && !t.path.contains("/jobs")));
    let details = tools
        .iter()
        .find(|t| t.name == "stableenrich_google_maps_place_details_partial")
        .unwrap();
    let routed = details
        .route(
            cfg.base_url.as_ref().unwrap(),
            json!({"placeId":"place/with space"}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(routed.method, "GET");
    assert_eq!(
        routed.url,
        "https://stableenrich.dev/api/google-maps/place-details/partial"
    );
    assert_eq!(routed.query["placeId"], "place/with space");
    assert!(routed.body.is_none());
    let search = tools
        .iter()
        .find(|t| t.name == "stableenrich_google_maps_text_search_partial")
        .unwrap();
    let args = json!({"textQuery":"cafes","maxResultCount":3,"locationBias":{"circle":{"center":{"latitude":37.4,"longitude":-122.1},"radius":500.0}}});
    assert_eq!(
        search
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap()
            .body,
        Some(args)
    );
    assert!(search.description.contains("$0.02"));
    let scrape = tools
        .iter()
        .find(|t| t.name == "stableenrich_firecrawl_scrape")
        .unwrap();
    assert!(scrape.description.contains("$0.0126"));
    let exa = tools
        .iter()
        .find(|t| t.name == "stableenrich_exa_search")
        .unwrap();
    assert!(
        exa.input_schema["properties"]["stream"]["description"]
            .as_str()
            .unwrap()
            .contains("false")
    );
    assert_eq!(
        tools.last().unwrap().help_url.as_deref(),
        Some("https://stableenrich.dev/llms.txt")
    );
    cfg.tags = vec!["Google Maps".into()];
    let selected = catalog::build_tools(&cfg, &doc, "stableenrich").unwrap();
    // Source tag selection keeps the generated help tool; listener positive tags may remove it.
    assert_eq!(selected.len(), 10);
    assert!(
        selected
            .iter()
            .all(|t| t.help_url.is_some() || t.path.starts_with("/api/google-maps/"))
    );
}

#[tokio::test]
async fn agent402_defaults_to_web_and_allows_reviewed_tag_subsets() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let mut cfg =
        config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/agent402.toml"))
            .await
            .unwrap()
            .settings;
    assert!(!cfg.probe_pricing);
    assert_eq!(cfg.tags, ["web"]);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/agent402_openapi.json")).unwrap();
    assert_eq!(catalog::tag_counts(&doc).unwrap().len(), 24);
    doc["paths"]["/api/health"] = json!({"get":{"tags":["web"]}});
    doc["paths"]["/api/search"]["post"] = json!({"tags":["web"]});
    doc["paths"]["/api/search/debug"] = json!({"get":{"tags":["web"]}});
    let tools = catalog::build_tools(&cfg, &doc, "agent402").unwrap();
    assert_eq!(tools.len(), 49);
    assert!(!tools.iter().any(|t| t.path.contains("image-resize")
        || t.path.contains("pdf-merge")
        || t.path.contains("/debug")
        || t.path.contains("health")));
    let search = tools.iter().find(|t| t.name == "agent402_search").unwrap();
    let routed = search
        .route(
            cfg.base_url.as_ref().unwrap(),
            json!({"q":"Rust & MCP","count":3}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(routed.url, "https://agent402.tools/api/search");
    assert_eq!(routed.method, "GET");
    assert_eq!(routed.query["q"], "Rust & MCP");
    assert_eq!(routed.query["count"], 3);
    assert!(routed.body.is_none());
    assert!(!search.param_routes.contains_key("Idempotency-Key"));
    assert!(search.description.contains("$0.01"));
    assert_eq!(
        tools.last().unwrap().help_url.as_deref(),
        Some("https://agent402.tools/llms.txt")
    );
    cfg.tags = vec!["data".into(), "crypto".into()];
    assert_eq!(
        catalog::build_tools(&cfg, &doc, "agent402").unwrap().len(),
        215
    );
    cfg.tags = vec!["llm".into()];
    let llm = catalog::build_tools(&cfg, &doc, "agent402").unwrap();
    assert_eq!(llm.len(), 48);
    let chat = llm
        .iter()
        .find(|t| t.name == "agent402_chat_completions")
        .unwrap();
    let args = json!({"messages":[{"role":"user","content":"Hello"}],"max_tokens":32});
    let routed = chat
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(routed.url, "https://agent402.tools/v1/chat/completions");
    assert_eq!(routed.body, Some(args));
    let messages = llm.iter().find(|t| t.name == "agent402_messages").unwrap();
    assert!(
        messages.input_schema["properties"]["stream"]["description"]
            .as_str()
            .unwrap()
            .contains("false")
    );
    cfg.tags.clear();
    let all = catalog::build_tools(&cfg, &doc, "agent402").unwrap();
    assert_eq!(all.len(), 481);
    assert!(!all.iter().any(|t| t.path.starts_with("/api/memory")
        || t.path == "/api/my-usage"
        || t.path.starts_with("/api/route/")
        || t.path.starts_with("/api/skill/")
        || t.path == "/v1/audio/speech"));
    assert!(
        all.iter()
            .find(|t| t.name == "agent402_images_generations")
            .unwrap()
            .response_mapping
            .is_some()
    );
    assert!(
        all.iter()
            .find(|t| t.name == "agent402_image_crop")
            .unwrap()
            .response_mapping
            .is_some()
    );
    cfg.tags = vec!["memory".into()];
    assert!(
        catalog::build_tools(&cfg, &doc, "agent402")
            .unwrap_err()
            .to_string()
            .contains("no operations matched")
    );
}

#[tokio::test]
async fn agentutility_restores_tags_deduplicates_and_keeps_research_focused() {
    use x402_treazury::{catalog, config, output::HttpOutput};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cfg = config::load(&root.join("providers/agentutility/provider.toml"))
        .await
        .unwrap()
        .settings;
    let mut doc: Value =
        serde_json::from_str(include_str!("../providers/agentutility/openapi.json")).unwrap();
    assert_eq!(doc["paths"].as_object().unwrap().len(), 817);
    assert_eq!(
        doc["paths"]
            .as_object()
            .unwrap()
            .values()
            .filter(|o| o["post"].get("x-agentutility-alias-of").is_some())
            .count(),
        282
    );
    assert_eq!(cfg.tags, ["treazury-research"]);
    assert!(!cfg.probe_pricing);
    doc["paths"]["/health"] = serde_json::json!({"post":{"tags":["treazury-research"]}});
    doc["paths"]["/web-search"]["get"] = serde_json::json!({"tags":["treazury-research"]});
    let tools = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    assert_eq!(tools.len(), 16);
    assert!(
        !tools
            .iter()
            .any(|t| t.path == "/health" || t.path == "/dns-lookup")
    );
    let search = tools
        .iter()
        .find(|t| t.name == "agentutility_web_search")
        .unwrap();
    let args = serde_json::json!({"query":"Rust MCP","num_results":2,"recency":"week"});
    let route = search
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://x402.agentutility.ai/web-search");
    assert_eq!(route.method, "POST");
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    assert!(search.description.contains("$0.006"));
    assert_eq!(
        search.input_schema["required"],
        serde_json::json!(["query"])
    );
    assert_eq!(
        tools.last().unwrap().help_url.as_deref(),
        Some("https://agentutility.ai/llms.txt")
    );
    cfg.tags.clear();
    let all = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    assert_eq!(all.len(), 526);
    for tool in all.iter().filter(|t| t.help_url.is_none()) {
        assert!(
            doc["paths"][&tool.path]["post"]
                .get("x-agentutility-alias-of")
                .is_none()
        );
    }
    for excluded in [
        "/watch-page",
        "/browser-session",
        "/ipfs-fetch",
        "/qr-code-generate",
        "/db-migration-risk",
    ] {
        assert!(!all.iter().any(|t| t.path == excluded));
    }
    let keywords = all.iter().find(|t| t.path == "/keyword-suggest").unwrap();
    assert_eq!(
        keywords.input_schema["anyOf"],
        serde_json::json!([{"required":["query"]},{"required":["keyword"]}])
    );
    cfg.tags = vec!["synthforge".into()];
    let images = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    assert_eq!(images.len(), 14); // Hosted URL results require no automatic asset download.
    assert!(
        images
            .iter()
            .any(|t| t.path == "/image-generate" && t.response_mapping.is_none())
    );
    let satellite = all.iter().find(|t| t.path == "/satellite-change").unwrap();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP8sAAAAASUVORK5CYII=";
    let body = serde_json::json!({"before":{"data_base64":png,"anchor_date":"2025-05-15"},"after":{"data_base64":png,"anchor_date":"2026-05-15"},"content_type":"image/png","attribution":"Copernicus fixture","license":"CC BY-SA 3.0 IGO"});
    let output = HttpOutput {
        advisories: vec![],
        bytes: serde_json::to_vec(&body).unwrap(),
        mime_type: Some("application/json".into()),
        paid_submission: false,
    }
    .render(satellite.response_mapping.as_ref(), &satellite.image_limits)
    .unwrap();
    assert_eq!(output.images.len(), 2);
    assert!(output.text.contains("Copernicus fixture"));
    assert!(output.text.contains("2025-05-15"));
    assert!(!output.text.contains(png));
}

#[tokio::test]
async fn added_help_tools_build_offline_without_changing_api_routes() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for provider in [
        "botsmith",
        "brazilayer",
        "kronos",
        "otto",
        "regimeshift",
        "socialfetch",
        "x402-list",
    ] {
        let mut cfg = config::load(&root.join(format!("providers/{provider}.toml")))
            .await
            .unwrap()
            .settings;
        // Inventory generation never loads the remote help document. Keep a tiny
        // local operation independent of each provider's real selection filters.
        cfg.include.clear();
        cfg.exclude.clear();
        cfg.include_operations.clear();
        cfg.tags.clear();
        cfg.exclude_tags.clear();
        let doc = serde_json::json!({"paths":{"/fixture":{"get":{"summary":"Fixture"}}}});
        let tools = catalog::build_tools(&cfg, &doc, cfg.prefix.as_deref().unwrap()).unwrap();
        assert_eq!(tools.len(), 2, "{provider}");
        assert_eq!(tools[0].path, "/fixture");
        let help = &tools[1];
        assert_eq!(help.help_url, cfg.help_url);
        assert!(
            help.input_schema["properties"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(help.param_routes.is_empty());
        assert!(!help.has_body);
        assert!(help.description.contains("cached for this process"));
    }
}

#[tokio::test]
async fn new_market_providers_keep_curated_routes_and_body_query_contracts() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (name, count) in [("blockrun", 7), ("claw402", 13)] {
        let cfg = config::load(&root.join(format!("providers/{name}.toml")))
            .await
            .unwrap()
            .settings;
        assert!(!cfg.probe_pricing);
        assert!(!cfg.allow_http1 && !cfg.allow_tls12);
        let mut doc: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("tests/fixtures/{name}_openapi.json"))).unwrap(),
        )
        .unwrap();
        doc["paths"]["/health"] = json!({"get":{"summary":"New diagnostic"}});
        doc["paths"]["/api/v1/new-paid-job"] = json!({"post":{"summary":"New async job"}});
        let tools = catalog::build_tools(&cfg, &doc, name).unwrap();
        assert_eq!(tools.len(), count);
        let help = tools
            .iter()
            .find(|t| t.name == format!("{name}_help"))
            .unwrap();
        assert_eq!(help.help_url, cfg.help_url);
        if name == "blockrun" {
            let t = tools
                .iter()
                .find(|t| t.name == "blockrun_exa_search")
                .unwrap();
            let args = json!({"query":"API documentation","numResults":1});
            let routed = t
                .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
                .unwrap();
            assert_eq!(routed.url, "https://blockrun.ai/api/v1/exa/search");
            assert_eq!(routed.body, Some(args));
            assert!(routed.query.is_empty());
            let chat = tools
                .iter()
                .find(|t| t.name == "blockrun_chat_completions")
                .unwrap();
            assert!(
                chat.input_schema["properties"]["stream"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("false")
            );
            assert!(
                tools
                    .iter()
                    .all(|t| !t.path.contains("images") && !t.path.contains("sandbox"))
            );
        } else {
            let t = tools
                .iter()
                .find(|t| t.name == "claw402_nofx_price_ranking")
                .unwrap();
            let args = json!({"duration":"1h"});
            let routed = t
                .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
                .unwrap();
            assert_eq!(routed.url, "https://claw402.ai/api/v1/nofx/price/ranking");
            assert_eq!(routed.query["duration"], "1h");
            assert!(routed.body.is_none());
            let mut selected = cfg.clone();
            selected.tags = vec!["Price".into()];
            let filtered = catalog::build_tools(&selected, &doc, name).unwrap();
            assert_eq!(filtered.len(), 2); // One tagged API operation plus lazy help.
            assert_eq!(filtered[0].name, "claw402_nofx_price_ranking");
        }
    }
}

#[tokio::test]
async fn relayshield_catalog_and_phone_companion_preserve_contracts() {
    use std::collections::BTreeSet;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/relayshield.toml"))
        .await
        .unwrap()
        .settings;
    assert_eq!(cfg.prefix.as_deref(), Some("relayshield"));
    assert_eq!(cfg.pricing_key.as_deref(), Some("x-payment-info"));
    assert!(!cfg.probe_pricing);
    assert_eq!(cfg.read_timeout_seconds, Some(90.0));
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/relayshield_openapi.json")).unwrap();
    // The real omitted inventory and prospective routes stay excluded.
    assert!(doc["paths"]["/v1/payg/scan-file"].is_object());
    assert!(doc["paths"]["/v1/metered/breach"].is_object());
    assert!(doc["paths"]["/v1/payg/phone-reputation"].is_null());
    doc["paths"]["/v1/payg/new-admin-action"] =
        serde_json::json!({"post":{"description":"new write"}});
    let tools = catalog::build_tools(&cfg, &doc, "relayshield").unwrap();
    let expected: BTreeSet<_> = [
        "breach",
        "bulk_identity_risk",
        "campaign_scan",
        "cert_expiry",
        "domain",
        "identity_graph",
        "identity_risk_score",
        "infostealer",
        "ip_intel",
        "llm_credential_exposure",
        "mcp_registry_risk",
        "nft_security",
        "nhi_exposure",
        "oauth_watchlist",
        "prompt_injection_breach",
        "ransomware_risk",
        "scamkit_fingerprint",
        "scamkit_match",
        "scan_url",
        "scan_wallet",
        "secret_scan",
        "secret_scan_text",
        "session_risk",
        "sim_swap",
        "supply_chain",
        "target_risk",
        "tech_stack_cve",
        "token_security",
        "wallet_risk",
        "wallet_screen_batch",
        "help",
    ]
    .into_iter()
    .map(|name| format!("relayshield_{name}"))
    .collect();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(tools.len(), 31);
    for tool in tools.iter().filter(|tool| tool.help_url.is_none()) {
        assert_eq!(tool.method, "POST");
        assert!(tool.path.starts_with("/v1/payg/"));
        assert!(tool.has_body);
        let price = doc["paths"][&tool.path]["post"]["x-payment-info"]["price"]["amount"]
            .as_str()
            .unwrap();
        assert!(
            tool.description.contains(&format!("${price}")),
            "{} price",
            tool.name
        );
    }
    let scan = tools
        .iter()
        .find(|tool| tool.name == "relayshield_scan_url")
        .unwrap();
    let args = serde_json::json!({"url":"https://example.com/"});
    let route = scan
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://api.relayshield.net/v1/payg/scan-url");
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    assert!(scan.description.contains("cannot be polled"));

    let companion = config::load(&root.join("providers/relayshield/phone-reputation.toml"))
        .await
        .unwrap()
        .settings;
    assert_eq!(companion.prefix, cfg.prefix);
    assert_eq!(companion.base_url, cfg.base_url);
    assert_eq!(companion.pricing_key, cfg.pricing_key);
    assert_eq!(companion.read_timeout_seconds, cfg.read_timeout_seconds);
    assert!(!companion.probe_pricing);
    assert!(companion.help_url.is_none());
    // Read the resolved relative local path, with no catalog HTTP request.
    let phone_doc: Value =
        serde_json::from_slice(&std::fs::read(&companion.spec).unwrap()).unwrap();
    let phone_tools = catalog::build_tools(&companion, &phone_doc, "relayshield").unwrap();
    assert_eq!(phone_tools.len(), 1);
    let phone = &phone_tools[0];
    assert_eq!(phone.name, "relayshield_phone_reputation");
    assert_eq!(phone.input_schema["required"], serde_json::json!(["phone"]));
    assert_eq!(phone.input_schema["properties"]["phone"]["type"], "string");
    // The generic converter rebuilds root tool schemas; closure belongs to the
    // authored request schema rather than the generated MCP schema.
    assert_eq!(
        phone_doc["paths"]["/v1/payg/phone-reputation"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["additionalProperties"],
        false
    );
    assert!(phone.description.contains("$0.05"));
    let args = serde_json::json!({"phone":"+14155552671"});
    let route = phone
        .route(
            companion.base_url.as_ref().unwrap(),
            args.as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(route.method, "POST");
    assert_eq!(
        route.url,
        "https://api.relayshield.net/v1/payg/phone-reputation"
    );
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    let names: BTreeSet<_> = tools
        .iter()
        .chain(&phone_tools)
        .map(|tool| &tool.name)
        .collect();
    assert_eq!(names.len(), tools.len() + phone_tools.len());
}

#[tokio::test]
async fn ipintel_preserves_paid_post_contracts_and_arbitrary_json() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/ipintel.toml"))
        .await
        .unwrap()
        .settings;
    let doc: Value = serde_json::from_str(include_str!("fixtures/ipintel_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "ipintel").unwrap();
    let find = |name: &str| tools.iter().find(|t| t.name == name).unwrap();
    let dns = find("ipintel_dns_lookup");
    assert_eq!(dns.input_schema["required"], json!(["domain"]));
    assert_eq!(
        dns.input_schema["properties"]["types"]["items"]["enum"],
        json!(["A", "AAAA", "MX", "NS", "TXT", "CAA", "SOA"])
    );
    let args = json!({"domain":"example.com","types":["MX","TXT"]});
    let route = dns
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://tools.ipintel.ai/dns-lookup");
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    let satellite = find("ipintel_satellite");
    assert_eq!(
        satellite.input_schema["properties"]["date"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(
        satellite.input_schema["properties"]["fallback"]["default"],
        true
    );
    assert!(satellite.response_mapping.is_none());
    let validator = find("ipintel_json_schema_validator");
    assert!(
        validator.input_schema["properties"]["data"]
            .get("type")
            .is_none()
    );
    for data in [json!(null), json!([1, "two"]), json!(false)] {
        let args = json!({"schema":{},"data":data});
        let route = validator
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.body, Some(args));
    }
    assert_eq!(
        find("ipintel_http_latency").input_schema["properties"]["method"]["enum"],
        json!(["HEAD", "GET"])
    );
    assert!(
        find("ipintel_jwt_decode")
            .description
            .contains("without verifying")
    );
    assert_eq!(
        find("ipintel_help").help_url.as_deref(),
        Some("https://tools.ipintel.ai/llms.txt")
    );
}

#[tokio::test]
async fn newsgurus_uses_catalog_templates_and_preserves_parameter_contracts() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/newsgurus/provider.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let raw: Value =
        serde_json::from_str(include_str!("fixtures/newsgurus_x402_catalog.json")).unwrap();
    let mut doc: Value =
        serde_json::from_str(include_str!("../providers/newsgurus/openapi.json")).unwrap();
    assert_eq!(doc["paths"].as_object().unwrap().len(), 49);
    for resource in raw["resources"].as_array().unwrap() {
        let path = resource["resource"].as_str().unwrap();
        let op = &doc["paths"][path]["get"];
        assert_eq!(op["description"], resource["description"]);
        let properties =
            &resource["extensions"]["bazaar"]["schema"]["properties"]["input"]["properties"];
        let mut expected = vec![];
        for (key, location) in [("pathParams", "path"), ("queryParams", "query")] {
            if let Some(fields) = properties[key]["properties"].as_object() {
                for (name, schema) in fields {
                    let required = location == "path"
                        || properties[key]["required"]
                            .as_array()
                            .is_some_and(|required| required.contains(&json!(name)));
                    expected.push(
                        json!({"name":name,"in":location,"required":required,"schema":schema}),
                    );
                }
            }
        }
        let mut actual = op["parameters"].as_array().unwrap().clone();
        expected.sort_by_key(|p| (p["in"].to_string(), p["name"].to_string()));
        actual.sort_by_key(|p| (p["in"].to_string(), p["name"].to_string()));
        assert_eq!(actual, expected, "{path}");
        let price = op["x-payment-info"]["price"]["amount"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert_eq!(price, resource["price_usd"].as_f64().unwrap());
    }
    doc["paths"]["/x402/new-action"] = json!({"post":{"summary":"New action"}});
    doc["paths"]["/x402/brain/recent"]["delete"] = json!({"summary":"New method"});
    let tools = catalog::build_tools(&cfg, &doc, "newsgurus").unwrap();
    assert_eq!(tools.len(), 49);
    assert!(!tools.iter().any(|t| t.path == "/x402/agents/status"));
    assert!(tools.iter().all(|t| t.method == "GET"));
    let institutions = tools
        .iter()
        .find(|t| t.name == "newsgurus_institutions_symbol")
        .unwrap();
    let args = json!({"symbol":"MSFT","limit":3});
    let route = institutions
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://api.newsgurus.ai/x402/institutions/MSFT");
    assert_eq!(route.query["limit"], 3);
    assert!(route.body.is_none());
    assert!(institutions.description.contains("$0.1"));
    let brain = tools
        .iter()
        .find(|t| t.name == "newsgurus_api_v1_brain_symbol")
        .unwrap();
    assert_eq!(brain.path, "/api/v1/brain/{symbol}");
    let fantasy = tools
        .iter()
        .find(|t| t.name == "newsgurus_fantasy_lineup")
        .unwrap();
    assert_eq!(
        fantasy.input_schema["required"],
        json!(["game_date", "roster_player_ids", "scoring_system", "sport"])
    );
    assert_eq!(
        tools
            .iter()
            .find(|t| t.name == "newsgurus_help")
            .unwrap()
            .help_url,
        cfg.help_url
    );
}

#[tokio::test]
async fn vaaya_selects_synchronous_x402_research_and_routes_json_filters() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/vaaya.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    assert!(!cfg.allow_http1 && !cfg.allow_tls12);
    assert!(cfg.additional_properties.is_none());
    let mut doc: Value = serde_json::from_str(include_str!("fixtures/vaaya_openapi.json")).unwrap();
    let discovery: Value =
        serde_json::from_str(include_str!("fixtures/vaaya_x402_discovery.json")).unwrap();
    assert_eq!(doc["paths"].as_object().unwrap().len(), 252);
    doc["paths"]["/api/run/edgar/new-action"] = json!({"post":{"summary":"New action"}});
    doc["paths"]["/api/run/edgar/entities"]["get"] = json!({"summary":"New method"});
    let tools = catalog::build_tools(&cfg, &doc, "vaaya").unwrap();
    assert_eq!(tools.len(), 47);
    for tool in tools.iter().filter(|t| t.help_url.is_none()) {
        assert_eq!(tool.method, "POST");
        assert!(
            discovery["resources"]
                .as_array()
                .unwrap()
                .contains(&json!(format!("https://vaaya.ai{}", tool.path)))
        );
        let payment = &doc["paths"][&tool.path]["post"]["x-payment-info"];
        assert!(
            payment["protocols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p.get("x402").is_some())
        );
        assert!(tool.description.contains(&format!(
                "${}",
                payment["price"]["amount"]
                    .as_str()
                    .unwrap()
                    .trim_end_matches('0')
                    .trim_end_matches('.')
            )));
    }
    for excluded in [
        "/api/run/colors/generate",
        "/api/run/cueframe/render",
        "/api/run/vaaya/supersearch",
        "/api/run/vaaya/fetch",
        "/api/run/firecrawl/crawl",
        "/api/run/firecrawl/extract",
        "/api/run/agentmail/send",
        "/api/run/akta/employee-reviews",
        "/api/run/akta/website-traffic",
    ] {
        assert!(!tools.iter().any(|t| t.path == excluded));
    }
    let search = tools.iter().find(|t| t.name == "vaaya_onesearch").unwrap();
    let args = json!({"query":"Acme filings","facets":["public-filings"],"fidelityRequired":true});
    let route = search
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://vaaya.ai/api/run/vaaya/onesearch");
    assert_eq!(route.body, Some(args));
    assert!(route.query.is_empty());
    let companies = tools
        .iter()
        .find(|t| t.name == "vaaya_theirstack_companies")
        .unwrap();
    let args = json!({"limit":2,"company_domain_or":["example.com"],"job_filters":{"job_title_or":["Rust"]},"vendor_future_filter":true});
    assert_eq!(
        companies
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap()
            .body,
        Some(args)
    );
    let fulltext = tools
        .iter()
        .find(|t| t.name == "vaaya_edgar_fulltext")
        .unwrap();
    assert_eq!(
        fulltext.input_schema["properties"]["startdt"]["pattern"],
        r"^\d{4}-\d{2}-\d{2}$"
    );
    let funding = tools
        .iter()
        .find(|t| t.name == "vaaya_signalbase_funding")
        .unwrap();
    assert!(
        funding.input_schema["properties"]["date_preset"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("last_30d"))
    );
    let scrape = tools
        .iter()
        .find(|t| t.name == "vaaya_firecrawl_scrape")
        .unwrap();
    assert_eq!(scrape.input_schema["properties"]["waitFor"]["minimum"], 0);
    let fundable = tools
        .iter()
        .find(|t| t.name == "vaaya_fundable_company")
        .unwrap();
    assert!(fundable.description.contains("mismatched 1 of 10"));
    assert_eq!(
        tools
            .iter()
            .find(|t| t.name == "vaaya_help")
            .unwrap()
            .help_url,
        cfg.help_url
    );
}

#[tokio::test]
async fn vaaya_tako_and_recallradar_keep_research_bodies_and_structured_prices() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/vaaya.toml"))
        .await
        .unwrap()
        .settings;
    let doc: Value = serde_json::from_str(include_str!("fixtures/vaaya_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "vaaya").unwrap();
    for (name, args, path, price) in [
        (
            "vaaya_tako_search",
            json!({"query":"Acme revenue","source_indexes":["data"]}),
            "/api/run/tako/search",
            "$0.03",
        ),
        (
            "vaaya_tako_answer",
            json!({"query":"Acme revenue","effort":"fast"}),
            "/api/run/tako/answer",
            "$0.03",
        ),
        (
            "vaaya_tako_contents",
            json!({"url":"https://example.com/card"}),
            "/api/run/tako/contents",
            "$0.05",
        ),
        (
            "vaaya_recallradar_search",
            json!({"q":"Acme kettle"}),
            "/api/run/recallradar/search",
            "$0.12",
        ),
        (
            "vaaya_recallradar_get",
            json!({"id":"public-fixture-id"}),
            "/api/run/recallradar/get",
            "$0.03",
        ),
    ] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let route = tool
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.url, format!("https://vaaya.ai{path}"));
        assert_eq!(route.method, "POST");
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
        assert!(tool.description.contains(price));
    }
    let recall = tools
        .iter()
        .find(|t| t.name == "vaaya_recallradar_search")
        .unwrap();
    assert!(
        recall.input_schema["properties"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_ne!(recall.input_schema["additionalProperties"], false);
    let search = tools
        .iter()
        .find(|t| t.name == "vaaya_tako_search")
        .unwrap();
    assert_eq!(search.input_schema["required"], json!(["query"]));
    assert_eq!(search.input_schema["properties"]["effort"]["const"], "fast");
    assert!(
        !tools.iter().any(
            |t| t.path == "/api/run/tako/visualize" || t.path == "/api/run/tako/thinviz-create"
        )
    );
}

#[tokio::test]
async fn privacy_company_catalog_keeps_separate_relayshield_sources() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("examples/deployments/privacy.toml");
    let deployment = config::read_table(&path).await.unwrap();
    let sources = deployment["sources"].as_table().unwrap();
    let company = deployment["servers"]["company"]["sources"]
        .as_array()
        .unwrap();
    assert!(!company.iter().any(|source| source.as_str() == Some("exa")));
    assert!(
        deployment["servers"]["web"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source.as_str() == Some("exa"))
    );
    let cfg = config::resolve(
        sources["company_relayshield"].as_table().unwrap().clone(),
        &path,
    )
    .await
    .unwrap()
    .settings;
    assert_eq!(cfg.spec, "https://api.relayshield.net/openapi.json");
    let original: Value =
        serde_json::from_str(include_str!("fixtures/relayshield_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &original, "relayshield").unwrap();
    assert_eq!(tools.len(), 11);
    assert!(
        !tools
            .iter()
            .any(|tool| tool.name == "relayshield_phone_reputation")
    );
    let companion = config::resolve(
        sources["company_relayshield_phone"]
            .as_table()
            .unwrap()
            .clone(),
        &path,
    )
    .await
    .unwrap()
    .settings;
    let phone: Value = serde_json::from_slice(&std::fs::read(&companion.spec).unwrap()).unwrap();
    let phone_tools = catalog::build_tools(&companion, &phone, "relayshield").unwrap();
    assert_eq!(phone_tools.len(), 1);
    assert_eq!(phone_tools[0].name, "relayshield_phone_reputation");
    for source in ["company_relayshield", "company_relayshield_phone"] {
        assert!(
            company
                .iter()
                .any(|selected| selected.as_str() == Some(source))
        );
    }
    let netintel = config::resolve(
        sources["company_netintel"].as_table().unwrap().clone(),
        &path,
    )
    .await
    .unwrap()
    .settings;
    assert_eq!(netintel.include_tools.len(), 7);
    for removed in [
        "netintel_ip_risk_score",
        "netintel_asn_lookup_analyze",
        "netintel_domain_vendor_risk",
    ] {
        assert!(!netintel.include_tools.iter().any(|name| name == removed));
    }
}
