use serde_json::{Value, json};
use std::path::Path;
use x402_treazury::{config, deployment::Deployment};

#[tokio::test]
async fn composition_replaces_fields_and_resolves_paths_at_their_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(
        &provider,
        r#"
spec = "catalog.json"
prefix = "stable"
tags = ["A", "B"]
timeout = 90
allow_http1 = true
allow_tls12 = true
max_response_bytes = 100
max_help_bytes = 50
max_spec_bytes = 200
catalog_cache_ttl_seconds = 123
[overrides.stable_old]
description = "Old"
"#,
    )
    .unwrap();
    let deployment = dir.path().join("deploy/server.toml");
    std::fs::create_dir(deployment.parent().unwrap()).unwrap();
    let source = toml::from_str(
        r#"extends = "../provider.toml"
tags = []
timeout = 42
allow_http1 = false
http_cache_enabled = false
catalog_cache_ttl_seconds = 0
max_help_bytes = 80
[overrides.stable_new]
description = "New"
"#,
    )
    .unwrap();
    let resolved = config::resolve(source, &deployment).await.unwrap();
    assert!(!resolved.settings.allow_http1);
    assert!(!resolved.settings.http_cache_enabled);
    assert_eq!(resolved.settings.catalog_cache_ttl_seconds, 0);
    assert_eq!(
        resolved.origins["catalog_cache_ttl_seconds"],
        deployment.display().to_string()
    );
    assert_eq!(
        resolved.origins["http_cache_enabled"],
        deployment.display().to_string()
    );
    assert!(resolved.settings.allow_tls12);
    assert!(resolved.origins["allow_tls12"].ends_with("provider.toml"));
    assert_eq!(
        resolved.origins["allow_http1"],
        deployment.display().to_string()
    );
    assert!(resolved.settings.tags.is_empty());
    assert_eq!(resolved.settings.read_timeout_seconds, Some(42.0));
    assert_eq!(resolved.settings.max_response_bytes, 100);
    assert_eq!(resolved.settings.max_help_bytes, 80);
    assert_eq!(resolved.settings.max_spec_bytes, 200);
    assert!(resolved.origins["max_response_bytes"].ends_with("provider.toml"));
    assert_eq!(
        resolved.origins["max_help_bytes"],
        deployment.display().to_string()
    );
    assert_eq!(resolved.settings.prefix.as_deref(), Some("stable"));
    assert!(Path::new(&resolved.settings.spec).ends_with("catalog.json"));
    assert_eq!(
        Path::new(&resolved.settings.spec)
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap(),
        dir.path().canonicalize().unwrap()
    );
    assert_eq!(
        resolved.settings.overrides,
        json!({"stable_new":{"description":"New"}})
    );
    assert_eq!(
        resolved.origins["read_timeout_seconds"],
        deployment.display().to_string()
    );
    assert!(resolved.origins["spec"].ends_with("provider.toml"));
    let resolved = config::resolve(
        toml::from_str("extends = \"../provider.toml\"\nspec = \"local.json\"").unwrap(),
        &deployment,
    )
    .await
    .unwrap();
    assert_eq!(resolved.settings.catalog_cache_ttl_seconds, 123);
    assert_eq!(
        resolved.settings.spec,
        deployment
            .parent()
            .unwrap()
            .join("local.json")
            .display()
            .to_string()
    );
}

#[tokio::test]
async fn composition_rejects_json_unknown_fields_nested_extends_and_invalid_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.toml");
    for invalid in [
        r#"{"spec":"api.json"}"#,
        "spec = 'api.json'\nunknown = true",
        "spec = 'api.json'\ncatalog_cache_ttl_seconds = -1",
        "spec = 'api.json'\ncatalog_cache_ttl_seconds = 1.5",
        "spec = 'api.json'\ntimeout = 0",
        "extends = ['a.toml']",
        "spec = 'api.json'\nmax_response_bytes = 0",
        "spec = 'api.json'\nmax_help_bytes = 0",
        "spec = 'api.json'\nmax_spec_bytes = 0",
        "spec = 'api.json'\nmax_response_bytes = -1",
        "config = 'old.json'",
        "spec = 'api.json'\nallow_http1 = 'yes'",
        "spec = 'api.json'\nallow_tls12 = 1",
        "spec = 'api.json'\nprobe_methods = ['POST']",
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(config::load(&path).await.is_err(), "{invalid}");
    }
    std::fs::write(
        dir.path().join("provider.toml"),
        "extends = 'source.toml'\nspec = 'api.json'",
    )
    .unwrap();
    std::fs::write(&path, "extends = 'provider.toml'").unwrap();
    assert!(format!("{:#}", config::load(&path).await.err().unwrap()).contains("nested extends"));
}

