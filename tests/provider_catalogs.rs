//! Frozen independent catalog expectations; no Python or live vendor needed.
use serde::Deserialize;
use serde_json::Value;
use std::{path::Path, process::Command};
#[derive(Deserialize)]
struct Case {
    name: String,
    provider: String,
    spec: Option<String>,
}
#[test]
fn bundled_catalogs_match_reviewed_tool_contracts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = root;
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/catalogs/cases.json")).unwrap();
    assert_eq!(cases.len(), 17);
    let mut total = 0;
    for case in cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_treazure"));
        command
            .env_clear()
            .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
            .current_dir(repo)
            .arg("--config")
            .arg(&case.provider)
            .arg("--list-tools");
        if let Some(spec) = case.spec {
            command.arg("--spec").arg(spec);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            case.name,
            String::from_utf8_lossy(&output.stderr)
        );
        let mut actual: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
        actual.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let expected: Vec<Value> = serde_json::from_slice(
            &std::fs::read(root.join(format!("tests/fixtures/catalogs/{}.json", case.name)))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(actual.len(), expected.len(), "{} tool count", case.name);
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual, expected, "{}: {}", case.name, expected["name"]);
        }
        total += actual.len();
    }
    assert_eq!(total, 736);
}

#[tokio::test]
async fn directory_exact_allowlist_excludes_new_write_and_subroutes() {
    use x402_treazure::{catalog, config};
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
    assert_eq!(tools.len(), 5);
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
    use x402_treazure::{catalog, config};
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
    use x402_treazure::catalog::{self, Config};
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
    use x402_treazure::{catalog, config};
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
