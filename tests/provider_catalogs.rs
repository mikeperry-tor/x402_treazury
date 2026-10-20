//! Focused captured provider regressions; no live vendor access.
use serde_json::Value;
use std::path::Path;
#[test]
fn reviewed_provider_scope_matches_explicit_paid_preset() {
    use std::collections::BTreeSet;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
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
        let config = row["config"].as_str().unwrap();
        assert!(
            root.join(config).is_file(),
            "missing scoped config {config}"
        );
        assert!(accounted.insert(config));
        if let Some(case) = row["case"].as_str() {
            assert!(
                cases.contains(case),
                "missing reviewed provider case {case}"
            );
            assert!(paid.insert(case));
        }
    }
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
async fn directory_defaults_include_public_operations_and_exclude_ownership() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/x402-list.toml"))
        .await
        .unwrap()
        .settings;
    assert!(!cfg.probe_pricing);
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/x402_list_openapi.json")).unwrap();
    doc["paths"]["/services"]["post"] = serde_json::json!({"description":"new supported request"});
    doc["paths"]["/services/new-public-read"] =
        serde_json::json!({"get":{"description":"new route"}});
    let tools = catalog::build_tools(&cfg, &doc, "x402_list").unwrap();
    assert!(tools.iter().any(|t| t.path == "/services/new-public-read"));
    for path in ["/assess", "/suggestions"] {
        assert!(tools.iter().any(|t| t.path == path && t.method == "POST"));
    }
    for path in [
        "/submit",
        "/services/{slug}/verify-ownership",
        "/services/{slug}/request-update",
        "/services/{slug}/reissue-token",
    ] {
        assert!(
            doc["paths"][path].is_object(),
            "missing ownership witness {path}"
        );
        assert!(!tools.iter().any(|t| t.path == path), "{path}");
    }
    assert!(tools.iter().any(|t| t.path == "/services/{slug}/scores"));
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
    for path in ["/answer", "/batches", "/batches/{id}"] {
        assert!(
            doc["paths"][path].is_object(),
            "missing auth/job witness {path}"
        );
        assert!(!tools.iter().any(|t| t.path == path));
    }
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
    doc["paths"]["/v1/tools/search/jobs"] = json!({"post":{"summary":"New public read"}});
    let tools = catalog::build_tools(&cfg, &doc, "oneshot").unwrap();
    assert!(tools.iter().any(|t| t.name == "oneshot_health"));
    assert!(tools.iter().any(|t| t.path == "/v1/soul/{wallet}"));
    assert!(
        tools
            .iter()
            .any(|t| t.path == "/.well-known/acp/manifest.json")
    );
    assert!(
        !tools.iter().any(
            |t| t.path.contains("/jobs") || (t.path == "/v1/tools/search" && t.method == "GET")
        )
    );
    for path in [
        "/.well-known/acp/manifest.json",
        "/health",
        "/v1/soul",
        "/v1/soul/browse",
        "/v1/soul/search",
        "/v1/soul/service-types",
        "/v1/soul/stats",
        "/v1/soul/templates",
        "/v1/soul/{wallet}",
        "/v1/stats",
        "/v1/status",
    ] {
        assert!(doc["paths"][path].is_object());
        assert!(
            tools.iter().any(|t| t.path == path && t.method == "GET"),
            "{path}"
        );
    }
    for path in ["/v1/tools/research", "/v1/requests/{id}"] {
        assert!(doc["paths"][path].is_object());
        assert!(!tools.iter().any(|t| t.path == path));
    }
    let search = tools.iter().find(|t| t.name == "oneshot_search").unwrap();
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
        tools.last().unwrap().help_url.as_deref(),
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
    doc["paths"]["/api/exa/new-read"] = json!({"post":{"summary":"New public read"}});
    // Synthetic public metadata witnesses, not claims about the captured inventory.
    for path in [
        "/api/hunter/new-read",
        "/api/cloudflare/public-info",
        "/api/google-maps/aerial-view/public-info",
    ] {
        doc["paths"][path] = json!({"get":{"summary":"Public metadata","security":[]}});
    }
    let tools = catalog::build_tools(&cfg, &doc, "stableenrich").unwrap();
    assert!(tools.iter().any(|t| t.path == "/api/health"));
    assert!(tools.iter().any(|t| t.path == "/api/exa/new-read"));
    for path in [
        "/api/hunter/new-read",
        "/api/cloudflare/public-info",
        "/api/google-maps/aerial-view/public-info",
    ] {
        assert!(tools.iter().any(|t| t.path == path), "{path}");
    }
    for path in [
        "/api/cloudflare/crawl",
        "/api/cloudflare/jobs",
        "/api/hunter/email-verifier",
        "/api/hunter/email-verifier/jobs/{jobId}",
        "/api/google-maps/aerial-view/render-video",
        "/api/google-maps/aerial-view/lookup-video",
    ] {
        assert!(doc["paths"][path].is_object());
        assert!(!tools.iter().any(|t| t.path == path), "{path}");
    }
    for path in [
        "/api/google-maps/solar/rgb-image",
        "/api/google-maps/solar/data-layers",
    ] {
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        assert!(tool.response_mapping.is_none()); // Hosted URLs remain ordinary JSON metadata.
    }
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
    assert!(
        selected
            .iter()
            .any(|t| t.name == "stableenrich_google_maps_text_search_partial")
    );
    assert!(
        selected
            .iter()
            .all(|t| t.help_url.is_some() || t.path.starts_with("/api/google-maps/"))
    );
}