#[tokio::test]
async fn every_bundled_provider_validates_offline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers");
    let mut pending = vec![root];
    let mut definitions = 0;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "toml") {
                let cfg = config::load(&path).await.unwrap().settings;
                if !cfg.spec.starts_with("https://") && !cfg.spec.starts_with("http://") {
                    assert!(Path::new(&cfg.spec).exists(), "{}", path.display());
                }
                definitions += 1;
            }
        }
    }
    assert!(definitions > 0);
}

fn deployment_text() -> &'static str {
    r#"
version = 1
[sources.shared]
spec = "api.json"
prefix = "api"
base_url = "http://127.0.0.1:1"
probe_pricing = false
[wallets.default]
mode = "static"
private_key_env = "PRIVATE_ENV_REFERENCE"
[servers.a]
listen = "127.0.0.1:0"
bearer_token_env = "AUTH_ENV_REFERENCE"
wallet = "default"
sources = ["shared"]
tags = ["A", "B"]
exclude_tags = ["B"]
[servers.b]
listen = "127.0.0.1:0"
bearer_token_env = "AUTH_ENV_REFERENCE"
wallet = "default"
sources = ["shared"]
tags = ["B"]
"#
}

#[tokio::test]
async fn listeners_select_tags_from_one_shared_inline_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("api.json"),
        json!({"paths":{
            "/a":{"get":{"tags":["A"]}},"/b":{"get":{"tags":["B"]}},"/untagged":{"get":{}}
        }})
        .to_string(),
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, deployment_text()).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    assert_eq!(inventory[0].tools.len(), 1);
    assert_eq!(inventory[0].tools[0].tool.name, "api_a");
    assert_eq!(inventory[1].tools.len(), 1);
    assert_eq!(inventory[1].tools[0].tool.name, "api_b");
    let running = deployment
        .bind(&std::collections::BTreeMap::from([
            ("PRIVATE_ENV_REFERENCE".into(), format!("{:064x}", 1)),
            ("AUTH_ENV_REFERENCE".into(), "test-token".into()),
        ]))
        .await
        .unwrap();
    assert_eq!(running.addresses().len(), 2);
}

#[test]
fn show_config_is_offline_reports_origins_and_never_resolves_secret_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    // api.json intentionally does not exist: inspecting composition never loads specs.
    std::fs::write(&path, deployment_text()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x402_treazury"))
        .env_clear()
        .envs(std::env::var("LLVM_PROFILE_FILE").map(|value| ("LLVM_PROFILE_FILE", value)))
        .env("PRIVATE_ENV_REFERENCE", "do-not-expose-this")
        .args(["config", "show", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("do-not-expose-this"));
    let shown: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(shown["sources"]["shared"]["settings"]["prefix"], "api");
    assert_eq!(
        shown["sources"]["shared"]["origins"]["read_timeout_seconds"],
        "default"
    );
    assert!(
        shown["sources"]["shared"]["origins"]["spec"]
            .as_str()
            .unwrap()
            .ends_with("deployment.toml")
    );
}

#[tokio::test]
async fn discovery_relay_policy_is_offline_explicit_and_validates_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let text = deployment_text();
    let relay = "\n[discovery_relay]\nprovider='missing-local-provider.toml'\nwallet='default'\nserve=true\nwarm=true\nsources=['shared']\n";
    std::fs::write(&path, format!("{text}{relay}")).unwrap();
    let shown = x402_treazury::deployment::Deployment::show_config(&path)
        .await
        .unwrap();
    assert_eq!(shown["discovery_relay"]["wallet"], "default");
    assert_eq!(shown["discovery_relay"]["serve"], true);
    std::fs::write(&path,format!("{text}{relay}\n[source_management]\nwallet='default'\nregistry_file='missing-local-provider.toml'\n")).unwrap();
    let error = x402_treazury::deployment::Deployment::show_config(&path)
        .await
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("aliases a protected"));
    for invalid in [
        relay.replace("wallet='default'", "wallet='missing'"),
        relay.replace("sources=['shared']", "sources=['missing']"),
        format!("{relay}max_spend='1'\n"),
    ] {
        std::fs::write(&path, format!("{text}{invalid}")).unwrap();
        assert!(
            x402_treazury::deployment::Deployment::show_config(&path)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn relay_serving_loads_only_its_static_key_before_catalog_selection() {
    use std::collections::BTreeMap;
    use x402_treazury::deployment::Deployment;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("api.json"),
        r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
    )
    .unwrap();
    let provider = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        format!(
            r#"
version=1
[discovery_relay]
provider='{}'
wallet='relay'
serve=true
[wallets.relay]
mode='static'
private_key_env='RELAY_FIXTURE_KEY'
[wallets.used]
mode='static'
private_key_env='USED_FIXTURE_KEY'
[wallets.unused]
mode='static'
private_key_env='NEVER_LOAD_THIS_KEY'
[sources.api]
spec='api.json'
base_url='https://example.com'
wallet='used'
probe_pricing=false
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='AUTH_FIXTURE_TOKEN'
wallet='unused'
sources=['api']
"#,
            provider.display()
        ),
    )
    .unwrap();
    // Inspection ignores the paid relay, without any credentials.
    Deployment::load(&path).await.unwrap();
    let env = BTreeMap::from([
        (
            "RELAY_FIXTURE_KEY".into(),
            "0000000000000000000000000000000000000000000000000000000000000001".into(),
        ),
        ("AUTH_FIXTURE_TOKEN".into(), "fixture-token".into()),
    ]);
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let error = deployment.bind(&env).await.err().unwrap();
    assert!(format!("{error:#}").contains("USED_FIXTURE_KEY"));
    let mut env = env;
    env.insert(
        "USED_FIXTURE_KEY".into(),
        "0000000000000000000000000000000000000000000000000000000000000002".into(),
    );
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let running = deployment.bind(&env).await.unwrap();
    assert_eq!(running.addresses().len(), 1);
    // With no override, only the source-assigned key is required for discovery.
    let automatic = std::fs::read_to_string(&path)
        .unwrap()
        .replace("wallet='relay'\nserve=true", "serve=true");
    std::fs::write(&path, automatic).unwrap();
    env.remove("RELAY_FIXTURE_KEY");
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let running = deployment.bind(&env).await.unwrap();
    assert_eq!(running.addresses().len(), 1);
}

