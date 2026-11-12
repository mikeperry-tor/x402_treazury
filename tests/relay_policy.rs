//! Offline relay-list composition and pinned bootstrap assets; no wallets or network.
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use x402_treazury::discovery_relay::Policy;

fn policy(input: &str) -> Policy {
    toml::from_str(input).unwrap()
}

#[test]
fn method_lists_preserve_authored_order_without_inheritance_or_provider_io() {
    let parsed = policy(
        r#"
wallet = "research"
sources = ["docs"]
[relays]
GET = ["missing/curl.toml", "missing/agproxy.toml", "missing/curl.toml"]
HEAD = []
POST = ["missing/post.toml"]
"#,
    );
    assert!(parsed.source_discovery && parsed.provider_metadata && !parsed.api_requests);
    assert_eq!(
        parsed.relays()["GET"],
        [
            "missing/curl.toml",
            "missing/agproxy.toml",
            "missing/curl.toml"
        ]
        .map(PathBuf::from)
    );
    assert!(parsed.relays()["HEAD"].is_empty());
    assert_eq!(
        parsed.relays()["POST"],
        [PathBuf::from("missing/post.toml")]
    );
    assert_eq!(parsed.provider_paths().count(), 4);
    let inspection = serde_json::to_value(&parsed).unwrap();
    assert!(inspection.get("provider").is_none());
    assert_eq!(
        inspection["relays"]["GET"],
        json!([
            "missing/curl.toml",
            "missing/agproxy.toml",
            "missing/curl.toml"
        ])
    );
    assert_eq!(inspection["wallet"], "research");
    assert_eq!(inspection["sources"], json!(["docs"]));
    let get_only = policy_from_json(inspection);
    assert_eq!(get_only.relays(), parsed.relays());
    assert!(
        !policy("[relays]\nGET=['missing.toml']")
            .relays()
            .contains_key("HEAD")
    );
    assert!(policy("").relays().is_empty());
    assert!(policy("[relays]").relays().is_empty());
}

fn policy_from_json(value: Value) -> Policy {
    serde_json::from_value(value).unwrap()
}

#[test]
fn section_names_are_strict_in_deployments_and_network_overrides() {
    use x402_treazury::{deployment::MetaConfig, network::NetworkConfig};

    let current = "[relay_fallback.relays]\nGET=['missing.toml']\n";
    let config: MetaConfig = toml::from_str(&format!("version=1\n[servers]\n{current}")).unwrap();
    let shown = serde_json::to_value(config).unwrap();
    assert_eq!(
        shown["relay_fallback"]["relays"]["GET"],
        json!(["missing.toml"])
    );
    assert!(shown.get("discovery_relay").is_none());
    let override_file: NetworkConfig =
        toml::from_str(&format!("[network]\nmode='direct'\n{current}")).unwrap();
    assert!(override_file.relay_fallback.unwrap().has_method("GET"));

    for retired in [
        "[discovery_relay]\n",
        "[relay_fallback.fallbacks]\nGET=['missing.toml']\n",
        "[relay_fallback]\nprovider='missing.toml'\n",
    ] {
        let deployment = format!("version=1\n[servers]\n{retired}");
        assert!(
            toml::from_str::<MetaConfig>(&deployment).is_err(),
            "{retired}"
        );
        let network = format!("[network]\nmode='direct'\n{retired}");
        assert!(
            toml::from_str::<NetworkConfig>(&network).is_err(),
            "{retired}"
        );
    }
}

#[test]
fn shipped_examples_use_explicit_relay_lists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tor = x402_treazury::network::NetworkConfig::load(&root.join("examples/network/tor.toml"))
        .unwrap();
    let relay = tor.relay_fallback.unwrap();
    for method in ["GET", "POST", "HEAD"] {
        assert_eq!(relay.relays()[method].len(), 2);
    }
    assert!(
        relay
            .provider_paths()
            .all(|path| path.is_absolute() && path.is_file())
    );
    let comprehensive: x402_treazury::deployment::MetaConfig =
        toml::from_str(include_str!("../examples/deployments/comprehensive.toml")).unwrap();
    let relay = comprehensive.relay_fallback.unwrap();
    for method in ["GET", "POST", "HEAD"] {
        assert_eq!(
            relay.relays()[method],
            [
                "../../providers/readfetch.toml",
                "../../providers/agproxy.toml"
            ]
            .map(PathBuf::from)
        );
    }
}

