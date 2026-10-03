//! Join AgentUtility's OpenAPI request contracts with its more complete registry tags.
//! Local files only: fetching/reviewing the two upstream documents is an explicit step.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
const DEFAULTS: &[&str] = &[
    "web-search",
    "answer-web",
    "research-brief",
    "scrape",
    "scrape-to-json",
    "link-extract",
    "archive-snapshot",
    "arxiv-search",
    "arxiv-summarize",
    "pubmed-search",
    "hn-search",
    "wikipedia-search",
    "github-readme",
    "youtube-transcript",
    "pdf-to-markdown",
];
#[derive(Parser)]
struct Args {
    spec: PathBuf,
    registry: PathBuf,
    output: PathBuf,
}
fn snapshot(mut spec: Value, registry: &Value) -> Result<Vec<u8>> {
    let services = registry["services"]
        .as_object()
        .context("registry services must be an object")?;
    ensure!(
        registry["totalServices"].as_u64() == Some(services.len() as u64),
        "registry service count mismatch"
    );
    let paths = spec["paths"]
        .as_object_mut()
        .context("OpenAPI paths must be an object")?;
    ensure!(
        paths.len() == services.len(),
        "spec and registry inventories differ"
    );
    for (slug, service) in services {
        ensure!(
            service["slug"].as_str() == Some(slug)
                && !slug.is_empty()
                && slug
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid service slug"
        );
        ensure!(
            service["url"] == format!("https://x402.agentutility.ai/{slug}"),
            "unexpected service origin/path"
        );
        ensure!(
            service["methods"] == json!(["POST"]),
            "unexpected service methods"
        );
        let item = paths
            .get_mut(&format!("/{slug}"))
            .context("registry route missing from spec")?
            .as_object_mut()
            .context("invalid path item")?;
        ensure!(
            item.len() == 1 && item.contains_key("post"),
            "unexpected OpenAPI methods"
        );
        let op = item["post"].as_object_mut().context("invalid operation")?;
        ensure!(
            op["requestBody"]["content"]["application/json"]["schema"].is_object(),
            "missing request schema"
        );
        let mut tags = BTreeSet::new();
        for list in [op.get("tags"), service.get("tags")].into_iter().flatten() {
            for tag in list.as_array().context("tags must be an array")? {
                tags.insert(tag.as_str().context("tag must be a string")?.to_owned());
            }
        }
        tags.insert(
            service["cluster"]
                .as_str()
                .context("missing cluster")?
                .to_owned(),
        );
        if DEFAULTS.contains(&slug.as_str()) {
            ensure!(
                service.get("aliasOf").is_none(),
                "default route became an alias; review required"
            );
            tags.insert("treazure-research".into());
        }
        if let Some(alias) = service.get("aliasOf") {
            ensure!(
                alias.as_str().is_some_and(|s| services.contains_key(s)),
                "unknown alias target"
            );
            op.insert("x-agentutility-alias-of".into(), alias.clone());
        }
        op.insert("tags".into(), serde_json::to_value(tags)?);
        op.remove("responses");
    }
    for slug in DEFAULTS {
        ensure!(
            services.contains_key(*slug),
            "default research route missing; review required"
        );
    }
    spec["x-treazure-provenance"] = json!({"openapi":"https://x402.agentutility.ai/openapi.json","registry":"https://agentutility.ai/registry.json","transforms":["union OpenAPI tags, registry tags and cluster","add treazure-research to reviewed default routes","record registry aliasOf","omit response documentation"],"service_count":services.len()});
    spec.sort_all_objects();
    Ok(format!("{}\n", serde_json::to_string(&spec)?).into_bytes())
}
fn main() -> Result<()> {
    let args = Args::parse();
    let spec = serde_json::from_slice(&std::fs::read(args.spec)?)?;
    let registry = serde_json::from_slice(&std::fs::read(args.registry)?)?;
    let bytes = snapshot(spec, &registry)?;
    std::fs::write(&args.output, bytes)?;
    eprintln!("Wrote {}", args.output.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> (Value, Value) {
        let mut paths = serde_json::Map::new();
        let mut services = serde_json::Map::new();
        for slug in DEFAULTS.iter().copied().chain(["alias-search"]) {
            paths.insert(format!("/{slug}"),json!({"post":{"description":"Keep full guidance","requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"q":{"type":"string"}},"anyOf":[{"required":["q"]}]}}}},"responses":{"200":{}}}}));
            services.insert(slug.into(),json!({"slug":slug,"methods":["POST"],"url":format!("https://x402.agentutility.ai/{slug}"),"cluster":"web-probe","tags":["search"]}));
        }
        services["alias-search"]["aliasOf"] = json!("web-search");
        (
            json!({"openapi":"3.1.0","paths":paths}),
            json!({"totalServices":services.len(),"services":services}),
        )
    }
    #[test]
    fn preserves_requests_restores_tags_and_marks_aliases_deterministically() {
        let (s, r) = inputs();
        let bytes = snapshot(s.clone(), &r).unwrap();
        assert_eq!(bytes, snapshot(s.clone(), &r).unwrap());
        let output: Value = serde_json::from_slice(&bytes).unwrap();
        let op = &output["paths"]["/web-search"]["post"];
        assert_eq!(
            op["requestBody"],
            s["paths"]["/web-search"]["post"]["requestBody"]
        );
        assert_eq!(op["description"], "Keep full guidance");
        assert_eq!(
            op["tags"],
            json!(["search", "treazure-research", "web-probe"])
        );
        assert!(op.get("responses").is_none());
        assert_eq!(
            output["paths"]["/alias-search"]["post"]["x-agentutility-alias-of"],
            "web-search"
        );
    }
    #[test]
    fn mismatched_inventory_origins_and_methods_fail_before_writing() {
        for field in ["url", "methods", "aliasOf"] {
            let (s, mut r) = inputs();
            r["services"]["web-search"][field] = match field {
                "url" => json!("https://wrong.example/web-search"),
                "methods" => json!(["GET"]),
                _ => json!("missing"),
            };
            assert!(snapshot(s, &r).is_err());
        }
        let (mut s, r) = inputs();
        s["paths"].as_object_mut().unwrap().remove("/web-search");
        assert!(snapshot(s, &r).is_err());
    }
}