#[tokio::test]
async fn comprehensive_example_enables_source_wallet_discovery_offline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shown = x402_treazury::deployment::Deployment::show_config(
        &root.join("examples/deployments/comprehensive.toml"),
    )
    .await
    .unwrap();
    assert_eq!(shown["discovery_relay"]["serve"], true);
    assert_eq!(shown["discovery_relay"]["warm"], true);
    assert!(shown["discovery_relay"]["wallet"].is_null());
    assert_eq!(shown["discovery_wallets"]["exa"], "web");
    assert_eq!(shown["discovery_wallets"]["social"], "company");
    assert_eq!(
        shown["wallet_bindings"]["social"]["social"]["wallet"],
        "company"
    );
    assert_eq!(shown["discovery_wallets"]["webinfo"], "web");
    assert_eq!(shown["wallet_bindings"]["web"]["exa"]["wallet"], "web");
    assert!(shown["wallet_bindings"]["company"]["exa"].is_null());
    // This deployment deliberately adopts research advice without broadening its
    // independent company/person subsets or changing the shared payer.
    let telesint = &shown["sources"]["telesint"]["settings"];
    assert_eq!(telesint["use_recommended_tools"], true);
    assert_eq!(telesint["exclude_tools"], json!(["telesint_help"]));
    assert_eq!(
        shown["wallet_bindings"]["social"]["telesint"]["wallet"],
        "company"
    );
    let social = &shown["servers"]["social"];
    assert_eq!(social["discover_on_demand"], true);
    let instructions = social["instructions_text"].as_str().unwrap();
    let search = social["discovery_search_tool_instructions_text"]
        .as_str()
        .unwrap();
    for platform in shown["sources"]["social"]["settings"]["tags"]
        .as_array()
        .unwrap()
    {
        let platform = platform.as_str().unwrap();
        assert!(
            search.contains(platform),
            "missing discovery coverage: {platform}"
        );
        assert!(
            !instructions.contains(platform),
            "duplicated initialization coverage: {platform}"
        );
    }
    assert!(search.starts_with("Social tool keywords to search for:"));
    for keyword in [
        "truthsocial",
        "telesint",
        "pdl_person",
        "stableenrich_reddit",
        "stableenrich_minerva",
        "stableenrich_serper_people_image_search",
    ] {
        assert!(
            search.contains(keyword),
            "missing social search keyword: {keyword}"
        );
    }
    assert!(social["sources_search_tool_instructions_text"].is_null());
}