#[tokio::test]
async fn agent402_exposes_supported_media_json_and_preserves_request_contracts() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/agent402.toml"))
        .await
        .unwrap()
        .settings;
    assert!(cfg.tags.is_empty());
    let mut doc: Value =
        serde_json::from_str(include_str!("fixtures/agent402_openapi.json")).unwrap();
    doc["paths"]["/api/future-public-read"] = json!({"get":{"tags":["web"]}});
    let tools = catalog::build_tools(&cfg, &doc, "agent402").unwrap();
    for path in [
        "/api/future-public-read",
        "/api/image-resize",
        "/api/image-convert",
        "/api/image-thumbnail",
        "/api/pdf-extract-pages",
        "/api/pdf-rotate",
        "/api/images-to-pdf",
        "/api/audio-normalize",
        "/api/tts",
        "/api/tts-hd",
        "/api/tts-lite",
        "/v1/videos/generations",
        "/api/favicon-grab",
        "/v1/linkedin-article",
        "/api/skill/pdf-pipeline",
        "/api/decide",
        "/api/pdf-merge",
        "/api/audio-convert",
        "/api/onramp-link",
        "/api/skill/media-pipeline",
        "/api/feedback/summary",
    ] {
        assert!(tools.iter().any(|t| t.path == path), "missing {path}");
    }
    for path in [
        "/api/feedback",
        "/api/testnet-fund",
        "/api/my-usage",
        "/api/memory",
        "/api/decide/execute",
        "/v1/audio/speech",
        "/v1/audio/transcriptions",
        "/v1/pro/audio/transcriptions",
    ] {
        assert!(
            doc["paths"][path].is_object(),
            "missing negative witness {path}"
        );
        assert!(!tools.iter().any(|t| t.path == path), "unsupported {path}");
    }
    for path in [
        "/api/image-resize",
        "/api/image-convert",
        "/api/image-thumbnail",
    ] {
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        assert!(
            tool.input_schema["properties"]["format"]["description"]
                .as_str()
                .unwrap()
                .contains("Choose png or jpeg")
        );
    }
    let messages = tools.iter().find(|t| t.path == "/v1/messages").unwrap();
    assert!(
        messages.input_schema["properties"]["stream"]["description"]
            .as_str()
            .unwrap()
            .contains("false")
    );
    let search = tools.iter().find(|t| t.name == "agent402_search").unwrap();
    let routed = search
        .route(
            cfg.base_url.as_ref().unwrap(),
            json!({"q":"Rust & MCP","count":3}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(routed.method, "GET");
    assert_eq!(routed.query["q"], "Rust & MCP");
    assert!(routed.body.is_none());
    let chat = tools
        .iter()
        .find(|t| t.name == "agent402_chat_completions")
        .unwrap();
    let args = json!({"messages":[{"role":"user","content":"Hello"}],"max_tokens":32});
    assert_eq!(
        chat.route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap()
            .body,
        Some(args)
    );
    for path in [
        "/api/image-crop",
        "/api/image-gen",
        "/api/image-gen-hd",
        "/api/image-gen-premium",
        "/v1/images/generations",
        "/v1/images/fast",
        "/v1/images/pro",
    ] {
        assert!(
            tools
                .iter()
                .find(|t| t.path == path)
                .unwrap()
                .response_mapping
                .is_some()
        );
    }
    // Encoded media in an ordinary JSON envelope stays readable JSON; it is not
    // automatically decoded into an MCP image or treated as raw binary.
    let pdf = tools.iter().find(|t| t.path == "/api/pdf-merge").unwrap();
    let output = x402_treazury::output::HttpOutput {
        advisories: vec![],
        bytes: serde_json::to_vec(&json!({"pages":2,"bytes":3,"pdfBase64":"YWJj"})).unwrap(),
        mime_type: Some("application/json".into()),
        paid_submission: false,
    }
    .render(pdf.response_mapping.as_ref(), &pdf.image_limits)
    .unwrap();
    assert!(output.text.contains("YWJj"));
    assert!(output.images.is_empty());
    let mut alias = cfg.clone();
    alias.prefix = Some("custom".into());
    alias.tags = vec!["payments".into()];
    let selected = catalog::build_tools(&alias, &doc, "custom").unwrap();
    assert!(selected.iter().any(|t| t.path == "/api/feedback/summary"));
    assert!(!selected.iter().any(|t| t.path == "/api/feedback"));
}

#[tokio::test]
async fn agentutility_preserves_aliases_json_modes_and_explicit_recommendations() {
    use x402_treazury::{catalog, config, output::HttpOutput};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cfg = config::load(&root.join("providers/agentutility/provider.toml"))
        .await
        .unwrap()
        .settings;
    let mut doc: Value =
        serde_json::from_str(include_str!("../providers/agentutility/openapi.json")).unwrap();
    doc["paths"]["/future-public-read"] =
        serde_json::json!({"get":{"summary":"Synthetic public diagnostic"}});
    let all = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    for path in [
        "/future-public-read",
        "/dns-lookup",
        "/browser-session",
        "/browser-render",
        "/ipfs-fetch",
        "/ipfs-gateway",
        "/qr-code-generate",
        "/qr-maker",
    ] {
        assert!(all.iter().any(|t| t.path == path), "{path}");
    }
    assert!(!all.iter().any(|t| t.path == "/watch-page"));
    for path in ["/web-search", "/search-web"] {
        let tool = all.iter().find(|t| t.path == path).unwrap();
        let args = serde_json::json!({"query":"Rust MCP","num_results":2,"recency":"week"});
        let route = tool
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.url, format!("https://x402.agentutility.ai{path}"));
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
        assert_eq!(
            tool.input_schema["required"],
            doc["paths"][path]["post"]["requestBody"]["content"]["application/json"]["schema"]["required"]
        );
        assert!(tool.description.contains("$0.006"));
    }
    assert_eq!(
        doc["paths"]["/search-web"]["post"]["x-agentutility-alias-of"],
        "web-search"
    );
    cfg.use_recommended_tools = true;
    let selected = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    assert!(selected.iter().any(|t| t.path == "/web-search"));
    assert!(
        !selected
            .iter()
            .any(|t| t.path == "/dns-lookup" || t.path == "/search-web")
    );
    cfg.use_recommended_tools = false;
    let keywords = all.iter().find(|t| t.path == "/keyword-suggest").unwrap();
    assert_eq!(
        keywords.input_schema["anyOf"],
        serde_json::json!([{"required":["query"]},{"required":["keyword"]}])
    );
    cfg.tags = vec!["synthforge".into()];
    let images = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    // Hosted URL results require no automatic asset download.
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
async fn blockrun_fixed_targets_and_hosted_audio_preserve_boundaries() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let name = "blockrun";
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
    doc["paths"]["/api/v1/new-research"] = json!({"post":{"summary":"New synchronous research"}});
    // Synthetic free image metadata; absent from the captured vendor input.
    doc["paths"]["/api/v1/images/models"] =
        json!({"get":{"summary":"Public image model metadata","security":[]}});
    let tools = catalog::build_tools(&cfg, &doc, name).unwrap();
    assert!(tools.iter().any(|t| t.path == "/api/v1/images/models"));
    assert!(tools.iter().any(|t| t.path == "/health"));
    assert!(tools.iter().any(|t| t.path == "/api/v1/new-research"));
    let help = tools
        .iter()
        .find(|t| t.name == format!("{name}_help"))
        .unwrap();
    assert_eq!(help.help_url, cfg.help_url);
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
            .any(|t| t.path == "/api/v1/phone/lookup" && t.has_body)
    );
    for path in [
        "/api/v1/images/generations",
        "/api/v1/images/image2image",
        "/api/v1/audio/generations",
        "/api/v1/videos/generations",
        "/api/v1/modal/sandbox/create",
        "/api/v1/modal/sandbox/status",
        "/api/v1/phone/numbers/buy",
        "/api/v1/zerox/gasless/submit",
        "/api/v1/pm/polymarket/candlesticks/token/0",
    ] {
        assert!(doc["paths"][path].is_object());
        assert!(!tools.iter().any(|t| t.path == path), "{path}");
    }
    assert!(
        !tools
            .iter()
            .any(|t| t.path == "/api/v1/modal/sandbox"
                || t.path.starts_with("/api/v1/modal/sandbox/"))
    );
    for (path, args) in [
        ("/api/v1/audio/speech", json!({"input":"Public fixture"})),
        (
            "/api/v1/audio/sound-effects",
            json!({"text":"Public fixture"}),
        ),
    ] {
        let audio = tools.iter().find(|t| t.path == path).unwrap();
        let route = audio
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.body, Some(args));
        assert!(audio.response_mapping.is_none());
    }
    assert!(!tools.iter().any(|t| t.path == "/api/v1/audio/generations"
        || t.path == "/api/v1/pm/polymarket/candlesticks/token/0"));
    for path in [
        "/api/v1/crypto/price/BTC-USD",
        "/api/v1/crypto/history/BTC-USD",
        "/api/v1/commodity/price/XAU-USD",
        "/api/v1/commodity/history/XAU-USD",
        "/api/v1/fx/price/EUR-USD",
        "/api/v1/fx/history/EUR-USD",
        "/api/v1/pm/binance/candles/BTCUSDT",
        "/api/v1/pm/binance/ticks/BTCUSDT",
    ] {
        let literal = tools.iter().find(|t| t.path == path).unwrap();
        let route = literal
            .route(
                cfg.base_url.as_ref().unwrap(),
                json!({}).as_object().unwrap(),
            )
            .unwrap();
        assert_eq!(route.url, format!("https://blockrun.ai{path}"));
        assert!(
            !literal.input_schema["properties"]
                .as_object()
                .unwrap()
                .contains_key("symbol")
        );
    }
}

