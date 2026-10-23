//! Offline relay-list composition and pinned bootstrap assets; no wallets or network.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use x402_treazury::discovery_relay::Policy;

fn policy(input: &str) -> Policy {
    toml::from_str(input).unwrap()
}

#[test]
fn method_lists_preserve_authored_order_without_inheritance_or_provider_io() {
    let parsed = policy(
        r#"
serve = true
warm = true
wallet = "research"
sources = ["docs"]
[fallbacks]
GET = ["missing/curl.toml", "missing/agproxy.toml", "missing/curl.toml"]
HEAD = []
POST = ["missing/post.toml"]
"#,
    );
    assert!(!parsed.is_legacy());
    assert!(parsed.search && parsed.serve && parsed.warm);
    assert_eq!(
        parsed.fallbacks()["GET"],
        [
            "missing/curl.toml",
            "missing/agproxy.toml",
            "missing/curl.toml"
        ]
        .map(PathBuf::from)
    );
    assert!(parsed.fallbacks()["HEAD"].is_empty());
    assert_eq!(
        parsed.fallbacks()["POST"],
        [PathBuf::from("missing/post.toml")]
    );
    assert_eq!(parsed.provider_paths().count(), 4);
    let inspection = serde_json::to_value(&parsed).unwrap();
    assert!(inspection.get("provider").is_none());
    assert_eq!(
        inspection["fallbacks"]["GET"],
        json!([
            "missing/curl.toml",
            "missing/agproxy.toml",
            "missing/curl.toml"
        ])
    );
    assert_eq!(inspection["wallet"], "research");
    assert_eq!(inspection["sources"], json!(["docs"]));
    let get_only = policy_from_json(inspection);
    assert_eq!(get_only.fallbacks(), parsed.fallbacks());
    assert!(
        !policy("[fallbacks]\nGET=['missing.toml']")
            .fallbacks()
            .contains_key("HEAD")
    );
    assert!(policy("").fallbacks().is_empty());
    assert!(policy("[fallbacks]").fallbacks().is_empty());
}

fn policy_from_json(value: Value) -> Policy {
    serde_json::from_value(value).unwrap()
}

#[test]
fn legacy_provider_is_one_get_entry_with_explicit_legacy_provenance() {
    let legacy = policy("provider='missing.toml'\nsearch=false");
    assert!(legacy.is_legacy());
    assert!(!legacy.search);
    assert!(!legacy.serve && !legacy.warm);
    assert_eq!(legacy.fallbacks()["GET"], [PathBuf::from("missing.toml")]);
    assert_eq!(legacy.fallbacks().len(), 1);
    let canonical = serde_json::to_value(&legacy).unwrap();
    assert!(canonical.get("provider").is_none());
    assert_eq!(canonical["fallbacks"], json!({"GET":["missing.toml"]}));
    assert!(
        !policy_from_json(canonical).is_legacy(),
        "canonical lists do not implicitly grant Curl compatibility"
    );
    for input in [
        "provider='old.toml'\nfallbacks={}",
        "provider='old.toml'\n[fallbacks]\nGET=['new.toml']",
        "[fallbacks]\nget=['missing.toml']",
        "[fallbacks]\nPUT=[]",
        "[fallbacks]\nDELETE=['missing.toml']",
        "[fallbacks]\nGET=[{provider='missing.toml',adapter='curl'}]",
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
        "[fallbacks]\nGET=['../curl.toml','{}']\nPOST=['post.toml']",
        absolute.display()
    ));
    listed.resolve_paths(&declaring).unwrap();
    assert_eq!(
        listed.fallbacks()["GET"],
        [
            std::path::absolute(directory.path().join("network/../curl.toml")).unwrap(),
            absolute
        ]
    );
    assert_eq!(
        listed.fallbacks()["POST"],
        [directory.path().join("network/post.toml")]
    );
    assert!(listed.provider_paths().all(|path| !path.exists()));
    let mut legacy = policy("provider='../old.toml'");
    legacy.resolve_paths(&declaring).unwrap();
    assert!(legacy.is_legacy());
    assert_eq!(
        legacy.provider.as_ref().unwrap(),
        &legacy.fallbacks()["GET"][0]
    );
    let config: x402_treazury::deployment::MetaConfig =
        toml::from_str("version=1\n[servers]").unwrap();
    listed.validate(&config).unwrap();
    assert!(policy("wallet='absent'").validate(&config).is_err());
    assert!(policy("sources=['absent']").validate(&config).is_err());
}

fn check_refs(document: &Value, node: &Value) {
    match node {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("$ref") {
                let reference = reference.as_str().unwrap();
                let pointer = reference
                    .strip_prefix('#')
                    .expect("bootstrap references must stay in the pinned document");
                assert!(
                    document.pointer(pointer).is_some(),
                    "unresolved pinned reference {reference}"
                );
            }
            for value in fields.values() {
                check_refs(document, value);
            }
        }
        Value::Array(items) => {
            for item in items {
                check_refs(document, item);
            }
        }
        _ => {}
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
        horizon["paths"]["/api/screenshot"].is_object(),
        "retain non-relay catalog consumers"
    );
}