#[tokio::test]
async fn read_timeout_migration_preserves_composition_and_historical_settings() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(&provider, "spec='api.json'\nread_timeout_seconds=90").unwrap();
    let resolve = |text: &str| toml::from_str(text).unwrap();
    let source = dir.path().join("source.toml");
    let inherited = config::resolve(
        resolve("extends='provider.toml'\ntimeout=42\nprobe_timeout=5"),
        &source,
    )
    .await
    .unwrap();
    assert_eq!(inherited.settings.read_timeout_seconds, Some(42.0));
    assert!(inherited.origins["read_timeout_seconds"].ends_with("source.toml"));
    let probe_only = config::resolve(resolve("spec='api.json'\nprobe_timeout=5"), &source)
        .await
        .unwrap();
    assert_eq!(probe_only.settings.read_timeout_seconds, Some(5.0));
    for text in [
        "spec='api.json'\ntimeout=42\nread_timeout_seconds=60",
        "spec='api.json'\ntimeout=42\nprobe_timeout=0",
    ] {
        assert!(config::resolve(resolve(text), &source).await.is_err());
    }
    let historical: x402_treazury::catalog::Config =
        serde_json::from_value(json!({"spec":"api.json","timeout":30.0,"probe_timeout":5.0}))
            .unwrap();
    assert_eq!(historical.read_timeout_seconds, Some(30.0));
}

#[tokio::test]
async fn recommendation_composition_is_typed_offline_and_lists_replace() {
    let dir = tempfile::tempdir().unwrap();
    let provider = dir.path().join("provider.toml");
    std::fs::write(&provider, "spec='https://offline.invalid/spec'\nprefix='api'\nrecommended_tools=['api_read']\nuse_recommended_tools=true\n").unwrap();
    let source = dir.path().join("source.toml");
    let resolve = |text: &str| toml::from_str(text).unwrap();
    let inherited = config::resolve(resolve("extends='provider.toml'"), &source)
        .await
        .unwrap();
    assert_eq!(inherited.settings.recommended_tools, ["api_read"]);
    assert!(inherited.settings.use_recommended_tools);
    assert!(inherited.origins["recommended_tools"].ends_with("provider.toml"));
    let replaced = config::resolve(resolve("extends='provider.toml'\nrecommended_tools=['alias_other']\nprefix='alias'\nuse_recommended_tools=false"), &source).await.unwrap();
    assert_eq!(replaced.settings.recommended_tools, ["alias_other"]);
    assert!(!replaced.settings.use_recommended_tools);
    assert!(replaced.origins["recommended_tools"].ends_with("source.toml"));
    let cleared = config::resolve(
        resolve("extends='provider.toml'\nrecommended_tools=[]"),
        &source,
    )
    .await
    .unwrap();
    assert!(cleared.settings.recommended_tools.is_empty());
    assert!(
        cleared.settings.use_recommended_tools,
        "offline inspection retains invalid empty opt-in for diagnosis"
    );
    for invalid in [
        "recommended_tools=true",
        "recommended_tools=[1]",
        "use_recommended_tools='true'",
        "recommended_tools=['']",
    ] {
        assert!(
            config::resolve(
                resolve(&format!("spec='https://offline.invalid/spec'\n{invalid}")),
                &source
            )
            .await
            .is_err(),
            "{invalid}"
        );
    }
}

#[tokio::test]
async fn recommendations_preserve_source_alias_listener_filters_and_wallet_bindings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("api.json"),
        json!({"paths":{"/read":{"get":{}},"/other":{"get":{}},"/hidden":{"get":{}}}}).to_string(),
    )
    .unwrap();
    std::fs::write(dir.path().join("provider.toml"), "spec='api.json'\nbase_url='https://api.example'\nprefix='api'\nrecommended_tools=['api_read']\nuse_recommended_tools=true\nexclude_tools=['api_hidden']").unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        r#"