#[test]
fn retired_keys_and_invalid_method_lists_are_rejected() {
    for input in [
        "provider='old.toml'",
        "fallbacks={}",
        "[fallbacks]\nGET=['old.toml']",
        "provider='old.toml'\n[relays]\nGET=['new.toml']",
        "[relays]\nget=['missing.toml']",
        "[relays]\nPUT=[]",
        "[relays]\nDELETE=['missing.toml']",
        "[relays]\nGET=[{provider='missing.toml',adapter='curl'}]",
        "unknown=true",
    ] {
        assert!(toml::from_str::<Policy>(input).is_err(), "{input}");
    }
}

#[test]
fn method_paths_resolve_at_declaration_without_reading_missing_providers() {
    let directory = tempfile::tempdir().unwrap();
    let declaring = directory.path().join("network/tor.toml");
    let absolute = directory.path().join("absolute.toml");
    let mut listed = policy(&format!(
        "[relays]\nGET=['../curl.toml','{}']\nPOST=['post.toml']",
        absolute.display()
    ));
    listed.resolve_paths(&declaring).unwrap();
    assert_eq!(
        listed.relays()["GET"],
        [
            std::path::absolute(directory.path().join("network/../curl.toml")).unwrap(),
            absolute
        ]
    );
    assert_eq!(
        listed.relays()["POST"],
        [directory.path().join("network/post.toml")]
    );
    assert!(listed.provider_paths().all(|path| !path.exists()));
    let config: x402_treazury::deployment::MetaConfig =
        toml::from_str("version=1\n[servers]").unwrap();
    listed.validate(&config).unwrap();
    assert!(policy("wallet='absent'").validate(&config).is_err());
    assert!(policy("sources=['absent']").validate(&config).is_err());
}

fn check_refs(document: &Value, node: &Value) {
    let mut pending = vec![(node, None::<&Value>)];
    while let Some((node, defs)) = pending.pop() {
        match node {
            Value::Object(fields) => {
                let nearest_defs = fields.get("$defs").or(defs);
                if let Some(reference) = fields.get("$ref") {
                    let reference = reference.as_str().unwrap();
                    let pointer = reference
                        .strip_prefix('#')
                        .expect("bootstrap references must stay in the pinned document");
                    let local = reference
                        .strip_prefix("#/$defs/")
                        .and_then(|name| nearest_defs.and_then(|defs| defs.get(name)));
                    assert!(
                        document.pointer(pointer).is_some() || local.is_some(),
                        "unresolved pinned reference {reference}"
                    );
                }
                pending.extend(fields.values().map(|value| (value, nearest_defs)));
            }
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, defs))),
            _ => {}
        }
    }
}