#[tokio::test]
async fn claw402_open_rootdata_bodies_preserve_prose_guided_arguments() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let name = "claw402";
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
    doc["paths"]["/api/v1/new-research"] = json!({"post":{"summary":"New synchronous research"}});
    let tools = catalog::build_tools(&cfg, &doc, name).unwrap();
    assert!(tools.iter().any(|t| t.path == "/health"));
    assert!(tools.iter().any(|t| t.path == "/api/v1/new-research"));
    let help = tools
        .iter()
        .find(|t| t.name == format!("{name}_help"))
        .unwrap();
    assert_eq!(help.help_url, cfg.help_url);
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
    for (suffix, args) in [
        ("search", json!({"query":"Acme", "precise_x_search":true})),
        (
            "project",
            json!({"project_id":42,"include_team":true,"include_investors":true}),
        ),
        (
            "org",
            json!({"org_id":7,"include_team":true,"include_investments":true}),
        ),
        (
            "funding",
            json!({"start_time":1700000000,"end_time":1800000000,"min_amount":100,"max_amount":100000,"project_id":42,"page":1,"page_size":5}),
        ),
    ] {
        let tool = tools
            .iter()
            .find(|t| t.name == format!("claw402_crypto_rootdata_{suffix}"))
            .unwrap();
        assert!(tool.has_body);
        let route = tool
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
    }
    let mut selected = cfg.clone();
    selected.tags = vec!["Price".into()];
    let filtered = catalog::build_tools(&selected, &doc, name).unwrap();
    assert_eq!(filtered[0].name, "claw402_nofx_price_ranking");
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
    // Actual lifecycle/authenticated contracts stay excluded; new compatible checks remain visible.
    assert!(doc["paths"]["/v1/payg/scan-file"].is_object());
    assert!(doc["paths"]["/v1/metered/breach"].is_object());
    assert!(doc["paths"]["/v1/account/info"].is_object());
    assert!(doc["paths"]["/v1/payg/phone-reputation"].is_null());
    doc["paths"]["/v1/payg/new-public-check"] =
        serde_json::json!({"post":{"description":"new supported request"}});
    let tools = catalog::build_tools(&cfg, &doc, "relayshield").unwrap();
    for suffix in ["email_check", "link_check", "wallet_risk"] {
        let public = tools
            .iter()
            .find(|t| t.name == format!("relayshield_v1_{suffix}"))
            .unwrap();
        assert!(public.has_body);
        assert!(public.description.contains("free"));
        let args = match suffix {
            "email_check" => {
                serde_json::json!({"from_address":"sender@example.com","subject":"Public fixture"})
            }
            "link_check" => serde_json::json!({"url":"https://example.com/"}),
            _ => serde_json::json!({"address":"0x0000000000000000000000000000000000000001"}),
        };
        let route = public
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(
            route.url,
            format!("https://api.relayshield.net{}", public.path)
        );
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
    }
    assert!(tools.iter().any(|t| t.name == "relayshield_wallet_risk"));
    assert!(tools.iter().any(|t| t.path == "/v1/payg/new-public-check"));
    assert!(!tools.iter().any(
        |t| (t.path == "/v1/metered" || t.path.starts_with("/v1/metered/"))
            || t.path == "/v1/payg/scan-file"
            || (t.path == "/v1/account" || t.path.starts_with("/v1/account/"))
    ));
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
    for resource in raw["resources"].as_array().unwrap() {
        let path = resource["resource"].as_str().unwrap();
        let op = &doc["paths"][path]["get"];
        assert_eq!(op["description"], resource["description"]);
        assert_eq!(
            op["responses"]["200"]["content"]["application/json"]["example"],
            resource["extensions"]["bazaar"]["info"]["output"]["example"]
        );
        assert!(
            op["responses"]["200"]["content"]["application/json"]
                .get("schema")
                .is_none()
        );
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
    assert!(tools.iter().any(|t| t.path == "/x402/new-action"));
    let status = tools
        .iter()
        .find(|t| t.name == "newsgurus_agents_status")
        .unwrap();
    let route = status
        .route(
            cfg.base_url.as_ref().unwrap(),
            json!({}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(route.method, "GET");
    assert_eq!(route.url, "https://api.newsgurus.ai/x402/agents/status");
    assert!(route.query.is_empty() && route.body.is_none());
    assert!(status.description.contains("$0.002"));
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
    doc["paths"]["/api/run/edgar/new-action"] = json!({"post":{"summary":"New action"}});
    let tools = catalog::build_tools(&cfg, &doc, "vaaya").unwrap();
    for path in [
        "/api/run/edgar/new-action",
        "/api/run/cueframe/validate",
        "/api/run/vaaya/supersearch",
        "/api/run/akta/social-posts",
        "/api/run/openai/image-generate",
    ] {
        assert!(tools.iter().any(|t| t.path == path), "missing {path}");
    }
    for excluded in [
        "/api/run/colors/generate",
        "/api/run/cueframe/render",
        "/api/run/agentmail/send",
        "/api/run/crw/extract",
        "/api/run/crw/extract_status",
        "/api/run/files/upload",
        "/api/run/vaaya/fetch",
    ] {
        assert!(doc["paths"][excluded].is_object());
        assert!(!tools.iter().any(|t| t.path == excluded));
    }
    for (path, args) in [
        (
            "/api/run/firecrawl/crawl",
            json!({"url":"https://example.com/"}),
        ),
        (
            "/api/run/firecrawl/extract",
            json!({"urls":["https://example.com/"],"prompt":"Extract public title"}),
        ),
    ] {
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        let route = tool
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
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
    assert!(tools.iter().any(|t| t.path == "/api/run/tako/visualize"));
    assert!(
        tools
            .iter()
            .any(|t| t.path == "/api/run/tako/thinviz-create")
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

#[tokio::test]
async fn netintel_post_bodies_aliases_and_canonical_image_mapping() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/netintel.toml"))
        .await
        .unwrap()
        .settings;
    let doc: Value = serde_json::from_str(include_str!("fixtures/netintel_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "netintel").unwrap();
    for (path, args) in [
        (
            "/breach-check/password",
            json!({"password":"public-unfunded-fixture"}),
        ),
        (
            "/exa/search",
            json!({"query":"Acme", "includeDomains":["example.com"]}),
        ),
        ("/exa/contents", json!({"urls":["https://example.com/"]})),
    ] {
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        assert_eq!(tool.method, "POST");
        assert!(!tool.name.ends_with("_post"));
        let route = tool
            .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(route.body, Some(args));
        assert!(route.query.is_empty());
        assert!(!tools.iter().any(|t| t.path == path && t.method == "GET"));
    }
    for name in [
        "netintel_api_v1_chat_completions",
        "netintel_v1_chat_completions",
        "netintel_api_v1_embeddings",
        "netintel_v1_embeddings",
    ] {
        assert!(tools.iter().any(|t| t.name == name), "{name}");
    }
    let image = tools
        .iter()
        .find(|t| t.path == "/ai-image/generate")
        .unwrap();
    assert!(image.response_mapping.is_some());
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP8sAAAAASUVORK5CYII=";
    let output = x402_treazury::output::HttpOutput {
        advisories: vec![],
        bytes: serde_json::to_vec(
            &json!({"image_url":format!("data:image/png;base64,{png}"),"model":"fixture"}),
        )
        .unwrap(),
        mime_type: Some("application/json".into()),
        paid_submission: false,
    }
    .render(image.response_mapping.as_ref(), &image.image_limits)
    .unwrap();
    assert_eq!(output.images.len(), 1);
    assert!(output.text.contains("fixture"));
    assert!(!output.text.contains(png));
}

#[tokio::test]
async fn hirescrape_names_future_public_routes_without_exposing_known_siwx() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg =
        config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/hirescrape.toml"))
            .await
            .unwrap()
            .settings;
    // Synthetic policy regression, not a vendor inventory or invented request contract.
    let doc = json!({"openapi":"3.0.3","paths":{
        "/api/tools/jobs":{"post":{"summary":"scraper"}},
        "/future-public-diagnostic":{"get":{"summary":"public diagnostic"}},
        "/api/skills/runs":{"post":{"summary":"SIWX run"}},
        "/api/snapshots/{id}":{"get":{"summary":"SIWX snapshot"}}
    }});
    let tools = catalog::build_tools(&cfg, &doc, "hirescrape").unwrap();
    assert!(tools.iter().any(|t| t.name == "hirescrape_jobs"));
    assert!(tools.iter().any(|t| t.path == "/future-public-diagnostic"));
    assert!(!tools.iter().any(|t| t.path == "/api/skills/runs"
        || t.path.starts_with("/api/skills/runs/")
        || t.path == "/api/snapshots"
        || t.path.starts_with("/api/snapshots/")));
}

#[tokio::test]
async fn regimeshift_diagnostics_and_unsigned_intents_preserve_provisioning_boundary() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cfg = config::load(&root.join("providers/regimeshift.toml"))
        .await
        .unwrap()
        .settings;
    let doc: Value =
        serde_json::from_str(include_str!("fixtures/regimeshift_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "regimeshift").unwrap();
    for path in [
        "/health",
        "/stats",
        "/manifest",
        "/v1/intent/lend",
        "/v1/intent/borrow",
    ] {
        assert!(tools.iter().any(|t| t.path == path), "{path}");
    }
    assert!(!tools.iter().any(|t| t.path == "/auto/provision"));
    assert!(tools.iter().any(|t| t.name == "regimeshift_rate_sofr_usd"));
    let cfg = config::load(&root.join("providers/otto.toml"))
        .await
        .unwrap()
        .settings;
    let doc: Value = serde_json::from_str(include_str!("fixtures/otto_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "otto").unwrap();
    assert!(tools.iter().any(|t| t.path == "/crypto-news"));
    assert!(
        !tools
            .iter()
            .any(|t| t.path == "/swap" || t.path == "/full-auto")
    );
}

#[tokio::test]
async fn agentutility_excludes_demonstrated_full_branch_conversion_loss() {
    use x402_treazury::{catalog, config};
    let cfg = config::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/agentutility/provider.toml"),
    )
    .await
    .unwrap()
    .settings;
    let doc: Value =
        serde_json::from_str(include_str!("../providers/agentutility/openapi.json")).unwrap();
    let supported = catalog::build_tools(&cfg, &doc, "agentutility").unwrap();
    for path in [
        "/db-migration-risk",
        "/dep-risk-summary",
        "/deploy-config-risk",
        "/production-readiness-score",
        "/prompt-injection-surface",
        "/secrets-exposure-check",
    ] {
        let original = &doc["paths"][path];
        let body = &original["post"]["requestBody"]["content"]["application/json"]["schema"];
        assert!(body["oneOf"][0]["properties"]["repo"].is_object());
        // Independently exercise each actual input without provider filters. The
        // build succeeds but loses all branch inputs and required alternatives.
        let isolated = serde_json::json!({"paths":{path:original}});
        let converted =
            catalog::build_tools(&catalog::Config::default(), &isolated, "probe").unwrap();
        assert_eq!(
            converted[0].input_schema,
            serde_json::json!({"type":"object","properties":{}})
        );
        assert!(!supported.iter().any(|t| t.path == path), "{path}");
        assert!(
            !doc["paths"]
                .as_object()
                .unwrap()
                .values()
                .any(|item| item["post"]["x-agentutility-alias-of"] == path[1..])
        );
    }
}

#[tokio::test]
async fn concordance_preserves_source_requests_and_public_discovery() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let cfg = config::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/concordance/provider.toml"),
    )
    .await
    .unwrap()
    .settings;
    let doc: Value =
        serde_json::from_str(include_str!("../providers/concordance/openapi.json")).unwrap();
    let raw: Value =
        serde_json::from_str(include_str!("fixtures/concordance_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "concordance").unwrap();
    for path in ["/", "/catalog", "/healthz"] {
        let mut projected = doc["paths"][path].clone();
        assert_eq!(projected["get"]["x-payment-info"]["price"]["amount"], "0");
        projected["get"]
            .as_object_mut()
            .unwrap()
            .remove("x-payment-info");
        assert_eq!(projected, raw["paths"][path]);
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        assert!(tool.description.contains("$0"));
        let route = tool
            .route(
                cfg.base_url.as_ref().unwrap(),
                json!({}).as_object().unwrap(),
            )
            .unwrap();
        assert_eq!(
            route.url,
            format!("https://concordancehq.duckdns.org{path}")
        );
        assert!(route.body.is_none() && route.query.is_empty());
    }
    let source_tools = catalog::build_tools(&cfg, &raw, "concordance").unwrap();
    for tool in tools.iter().filter(|t| t.path.starts_with("/tools/")) {
        let source = source_tools.iter().find(|t| t.path == tool.path).unwrap();
        assert_eq!(tool.input_schema, source.input_schema);
        assert_eq!(tool.param_routes, source.param_routes);
        assert!(!tool.input_schema.to_string().contains("$ref"));
    }
    let series = tools
        .iter()
        .find(|t| t.name == "concordance_series")
        .unwrap();
    let args = json!({"source":"fred","series_id":"GDP"});
    let route = series
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://concordancehq.duckdns.org/tools/series");
    assert_eq!(route.body, Some(args));
    assert_eq!(
        tools
            .iter()
            .find(|t| t.name == "concordance_help")
            .unwrap()
            .help_url
            .as_deref(),
        Some("https://concordancehq.duckdns.org/llms.txt")
    );
}

#[tokio::test]
async fn locus_discovery_and_gateway_preserve_complete_source_contracts() {
    use serde_json::json;
    use std::collections::BTreeSet;
    use x402_treazury::{catalog, config};
    let cfg =
        config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/locus/provider.toml"))
            .await
            .unwrap()
            .settings;
    let doc: Value = serde_json::from_str(include_str!("../providers/locus/openapi.json")).unwrap();
    let raw: Value = serde_json::from_str(include_str!("fixtures/locus_openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "locus").unwrap();
    for path in [
        "/.well-known/locus-tools.json",
        "/.well-known/locus-agent-endpoints.json",
    ] {
        let mut projected = doc["paths"][path].clone();
        assert_eq!(projected["get"]["x-payment-info"]["price"]["amount"], "0");
        projected["get"]
            .as_object_mut()
            .unwrap()
            .remove("x-payment-info");
        assert_eq!(projected, raw["paths"][path]);
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        let route = tool
            .route(
                cfg.base_url.as_ref().unwrap(),
                json!({}).as_object().unwrap(),
            )
            .unwrap();
        assert_eq!(route.url, format!("https://api.locus.report{path}"));
        assert_eq!(route.method, "GET");
        assert!(route.body.is_none() && route.query.is_empty());
        assert!(tool.description.contains("$0"));
    }
    let free: BTreeSet<_> = raw["paths"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.starts_with("/api/locus_"))
        .map(|(p, o)| {
            assert_eq!(o["post"]["x-locus-access"]["charged"], false);
            p.strip_prefix("/api/").unwrap()
        })
        .collect();
    let names: BTreeSet<_> =
        doc["paths"]["/tools/call"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["properties"]["name"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
    assert_eq!(free, names);
    let call = tools.iter().find(|t| t.name == "locus_call").unwrap();
    let args = json!({"name":"locus_coverage_check","arguments":{"place":"1 Main St"}});
    let route = call
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://api.locus.report/tools/call");
    assert_eq!(route.body, Some(args));
    // Preserve the full paid request schema and body-required flag. The authored
    // projection omits the dialect annotation and media examples; those examples
    // remain complete in the independently captured source object.
    let source_body = &raw["paths"]["/api/locus-place-report"]["post"]["requestBody"];
    let projected_body = &doc["paths"]["/api/locus-place-report"]["post"]["requestBody"];
    let mut source_schema = source_body["content"]["application/json"]["schema"].clone();
    source_schema.as_object_mut().unwrap().remove("$schema");
    assert_eq!(
        projected_body["content"]["application/json"]["schema"],
        source_schema
    );
    assert_eq!(projected_body["required"], source_body["required"]);
    let paid = tools
        .iter()
        .find(|t| t.path == "/api/locus-place-report")
        .unwrap();
    assert_eq!(
        paid.input_schema["anyOf"],
        json!([{"required":["address"]},{"required":["zip"]}])
    );
    let args = json!({"address":"1 Main St","radiusMeters":800});
    let route = paid
        .route(cfg.base_url.as_ref().unwrap(), args.as_object().unwrap())
        .unwrap();
    assert_eq!(route.url, "https://api.locus.report/api/locus-place-report");
    assert_eq!(route.body, Some(args));
    for path in [
        "/api/locus-place-report-batch",
        "/api/locus-record-batch",
        "/api/locus-property-update",
        "/api/property-update-jobs/{id}",
    ] {
        assert!(raw["paths"][path].is_object());
        assert!(!tools.iter().any(|t| t.path == path));
    }
}

#[tokio::test]
async fn authored_provider_definitions_allow_compatible_catalog_arrivals() {
    use x402_treazury::{catalog, config};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc = serde_json::json!({"paths":{"/future-public-metadata":{"get":{"summary":"Synthetic supported public metadata","security":[]}}}});
    for file in [
        "agentutility/provider.toml",
        "concordance/provider.toml",
        "curl/provider.toml",
        "glassnode/provider.toml",
        "locus/provider.toml",
        "newsgurus/provider.toml",
        "straits/provider.toml",
        "relayshield/phone-reputation.toml",
    ] {
        let cfg = config::load(&root.join("providers").join(file))
            .await
            .unwrap()
            .settings;
        assert!(!cfg.use_recommended_tools, "{file}");
        let tools = catalog::build_tools(&cfg, &doc, cfg.prefix.as_deref().unwrap()).unwrap();
        assert!(
            tools.iter().any(|t| t.path == "/future-public-metadata"),
            "{file}"
        );
    }
}

#[tokio::test]
async fn straits_public_formats_preserve_routes_tiers_and_historical_meaning() {
    use serde_json::json;
    use x402_treazury::{catalog, config};
    let mut cfg = config::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/straits/provider.toml"),
    )
    .await
    .unwrap()
    .settings;
    let doc: Value =
        serde_json::from_str(include_str!("../providers/straits/openapi.json")).unwrap();
    let tools = catalog::build_tools(&cfg, &doc, "straits").unwrap();
    // Independent literal publication expectations; these are authored from /api,
    // not a vendor OpenAPI capture or invented response examples.
    for path in [
        "/status",
        "/api/v1/vessels",
        "/api/v1/hormuz-snapshot",
        "/api/v1/cape-snapshot",
        "/data",
        "/data/status.csv",
        "/data/transits.csv",
        "/data/hormuz-index.csv",
        "/data/oil.csv",
        "/data/events.csv",
        "/feed.xml",
        "/status/feed.xml",
        "/sanctioned-vessels/feed.xml",
        "/briefs/feed.xml",
        "/embed",
    ] {
        let op = &doc["paths"][path]["get"];
        assert_eq!(op["tags"], json!(["Free"]));
        assert_eq!(op["x-payment-info"]["price"]["amount"], "0");
        assert!(op["responses"]["default"]["content"]["application/json"]["schema"].is_null());
        let tool = tools.iter().find(|t| t.path == path).unwrap();
        let request = tool
            .route(
                cfg.base_url.as_ref().unwrap(),
                json!({}).as_object().unwrap(),
            )
            .unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, format!("https://straits.live{path}"));
        assert!(request.body.is_none() && request.query.is_empty());
        assert!(tool.description.contains("Free"));
    }
    let paid = tools
        .iter()
        .find(|t| t.path == "/api/premium/vessels")
        .unwrap();
    let free = tools.iter().find(|t| t.path == "/api/v1/vessels").unwrap();
    assert_eq!(paid.name, "straits_vessels_get");
    assert_eq!(free.name, "straits_vessels_get_2");
    assert_eq!(
        free.input_schema["properties"]["flagged"]["type"],
        "boolean"
    );
    assert_eq!(
        free.input_schema["properties"]["region"]["enum"],
        json!(["hormuz", "cape"])
    );
    let route = free
        .route(
            cfg.base_url.as_ref().unwrap(),
            json!({"flagged":true,"region":"cape"}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(route.query.len(), 2);
    assert_eq!(route.query["flagged"], true);
    assert_eq!(route.query["region"], "cape");
    assert!(route.body.is_none());
    for path in ["/api/v1/hormuz-snapshot", "/api/v1/cape-snapshot"] {
        let description = &tools.iter().find(|t| t.path == path).unwrap().description;
        assert!(description.contains("2026-08-28") && description.contains("historical"));
    }
    for (tag, path) in [
        ("Free", "/api/v1/vessels"),
        ("Premium", "/api/premium/vessels"),
    ] {
        cfg.tags = vec![tag.into()];
        let selected = catalog::build_tools(&cfg, &doc, "straits").unwrap();
        assert_eq!(
            selected.iter().find(|t| t.path == path).unwrap().name,
            "straits_vessels"
        );
    }
    for path in [
        "/api/premium/stream",
        "/api/premium/stream/session",
        "/api/premium/webhooks",
    ] {
        assert!(doc["paths"][path].is_null());
    }
    assert_eq!(
        doc["x-public-documentation"]["url"],
        "https://straits.live/api"
    );
    assert!(
        doc["x-catalog-source"]
            .as_str()
            .unwrap()
            .contains("straits_digest.json")
    );
}