version=1
[sources.first]
extends='provider.toml'
[sources.second]
extends='provider.toml'
recommended_tools=['api_other']
[wallets.one]
mode='static'
private_key_env='UNREAD_ONE'
[wallets.two]
mode='static'
private_key_env='UNREAD_TWO'
[servers.a]
listen='127.0.0.1:0'
bearer_token_env='UNREAD_TOKEN'
wallet='one'
sources=['first']
include_tools=['api_read']
[servers.b]
listen='127.0.0.1:0'
bearer_token_env='UNREAD_TOKEN'
wallet='two'
sources=['second']
"#,
    )
    .unwrap();
    let selected = Deployment::load(&path).await.unwrap();
    let all = Deployment::load_for_inspection(&path, true).await.unwrap();
    let selected = serde_json::to_value(selected.inventory()).unwrap();
    let all = serde_json::to_value(all.inventory()).unwrap();
    assert_eq!(selected[0]["tools"][0]["name"], "api_read");
    assert_eq!(selected[1]["tools"][0]["name"], "api_other");
    assert_eq!(selected[0]["tools"][0]["source"], "first");
    assert_eq!(selected[1]["tools"][0]["source"], "second");
    assert_eq!(selected[0]["wallet_bindings"], all[0]["wallet_bindings"]);
    assert_eq!(selected[1]["wallet_bindings"], all[1]["wallet_bindings"]);
    assert_eq!(
        all[0]["tools"].as_array().unwrap().len(),
        1,
        "listener narrowing remains"
    );
    assert_eq!(
        all[1]["tools"].as_array().unwrap().len(),
        2,
        "source exclusion remains"
    );
    assert!(
        all[1]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "api_read" && t["recommended"] == false)
    );
    assert!(
        selected
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["tools"].as_array().unwrap().len() == 1)
    );
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(
        shown["sources"]["first"]["settings"]["use_recommended_tools"],
        true
    );
    assert_eq!(
        shown["sources"]["second"]["settings"]["recommended_tools"],
        json!(["api_other"])
    );
}

#[tokio::test]
async fn provider_description_inherits_and_replaces_with_field_origins() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("provider.toml"),
        "spec='schema.json'\ndescription='Corporate registry search'\n",
    )
    .unwrap();
    let source = dir.path().join("source.toml");
    let inherited = config::resolve(toml::from_str("extends='provider.toml'").unwrap(), &source)
        .await
        .unwrap();
    assert_eq!(
        inherited.settings.description.as_deref(),
        Some("Corporate registry search")
    );
    assert!(inherited.origins["description"].ends_with("provider.toml"));
    let replaced = config::resolve(
        toml::from_str("extends='provider.toml'\ndescription='Local research view'").unwrap(),
        &source,
    )
    .await
    .unwrap();
    assert_eq!(
        replaced.settings.description.as_deref(),
        Some("Local research view")
    );
    assert_eq!(
        replaced.origins["description"],
        source.display().to_string()
    );
    let serialized = serde_json::to_value(&replaced.settings).unwrap();
    assert_eq!(serialized["description"], "Local research view");
    assert!(
        toml::from_str::<x402_treazury::catalog::Config>("spec='schema.json'")
            .unwrap()
            .description
            .is_none()
    );
}

#[tokio::test]
async fn search_relay_opt_in_is_separate_from_static_bindings_and_wallet_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let text = r#"version=1
[wallets.payer]
mode="static"
private_key_env="UNUSED_PUBLIC_TEST_KEY"
[source_management]
wallet="payer"
[servers.tools]
listen="127.0.0.1:0"
bearer_token_env="UNUSED_TEST_TOKEN"
source_management=true
sources=[]
[discovery_relay]
provider="missing-local-provider.toml"
search=true
"#;
    std::fs::write(&path, text).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    assert_eq!(shown["discovery_relay"]["search"], true);
    assert_eq!(shown["discovery_relay"]["serve"], false);
    assert_eq!(shown["discovery_relay"]["warm"], false);
    assert_eq!(shown["discovery_wallets"], json!({}));
    assert_eq!(
        shown["source_management"]["wallet_bindings"]["tools"],
        "payer"
    );
    // Config-only inspection never loads the relay catalog or wallet key; strict
    // loading validates local bootstrap configuration before serving can use it.
    assert!(Deployment::load(&path).await.is_err());
    assert!(!dir.path().join("state").exists());
    let provider = format!(
        "spec={:?}\nbase_url='https://spec.example.com'\nprobe_pricing=false\n",
        Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/openapi.json")
    );
    std::fs::write(dir.path().join("relay.toml"), provider).unwrap();
    std::fs::write(
        &path,
        text.replace("missing-local-provider.toml", "relay.toml"),
    )
    .unwrap();
    let loaded = Deployment::load(&path).await.unwrap();
    let bound = loaded
        .bind(&std::collections::BTreeMap::from([
            ("UNUSED_PUBLIC_TEST_KEY".into(), format!("{:064x}", 1)),
            ("UNUSED_TEST_TOKEN".into(), "fixture-token".into()),
        ]))
        .await
        .unwrap();
    assert_eq!(bound.addresses().len(), 1);
    drop(bound);
    assert!(!dir.path().join("state").exists());
    std::fs::write(&path, text.replace("search=true", "search=\"yes\"")).unwrap();
    assert!(Deployment::show_config(&path).await.is_err());
}