#[test]
fn relay_bootstrap_assets_preserve_complete_primary_catalogs_and_internal_refs() {
    for (id, format, relative, primary) in [
        (
            "agproxy",
            "agproxy",
            "agproxy-openapi.json",
            include_str!("../providers/agproxy-openapi.json"),
        ),
        (
            "horizonpulse",
            "horizonpulse",
            "horizonpulse-openapi.json",
            include_str!("../providers/horizonpulse-openapi.json"),
        ),
        (
            "curl",
            "curl",
            "openapi.json",
            include_str!("../providers/curl/openapi.json"),
        ),
        (
            "readfetch",
            "readfetch",
            "readfetch-openapi.json",
            include_str!("../providers/readfetch-openapi.json"),
        ),
    ] {
        let provider = x402_treazury::providers::get(id).unwrap();
        assert_eq!(provider.config.relay_format.as_deref(), Some(format));
        assert_eq!(provider.config.spec, relative);
        assert_eq!(provider.local_schema.unwrap(), primary);
        let document: Value = serde_json::from_str(primary).unwrap();
        check_refs(&document, &document);
        assert!(!document["paths"].as_object().unwrap().is_empty());
        assert_eq!(provider.metadata()["schema"]["kind"], "bundled");
        assert_eq!(provider.metadata()["schema"]["reference"], relative);
        assert!(Path::new(relative).is_relative());
    }
    let agproxy: Value =
        serde_json::from_str(include_str!("../providers/agproxy-openapi.json")).unwrap();
    assert!(
        agproxy["paths"]["/x402/proxy"]["post"].is_object(),
        "pin the complete reusable catalog, not just the relay fetch operation"
    );
    let horizon: Value =
        serde_json::from_str(include_str!("../providers/horizonpulse-openapi.json")).unwrap();
    assert!(
        horizon["paths"]["/api/sec/company-facts"]["post"]["requestBody"].is_object(),
        "retain non-relay company-facts request contracts"
    );
    assert!(horizon["paths"].get("/api/screenshot").is_none());
    assert!(
        horizon["components"]["schemas"]
            .get("ScreenshotResponse")
            .is_none()
    );
    let readfetch: Value =
        serde_json::from_str(include_str!("../providers/readfetch-openapi.json")).unwrap();
    assert!(readfetch["paths"]["/v1/fetch"]["post"].is_object());
    assert!(readfetch["paths"]["/v1/text"]["post"].is_object());
    assert!(readfetch["paths"].get("/v1/claims").is_some());
}

#[test]
fn readfetch_provider_exposes_research_surface_without_support_writes() {
    let provider = x402_treazury::providers::get("readfetch").unwrap();
    assert_eq!(provider.config.relay_format.as_deref(), Some("readfetch"));
    let document: Value = serde_json::from_str(provider.local_schema.unwrap()).unwrap();
    let tools =
        x402_treazury::catalog::build_tools(&provider.config, &document, "readfetch").unwrap();
    let names: BTreeSet<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    for expected in [
        "readfetch_fetch",
        "readfetch_text",
        "readfetch_map",
        "readfetch_crawl",
        "readfetch_crawl_id",
        "readfetch_batch",
        "readfetch_pdf",
        "readfetch_section",
        "readfetch_diff",
        "readfetch_screenshot",
        "readfetch_search",
        "readfetch_probe",
        "readfetch_healthz",
    ] {
        assert!(names.contains(expected), "missing {expected}");
    }
    for excluded in [
        "readfetch_claims",
        "readfetch_feedback",
        "readfetch_scrape",
        "readfetch_scrape_residential",
    ] {
        assert!(!names.contains(excluded), "unexpected {excluded}");
    }
}

#[test]
fn categories_are_independent_defaulted_and_retired_controls_reject() {
    let default: Policy = toml::from_str("").unwrap();
    assert!(default.source_discovery && default.provider_metadata && !default.api_requests);
    assert_eq!(
        serde_json::to_value(&default).unwrap(),
        serde_json::to_value(Policy::default()).unwrap()
    );
    for category in ["source_discovery", "provider_metadata", "api_requests"] {
        for enabled in [true, false] {
            let policy: Policy = toml::from_str(&format!("{category}={enabled}")).unwrap();
            let shown = serde_json::to_value(policy).unwrap();
            assert_eq!(shown[category], enabled);
            for other in ["source_discovery", "provider_metadata", "api_requests"] {
                if other != category {
                    assert_eq!(shown[other], serde_json::to_value(&default).unwrap()[other]);
                }
            }
            for retired in ["serve", "warm", "on_demand", "search"] {
                assert!(shown.get(retired).is_none());
            }
        }
    }
    for retired in ["serve", "warm", "on_demand", "search"] {
        for value in ["true", "false", "'malformed'"] {
            assert!(toml::from_str::<Policy>(&format!("{retired}={value}")).is_err());
        }
    }
}
