//! Lossless evidence differences plus production compiler projections.
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use x402_treazury::catalog;

fn changes(before: &Value, after: &Value, path: &str, out: &mut Vec<Value>) {
    if before == after {
        return;
    }
    if let (Some(old), Some(new)) = (before.as_object(), after.as_object()) {
        for key in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
            let pointer = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
            match (old.get(key), new.get(key)) {
                (Some(a), Some(b)) => changes(a, b, &pointer, out),
                (a, b) => out.push(json!({"pointer":pointer,"before":a,"after":b,"kind":if a.is_none(){"added"}else{"removed"}})),
            }
        }
    } else {
        out.push(json!({"pointer":path,"before":before,"after":after,"kind":"changed"}));
    }
}
fn diff(before: &Value, after: &Value) -> Vec<Value> {
    let mut result = vec![];
    changes(before, after, "", &mut result);
    result
}
fn document(raw: &[u8]) -> Value {
    serde_json::from_slice(raw)
        .unwrap_or_else(|_| json!({"text": String::from_utf8_lossy(raw), "sha256": hash(raw)}))
}
fn tool_view(cfg: &Config, items: &Result<Vec<catalog::ToolSpec>>) -> Result<Value> {
    let recommended = cfg.recommendation_matcher()?;
    match items {
        Ok(items) => items
            .iter()
            .map(|tool| {
                Ok((
                    format!("{} {}", tool.method.to_uppercase(), tool.path),
                    serde_json::to_value(catalog::ToolInspection {
                        tool,
                        recommended: recommended(&tool.name),
                    })?,
                ))
            })
            .collect::<Result<serde_json::Map<String, Value>>>()
            .map(Value::Object),
        Err(error) => Ok(json!({"compiler_error":error.to_string()})),
    }
}
fn projection(cfg: &Config, root: &Value) -> Result<Value> {
    let pricing = cfg.effective_pricing()?;
    let catalog_key = pricing.as_ref().and_then(|pricing| pricing.catalog_key());
    let operations: serde_json::Map<String, Value> = catalog::operations(root, catalog_key)?
        .into_iter()
        .map(|op| {
            (
                format!(
                    "{} {}",
                    op["method"].as_str().unwrap_or("").to_uppercase(),
                    op["path"].as_str().unwrap_or("")
                ),
                op,
            )
        })
        .collect();
    let total_operations = operations.len();
    // Metadata presence is not an estimate: an operation may carry auth flags or
    // a shape the adapter leaves display-only/unknown.
    let pricing_metadata_operations = operations
        .values()
        .filter(|operation| !operation["pricing"].is_null())
        .count();
    let prefix = cfg.prefix.clone().map(Ok).unwrap_or_else(|| {
        catalog::default_prefix(cfg.base_url.as_deref().context("missing base_url/prefix")?)
    })?;
    let effective = catalog::build_tools(cfg, root, &prefix);
    let mut plain_cfg = cfg.clone();
    plain_cfg.overrides = json!({});
    let plain = catalog::build_tools(&plain_cfg, root, &prefix);
    // A failing tool-name selector must not hide its vanished override targets.
    // This auxiliary inventory preserves operation/path/tag/method selection.
    plain_cfg.include_tools.clear();
    plain_cfg.exclude_tools.clear();
    plain_cfg.use_recommended_tools = false;
    let diagnostic = match &plain {
        Ok(items) => Ok(items.clone()),
        Err(_) => catalog::build_tools(&plain_cfg, root, &prefix),
    };
    let diagnostics = cfg.overrides.as_object().into_iter().flatten().map(|(name,patch)| {
        let inventory = plain.as_ref().ok().or_else(||diagnostic.as_ref().ok());
        let tool = inventory.and_then(|items|items.iter().find(|t|&t.name==name));
        let missing = inventory.map(|_|tool.is_none()).or_else(||operations.is_empty().then_some(true));
        let missing_params: Vec<_> = patch["params"].as_object().into_iter().flatten().filter(|(key,_)|tool.is_some_and(|t|t.parameter_note_pointer(key).is_none()) || missing==Some(true)).map(|(key,_)|key).collect();
        json!({"name":name,"authored_override":patch,"missing_tool":missing,"missing_parameter_notes":missing_params,"unoverridden":tool,"effective":effective.as_ref().ok().and_then(|items|items.iter().find(|t|&t.name==name))})
    }).collect::<Vec<_>>();
    Ok(
        json!({"operations":operations,"pricing":{"catalog_key":catalog_key,"pricing_metadata_operations":pricing_metadata_operations,"total_operations":total_operations},"effective":tool_view(cfg,&effective)?,"unoverridden":tool_view(cfg,&plain)?,"override_diagnostic_inventory":tool_view(cfg,&diagnostic)?,"overrides":diagnostics,"config":cfg}),
    )
}
fn projected(cfg: &Config, raw: &[u8]) -> Value {
    match serde_json::from_slice(raw)
        .map_err(anyhow::Error::from)
        .and_then(|root| projection(cfg, &root))
    {
        Ok(value) => value,
        Err(error) => json!({"compiler_error":error.to_string()}),
    }
}
fn schema(provider: &Provider) -> Option<&Read> {
    let mut sources = provider.inputs.iter().filter(|r| r.purpose == "schema");
    let first = sources.next()?;
    if sources.next().is_some() {
        None
    } else {
        Some(first)
    }
}
fn projected_input(run: &Path, provider: &Provider, read: Option<&Read>) -> Result<Value> {
    match (&provider.config, read.filter(|r| r.hash.is_some())) {
        (Some(cfg), Some(read)) => Ok(projected(cfg, &bytes(run, read)?)),
        _ => Ok(Value::Null),
    }
}
fn runtime_input(provider: &Provider) -> Option<&Read> {
    provider.asset.as_ref().or_else(|| schema(provider))
}
fn asset_document(run: &Path, provider: &Provider) -> Result<Value> {
    provider
        .asset
        .as_ref()
        .map(|read| bytes(run, read).map(|raw| document(&raw)))
        .transpose()
        .map(|value| value.unwrap_or(Value::Null))
}
fn projection_changes(before: &Value, after: &Value) -> Value {
    if before.is_null() || after.is_null() {
        Value::Null
    } else {
        json!(diff(before, after))
    }
}
const PRICING_SIGNAL_LIMIT: usize = 100;
fn pricing_key_name(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("price")
        || key.contains("pricing")
        || key.contains("payment")
        || key.contains("tariff")
        || key.contains("billing")
        || key.contains("cost")
        || key.contains("credit")
        || (key.contains("x402") && key != "x402version")
}
fn signal_pointers(root: &Value) -> (Vec<String>, bool) {
    let mut pointers = BTreeSet::new();
    let mut stack = vec![(String::new(), root)];
    let mut truncated = false;
    while let Some((path, value)) = stack.pop() {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let pointer = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    if pricing_key_name(key) {
                        if pointers.len() >= PRICING_SIGNAL_LIMIT {
                            truncated = true;
                            break;
                        }
                        pointers.insert(pointer.clone());
                    }
                    stack.push((pointer, child));
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    stack.push((format!("{path}/{index}"), child));
                }
            }
            _ => {}
        }
        if truncated {
            break;
        }
    }
    (pointers.into_iter().collect(), truncated)
}
fn remote_purpose(purpose: &str) -> bool {
    matches!(
        purpose,
        "schema" | "registry" | "discovery" | "guide" | "help" | "pricing" | "source"
    )
}
// Advisory key-name discovery only: pointers identify where pricing-like fields
// exist, never what they mean or whether an adapter supports the shape.
fn pricing_signals(run: &Path, provider: &Provider) -> Vec<Value> {
    let mut sources = vec![];
    for read in provider.inputs.iter().chain(provider.pricing.iter()) {
        if !remote_purpose(&read.purpose) || read.hash.is_none() {
            continue;
        }
        let Ok(raw) = bytes(run, read) else {
            continue;
        };
        let Ok(root) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        let (pointers, truncated) = signal_pointers(&root);
        if pointers.is_empty() && !truncated {
            continue;
        }
        sources.push(json!({"source":read.source,"purpose":read.purpose,"pointers":pointers,"truncated":truncated}));
    }
    sources.sort_by(|a, b| {
        (a["source"].as_str(), a["purpose"].as_str())
            .cmp(&(b["source"].as_str(), b["purpose"].as_str()))
    });
    sources
}
fn pricing_sources(run: &Path, provider: &Provider, prior: Option<(&Path, &Provider)>) -> Value {
    let mut sources = vec![];
    for read in &provider.pricing {
        let raw = read.hash.as_ref().and_then(|_| bytes(run, read).ok());
        let (outcome, captured) = match (&read.hash, raw.as_ref()) {
            (Some(_), Some(_)) => ("captured", true),
            (Some(_), None) => ("captured bytes unavailable", false),
            (None, _) => (read.outcome.as_str(), false),
        };
        let shape = raw.as_ref().and_then(|raw| {
            serde_json::from_slice::<Value>(raw).ok().map(|root| {
                root.as_object()
                    .map(|root| root.keys().cloned().collect::<Vec<_>>())
            })
        });
        let comparison = prior
            .and_then(|(path, provider)| {
                provider
                    .pricing
                    .iter()
                    .find(|old| old.source == read.source && old.purpose == read.purpose)
                    .map(|old| (path, old))
            })
            .map(|(path, old)| {
                let previous = old.hash.as_ref().and_then(|_| bytes(path, old).ok());
                match (raw.as_ref(), previous.as_ref()) {
                    (Some(current), Some(previous)) => json!({
                        "basis":"prior capture",
                        "previous_response":old.response,
                        "changes":diff(&document(previous), &document(current)),
                    }),
                    _ => json!({"basis":"comparison unavailable: current or previous pricing bytes unavailable"}),
                }
            })
            .unwrap_or_else(|| {
                json!({"basis":if captured {"baseline established; no historical pricing capture"} else {"source unavailable"}})
            });
        sources.push(json!({
            "source":read.source,
            "purpose":read.purpose,
            "captured":captured,
            "outcome":outcome,
            "response":read.response,
            "shape":shape,
            "comparison":comparison,
        }));
    }
    let removed: Vec<_> = prior
        .into_iter()
        .flat_map(|(_, provider)| provider.pricing.iter())
        .filter(|old| {
            !provider
                .pricing
                .iter()
                .any(|new| new.source == old.source && new.purpose == old.purpose)
        })
        .map(|old| {
            json!({"source":old.source,"purpose":old.purpose,"outcome":"pricing source declaration removed; review relocation"})
        })
        .collect();
    json!({"sources":sources,"removed_sources":removed})
}
fn pricing_section(
    run: &Path,
    provider: &Provider,
    prior: Option<(&Path, &Provider)>,
    runtime: &Value,
) -> Value {
    let current = provider
        .config
        .as_ref()
        .and_then(|cfg| cfg.effective_pricing().ok().flatten());
    let previous = prior
        .and_then(|(_, provider)| provider.config.as_ref())
        .and_then(|cfg| cfg.effective_pricing().ok().flatten());
    let catalog_key = current.as_ref().and_then(|pricing| pricing.catalog_key());
    let external_source = current
        .as_ref()
        .filter(|pricing| pricing.source != "catalog")
        .map(|pricing| pricing.source.clone());
    let previous_external = previous
        .as_ref()
        .filter(|pricing| pricing.source != "catalog")
        .map(|pricing| pricing.source.clone());
    let priced = runtime["pricing"]["pricing_metadata_operations"]
        .as_u64()
        .unwrap_or(0);
    let total = runtime["pricing"]["total_operations"].as_u64().unwrap_or(0);
    let evidence = pricing_sources(run, provider, prior);
    json!({
        "catalog_key": catalog_key,
        "external_source": external_source,
        "evidence": evidence["sources"],
        "relocation": {
            "catalog_key_absent": catalog_key.is_some() && total > 0 && priced == 0,
            "source_changed": prior.is_some() && previous_external != external_source,
            "previous_external_source": previous_external,
            "removed_sources": evidence["removed_sources"],
        },
        "signals": {
            "sources": pricing_signals(run, provider),
            "limit_per_source": PRICING_SIGNAL_LIMIT,
            "advisory": "key-name discovery only; not price authority",
        },
    })
}
fn compare_provider(
    run: &Path,
    provider: &Provider,
    prior: Option<(&Path, &Provider)>,
) -> Result<Value> {
    let current_schema = schema(provider).filter(|r| r.hash.is_some());
    let multiple_schemas = provider
        .inputs
        .iter()
        .filter(|r| r.purpose == "schema")
        .count()
        > 1;
    let mut sources = vec![];
    for read in &provider.inputs {
        let basis = prior
            .and_then(|(path, p)| {
                p.inputs
                    .iter()
                    .find(|r| {
                        r.source == read.source && r.purpose == read.purpose && r.hash.is_some()
                    })
                    .map(|r| (path, r, "prior capture"))
            })
            .or_else(|| {
                (read.purpose == "schema"
                    && provider
                        .evidence
                        .as_ref()
                        .is_some_and(|e| e.intent == Intent::VendorCopy))
                .then_some(provider.asset.as_ref())
                .flatten()
                .map(|asset| (run, asset, "committed full-copy asset"))
            });
        let comparison = match (read.hash.as_ref(), basis) {
            (Some(_), Some((path, old, label))) if old.hash.is_some() => {
                json!({
                    "basis":label,
                    "previous_response":old.response,
                    "changes":diff(&document(&bytes(path,old)?),&document(&bytes(run,read)?)),
                })
            }
            (Some(_), _) => json!({"basis":"baseline established; no historical upstream capture"}),
            _ => json!({"basis":"source unavailable","outcome":read.outcome}),
        };
        let projection = match (
            multiple_schemas,
            read.purpose.as_str(),
            read.hash.as_ref(),
            provider.config.as_ref(),
        ) {
            (true, "schema", Some(_), Some(cfg)) => Some(projected(cfg, &bytes(run, read)?)),
            _ => None,
        };
        sources.push(json!({"source":read.source,"purpose":read.purpose,"response":read.response,"comparison":comparison,"projection":projection}));
    }

    let removed_sources:Vec<_>=prior.into_iter().flat_map(|(_,p)|p.inputs.iter()).filter(|old|old.purpose!="pricing"&&!provider.inputs.iter().any(|new|new.source==old.source && new.purpose==old.purpose)).map(|read|json!({"source":read.source,"purpose":read.purpose,"outcome":"source declaration removed; review authority change"})).collect();
    // Runtime history and authority history answer different questions. An authored
    // asset is runtime evidence, never a substitute for historical upstream input.
    let runtime = projected_input(run, provider, runtime_input(provider))?;
    let upstream = projected_input(run, provider, current_schema)?;
    let previous_runtime = prior
        .map(|(path, p)| projected_input(path, p, runtime_input(p)))
        .transpose()?
        .unwrap_or(Value::Null);
    let previous_upstream = prior
        .map(|(path, p)| projected_input(path, p, schema(p)))
        .transpose()?
        .unwrap_or(Value::Null);
    let (previous, tool_basis) = if !previous_upstream.is_null() {
        (previous_upstream, "prior capture")
    } else if provider
        .evidence
        .as_ref()
        .is_some_and(|e| e.intent == Intent::VendorCopy)
        && provider.asset.is_some()
    {
        (runtime.clone(), "committed full-copy asset")
    } else {
        (Value::Null, "no historical upstream capture")
    };
    let tool_changes = projection_changes(&previous, &upstream);
    let runtime_changes = projection_changes(&previous_runtime, &runtime);
    let asset_changes = prior
        .map(|(path, p)| -> Result<_> {
            Ok(diff(
                &asset_document(path, p)?,
                &asset_document(run, provider)?,
            ))
        })
        .transpose()?;
    let pricing = pricing_section(run, provider, prior, &runtime);
    let nonempty = |value: &Value| value.as_array().is_some_and(|items| !items.is_empty());
    let upstream_source_changed = sources.iter().any(|source| {
        !matches!(
            source["purpose"].as_str().unwrap_or(""),
            "provider TOML" | "maintenance guidance"
        ) && nonempty(&source["comparison"]["changes"])
    });
    let relocation = &pricing["relocation"];
    let pricing_changed = pricing["evidence"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| nonempty(&item["comparison"]["changes"]))
    }) || relocation["catalog_key_absent"] == true
        || relocation["source_changed"] == true
        || nonempty(&relocation["removed_sources"]);
    // Local provider TOML, override and maintenance-guidance edits surface in
    // runtime/tool projections; only source, pricing or committed-asset evidence
    // establishes upstream drift for this outcome label.
    let changed = upstream_source_changed
        || pricing_changed
        || asset_changes
            .as_ref()
            .is_some_and(|changes| !changes.is_empty())
        || !removed_sources.is_empty();
    let outcome = if provider.config.is_none() {
        provider.outcome.as_str()
    } else if provider.inputs.iter().any(|r| r.hash.is_none()) {
        "source unavailable"
    } else if provider.asset.is_some() && provider.evidence.is_none() {
        "missing maintenance metadata; authored review needed"
    } else if provider.asset.is_some() && previous.is_null() {
        "authored baseline established; manual consistency review needed"
    } else if provider.asset.is_some() && !changed {
        "authored no upstream evidence change"
    } else if provider.asset.is_some() {
        "authored comparison ready; human review pending"
    } else if previous.is_null() {
        "baseline established; current consistency review needed"
    } else if !changed {
        "no upstream evidence change"
    } else {
        "comparison ready; human review pending"
    };
    Ok(
        json!({"provider":provider.id,"outcome":outcome,"accepted":false,"sources":sources,"removed_sources":removed_sources,"runtime":runtime,"previous_runtime":previous_runtime,"runtime_changes":runtime_changes,"asset_changes":asset_changes,"pricing":pricing,"upstream":upstream,"previous_upstream":previous,"tool_changes":tool_changes,"tool_basis":tool_basis,"guidance":provider.evidence.as_ref().map(|e| &e.guidance)}),
    )
}
pub(super) fn review(run: &Path, prior: Option<&Path>) -> Result<()> {
    let report = replay(run)?;
    let previous = prior.map(replay).transpose()?;
    let mut packets = vec![];
    for (index, provider) in report.providers.iter().enumerate() {
        let old = previous
            .as_ref()
            .and_then(|r| r.providers.iter().find(|p| p.id == provider.id))
            .zip(prior)
            .map(|(provider, path)| (path, provider));
        let reviewed = (|| -> Result<Value> {
            // Numbered candidates are regenerated outputs, never capture evidence.
            match std::fs::remove_file(run.join(format!("candidate-{index}.json"))) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            for (root, p) in std::iter::once((run, provider)).chain(old) {
                for read in p
                    .inputs
                    .iter()
                    .chain(p.asset.iter())
                    .filter(|r| r.hash.is_some())
                {
                    bytes(root, read)?;
                }
            }
            let mut packet = compare_provider(run, provider, old)?;
            packet["candidate"] = candidates::stage(run, provider, index)?;
            if packet["candidate"]["file"].is_string() {
                packet["outcome"] = json!("candidate ready; human review pending");
            } else if packet["candidate"]["outcome"] == "committed asset unchanged; no candidate" {
                packet["outcome"] = json!("committed asset unchanged; no candidate");
            }
            Ok(packet)
        })();
        let packet=reviewed.unwrap_or_else(|_|json!({"provider":provider.id,"outcome":"review unavailable: input validation or candidate staging failed","accepted":false}));
        let filename = format!("review-{index}.json");
        std::fs::write(run.join(&filename), serde_json::to_vec_pretty(&packet)?)?;
        packets.push(json!({"provider":provider.id,"outcome":packet["outcome"],"packet":filename}));
    }
    std::fs::write(
        run.join("review.json"),
        serde_json::to_vec_pretty(
            &json!({"selected":report.selected.len(),"attempted":packets.len(),"accepted":0,"providers":packets}),
        )?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn cfg() -> Config {
        Config {
            spec: "https://example.com/openapi.json".into(),
            prefix: Some("demo".into()),
            base_url: Some("https://example.com".into()),
            ..Default::default()
        }
    }
    fn spec() -> Value {
        json!({"openapi":"3.0.0","paths":{"/one":{"get":{"description":"old prose","parameters":[{"name":"q","in":"query","schema":{"type":"string"}}],"responses":{}}}}})
    }
    #[test]
    fn masked_prose_removed_notes_and_help_remain_visible() {
        let mut cfg = cfg();
        cfg.help_url = Some("https://example.com/guide".into());
        let old = spec();
        let base = projection(&cfg, &old).unwrap();
        let name = base["effective"]["GET /one"]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        cfg.overrides = json!({name.clone():{"description":"authored","params":{"q":"query hint"}},"demo_help":{"description":"help override"}});
        let before = projection(&cfg, &old).unwrap();
        let mut new = old.clone();
        new["paths"]["/one"]["get"]["description"] = json!("new charged-error prose");
        new["paths"]["/one"]["get"]["parameters"] = json!([]);
        let after = projection(&cfg, &new).unwrap();
        assert_eq!(
            before["effective"]["GET /one"]["description"],
            after["effective"]["GET /one"]["description"]
        );
        assert!(!diff(&before["unoverridden"], &after["unoverridden"]).is_empty());
        assert!(
            after["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["missing_parameter_notes"] == json!(["q"]))
        );
        assert!(
            after["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == "demo_help" && v["missing_tool"] == false)
        );
        new["paths"] = json!({"/surviving":{"get":{"responses":{}}}});
        assert!(
            projection(&cfg, &new).unwrap()["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == name && v["missing_tool"] == true)
        );
    }
    #[test]
    fn parameter_notes_follow_production_whole_body_targets() {
        let mut cfg = cfg();
        let document = json!({"openapi":"3.1.0","paths":{"/one":{"post":{
            "parameters":[
                {"name":"id","in":"query","schema":{"type":"string"}},
                {"name":"body","in":"query","schema":{"type":"string"}}
            ],
            "requestBody":{"required":false,"content":{"application/json":{"schema":{
                "type":"object","additionalProperties":false,"properties":{
                    "id":{"type":"string"},"choice":{"type":"string"},
                    "choice_body":{"type":"string"},"a/b~c":{"type":"string"},"flag":false
                }
            }}}}
        }}}});
        let tool = catalog::build_tools(&cfg, &document, "demo")
            .unwrap()
            .remove(0);
        assert_eq!(
            tool.whole_body.as_ref().map(|body| body.name.as_str()),
            Some("body_body")
        );
        let name = tool.name.clone();
        cfg.overrides = json!({name:{"params":{
            "id":"outer id","id_body":"nested id","choice_body":"exact spelling",
            "a/b~c":"escaped name","flag":"not an object","missing":"not present"
        }}});
        let view = projection(&cfg, &document).unwrap();
        assert_eq!(
            view["overrides"][0]["missing_parameter_notes"],
            json!(["flag", "missing"])
        );
        let properties = &view["effective"]["POST /one"]["input_schema"]["properties"];
        assert_eq!(properties["id"]["description"], "outer id");
        let body = &properties["body_body"]["properties"];
        assert_eq!(body["id"]["description"], "nested id");
        assert_eq!(body["choice_body"]["description"], "exact spelling");
        assert!(body["choice"].get("description").is_none());
        assert_eq!(body["a/b~c"]["description"], "escaped name");
        assert_eq!(body["flag"], false);
        assert!(body.get("missing").is_none());
    }

    #[test]
    fn raw_arrays_extensions_filtered_additions_and_method_names() {
        let mut cfg = cfg();
        cfg.exclude = vec!["/hidden".into()];
        let old = spec();
        let mut new = old.clone();
        new["paths"]["/hidden"] = json!({"post":{"description":"hidden","responses":{}}});
        new["paths"]["/one"]["post"] = json!({"description":"post twin","responses":{}});
        new["x-billing"] = json!({"required":["b","a"]});
        let a = projection(&cfg, &old).unwrap();
        let b = projection(&cfg, &new).unwrap();
        assert!(b["operations"].get("POST /hidden").is_some());
        assert!(b["effective"].get("POST /hidden").is_none());
        assert!(a["effective"].get("GET /one").is_some());
        assert!(b["effective"].get("GET /one").is_none());
        assert!(b["effective"].get("POST /one").is_some());
        assert!(!diff(&old, &new).is_empty());
        assert!(!diff(&json!(["a", "b"]), &json!(["b", "a"])).is_empty());
        assert_eq!(diff(&old, &new), diff(&old, &new));
    }
    fn captured(run: &Path, source: &str, purpose: &str, raw: &[u8]) -> Read {
        Read {
            source: source.into(),
            purpose: purpose.into(),
            hash: Some(store(run, raw).unwrap()),
            outcome: "captured".into(),
            response: None,
        }
    }
    fn provider(run: &Path, raw: &[u8]) -> Provider {
        Provider {
            id: "synthetic".into(),
            path: "synthetic.toml".into(),
            config: Some(cfg()),
            inputs: vec![captured(
                run,
                "https://example.com/openapi.json",
                "schema",
                raw,
            )],
            asset: None,
            pricing: vec![],
            evidence: None,
            outcome: "captured".into(),
        }
    }
    #[test]
    fn authored_runtime_history_is_independent_of_upstream_history() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let raw = serde_json::to_vec(&spec()).unwrap();
        let mut old = provider(run, &raw);
        old.asset = Some(captured(run, "authored.json", "committed asset", &raw));
        old.evidence = Some(Evidence {
            intent: Intent::Authored,
            guidance: "guide".into(),
            sources: vec![],
        });
        let mut new = old.clone();
        let mut changed = spec();
        changed["paths"]["/one"]["get"]["parameters"][0]["required"] = json!(true);
        new.asset = Some(captured(
            run,
            "authored.json",
            "committed asset",
            &serde_json::to_vec(&changed).unwrap(),
        ));
        for authorities in 0..=2 {
            old.inputs.truncate(0);
            new.inputs.truncate(0);
            for index in 0..authorities {
                let read = captured(
                    run,
                    &format!("https://example.com/{index}.json"),
                    "schema",
                    &raw,
                );
                old.inputs.push(read.clone());
                new.inputs.push(read);
            }
            let packet = compare_provider(run, &new, Some((run, &old))).unwrap();
            assert!(!packet["runtime_changes"].as_array().unwrap().is_empty());
            assert!(
                packet["asset_changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|delta| delta["pointer"] == "/paths/~1one/get/parameters")
            );
            assert!(
                packet["previous_runtime"]["effective"]["GET /one"]["input_schema"]
                    .get("required")
                    .is_none()
            );
            assert_eq!(
                packet["runtime"]["effective"]["GET /one"]["input_schema"]["required"],
                json!(["q"])
            );
            if authorities == 1 {
                assert_eq!(packet["tool_changes"], json!([]));
            } else {
                assert!(packet["tool_changes"].is_null());
            }
            let first = compare_provider(run, &new, None).unwrap();
            assert!(first["runtime_changes"].is_null());
            assert!(first["asset_changes"].is_null());
            assert!(first["tool_changes"].is_null());
        }
    }
    #[test]
    fn failed_selection_retains_operations_and_missing_override_targets() {
        let mut cfg = cfg();
        cfg.include_tools = vec!["demo_one".into()];
        cfg.overrides = json!({"demo_one":{"description":"authored","params":{"q":"note"}}});
        let value = projection(&cfg, &json!({"paths":{}})).unwrap();
        assert!(value["effective"]["compiler_error"].is_string());
        assert_eq!(value["operations"], json!({}));
        assert_eq!(value["overrides"][0]["missing_tool"], true);
        assert_eq!(
            value["overrides"][0]["missing_parameter_notes"],
            json!(["q"])
        );
    }
    #[test]
    fn frozen_comparison_baselines_pinned_changes_and_guide_only_drift() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let old_raw = serde_json::to_vec(&spec()).unwrap();
        let mut new_spec = spec();
        new_spec["paths"]["/one"]["get"]["description"] = json!("new price conditions");
        let mut old = provider(run, &old_raw);
        old.inputs.push(captured(
            run,
            "https://example.com/guide",
            "help",
            b"old guide",
        ));
        let mut new = provider(run, &serde_json::to_vec(&new_spec).unwrap());
        new.inputs.push(captured(
            run,
            "https://example.com/guide",
            "help",
            b"new guide",
        ));
        let first = compare_provider(run, &new, None).unwrap();
        assert!(first["tool_changes"].is_null());
        assert_eq!(
            first["outcome"],
            "baseline established; current consistency review needed"
        );
        let packet = compare_provider(run, &new, Some((run, &old))).unwrap();
        assert_eq!(
            packet,
            compare_provider(run, &new, Some((run, &old))).unwrap()
        );
        assert!(!packet["tool_changes"].as_array().unwrap().is_empty());
        assert!(
            !packet["sources"][1]["comparison"]["changes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        old.inputs[0].hash = None;
        assert!(compare_provider(run, &new, Some((run, &old))).unwrap()["tool_changes"].is_null());
        new.asset = Some(captured(run, "pinned.json", "committed asset", &old_raw));
        new.evidence = Some(Evidence {
            intent: Intent::VendorCopy,
            guidance: "guide".into(),
            sources: vec![],
        });
        let pinned = compare_provider(run, &new, None).unwrap();
        assert_eq!(pinned["tool_basis"], "committed full-copy asset");
        assert!(!pinned["tool_changes"].as_array().unwrap().is_empty());
    }
    #[test]
    fn unchanged_second_capture_is_not_review_ready() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("new");
        let prior = dir.path().join("old");
        std::fs::create_dir(&run).unwrap();
        std::fs::create_dir(&prior).unwrap();
        let raw = serde_json::to_vec(&spec()).unwrap();
        for root in [&run, &prior] {
            let mut provider = provider(root, &raw);
            if *root == run {
                provider.config.as_mut().unwrap().prefix = Some("local".into());
            }
            if *root == prior {
                provider.inputs[0].response = Some(ResponseMeta {
                    etag: Some("\"v1\"".into()),
                    last_modified: None,
                    date: None,
                });
            }
            let report = Run {
                format: 1,
                version: "test".into(),
                tool_hash: "test".into(),
                baseline: "test".into(),
                dirty: String::new(),
                network: json!({}),
                metadata: captured(root, "metadata", "test", b"frozen metadata"),
                selected: BTreeMap::new(),
                providers: vec![provider],
            };
            std::fs::write(
                root.join("capture.json"),
                serde_json::to_vec(&report).unwrap(),
            )
            .unwrap();
        }
        review(&run, Some(&prior)).unwrap();
        let summary: Value =
            serde_json::from_slice(&std::fs::read(run.join("review.json")).unwrap()).unwrap();
        assert_eq!(
            summary["providers"][0]["outcome"],
            "no upstream evidence change"
        );
        let packet: Value =
            serde_json::from_slice(&std::fs::read(run.join("review-0.json")).unwrap()).unwrap();
        assert!(
            !packet["runtime_changes"].as_array().unwrap().is_empty(),
            "local config differences stay visible without becoming upstream drift"
        );
        let packet: Value =
            serde_json::from_slice(&std::fs::read(run.join("review-0.json")).unwrap()).unwrap();
        assert_eq!(
            packet["sources"][0]["comparison"]["previous_response"]["etag"],
            "\"v1\""
        );
    }
    #[test]
    fn replay_packets_keep_failed_siblings_and_are_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let mut valid = provider(run, &serde_json::to_vec(&spec()).unwrap());
        valid.pricing = vec![captured(
            run,
            "https://example.com/pricing",
            "pricing",
            b"{\"resources\":[{\"maxAmountRequired\":\"10000\"}]}",
        )];
        let mut failed = valid.clone();
        failed.id = "failed".into();
        failed.inputs[0].hash = None;
        failed.inputs[0].outcome = "source unavailable".into();
        let report = Run {
            format: 1,
            version: "test".into(),
            tool_hash: "test".into(),
            baseline: "test".into(),
            dirty: String::new(),
            network: json!({}),
            metadata: captured(run, "metadata", "test", b"frozen metadata"),
            selected: BTreeMap::new(),
            providers: vec![failed, valid],
        };
        let mut legacy = serde_json::to_value(&report).unwrap();
        for provider in legacy["providers"].as_array_mut().unwrap() {
            provider.as_object_mut().unwrap().remove("pricing");
        }
        let decoded: Run = serde_json::from_value(legacy).unwrap();
        assert!(
            decoded
                .providers
                .iter()
                .all(|provider| provider.pricing.is_empty())
        );
        std::fs::write(
            run.join("capture.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        review(run, None).unwrap();
        let first = std::fs::read(run.join("review.json")).unwrap();
        let packet = std::fs::read(run.join("review-1.json")).unwrap();
        review(run, None).unwrap();
        assert_eq!(first, std::fs::read(run.join("review.json")).unwrap());
        assert_eq!(packet, std::fs::read(run.join("review-1.json")).unwrap());
        let summary: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(summary["attempted"], 2);
        assert_eq!(summary["accepted"], 0);
        assert_eq!(summary["providers"][0]["outcome"], "source unavailable");
        assert_eq!(
            summary["providers"][1]["outcome"],
            "baseline established; current consistency review needed"
        );
    }
    #[test]
    fn multiple_schema_projections_and_removed_authority_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let raw = serde_json::to_vec(&spec()).unwrap();
        let mut old = provider(run, &raw);
        old.inputs.push(captured(
            run,
            "https://example.com/registry",
            "registry",
            b"old registry",
        ));
        let mut new = provider(run, &raw);
        let mut second = spec();
        second["paths"] = json!({"/second":{"post":{"responses":{}}}});
        new.inputs.push(captured(
            run,
            "https://example.com/second.json",
            "schema",
            &serde_json::to_vec(&second).unwrap(),
        ));
        new.asset = Some(captured(run, "authored.json", "committed asset", &raw));
        let packet = compare_provider(run, &new, Some((run, &old))).unwrap();
        assert_eq!(
            packet["removed_sources"][0]["source"],
            "https://example.com/registry"
        );
        assert!(
            packet["sources"][0]["projection"]["operations"]
                .get("GET /one")
                .is_some()
        );
        assert!(
            packet["sources"][1]["projection"]["operations"]
                .get("POST /second")
                .is_some()
        );
        assert!(packet["upstream"].is_null());
        assert!(packet["tool_changes"].is_null());
        new.inputs.truncate(1);
        new.inputs[0].hash = None;
        assert!(compare_provider(run, &new, Some((run, &old))).unwrap()["tool_changes"].is_null());
    }
    #[test]
    fn catalog_pricing_coverage_and_signals_are_advisory() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let mut schema = spec();
        schema["paths"]["/one"]["get"]["x-payment-info"] =
            json!({"amount":"0.01","currency":"USD"});
        let raw = serde_json::to_vec(&schema).unwrap();
        let mut cfg = cfg();
        cfg.pricing_key = Some("x-payment-info".into());
        let mut new = provider(run, &raw);
        new.config = Some(cfg.clone());
        let packet = compare_provider(run, &new, None).unwrap();
        assert_eq!(packet["pricing"]["catalog_key"], "x-payment-info");
        assert_eq!(
            packet["runtime"]["pricing"]["pricing_metadata_operations"],
            1
        );
        assert_eq!(packet["runtime"]["pricing"]["total_operations"], 1);
        assert_eq!(packet["pricing"]["relocation"]["catalog_key_absent"], false);
        assert_eq!(
            packet["pricing"]["signals"]["sources"][0]["pointers"],
            json!(["/paths/~1one/get/x-payment-info"])
        );
        // A configured key that no longer appears is a relocation candidate, not
        // proof that the operation is free.
        let mut moved = cfg;
        moved.pricing_key = Some("x-new-price".into());
        new.config = Some(moved);
        let relocated = compare_provider(run, &new, None).unwrap();
        assert_eq!(
            relocated["runtime"]["pricing"]["pricing_metadata_operations"],
            0
        );
        assert_eq!(
            relocated["pricing"]["relocation"]["catalog_key_absent"],
            true
        );
        assert!(
            relocated["pricing"]["signals"]["sources"][0]["pointers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|pointer| pointer.as_str().unwrap().contains("x-payment-info"))
        );
    }

    #[tokio::test]
    async fn external_pricing_source_relocation_diff_and_corrupt_bytes_stay_advisory() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let resolve = |source: &str| {
            let toml = format!(
                "spec='https://example.com/openapi.json'\nprefix='demo'\n[pricing]\nsource='{source}'\nformat='x402-resources'\n"
            );
            x402_treazury::config::resolve(toml::from_str(&toml).unwrap(), Path::new("demo.toml"))
        };
        let prior_cfg = resolve("https://example.com/pricing")
            .await
            .unwrap()
            .settings;
        let cfg = resolve("https://example.com/pricing-new")
            .await
            .unwrap()
            .settings;
        let raw = serde_json::to_vec(&spec()).unwrap();
        let mut prior = provider(run, &raw);
        prior.config = Some(prior_cfg);
        prior.pricing = vec![captured(
            run,
            "https://example.com/pricing",
            "pricing",
            b"{\"x402Version\":2,\"resources\":[{\"maxAmountRequired\":\"10000\"}]}",
        )];
        let mut new = provider(run, &raw);
        new.config = Some(cfg);
        new.pricing = vec![captured(
            run,
            "https://example.com/pricing-new",
            "pricing",
            b"{\"x402Version\":2,\"resources\":[{\"maxAmountRequired\":\"20000\"}]}",
        )];
        let packet = compare_provider(run, &new, Some((run, &prior))).unwrap();
        assert_eq!(
            packet["pricing"]["relocation"]["previous_external_source"],
            "https://example.com/pricing"
        );
        assert_eq!(packet["pricing"]["relocation"]["source_changed"], true);
        assert_eq!(
            packet["pricing"]["relocation"]["removed_sources"][0]["source"],
            "https://example.com/pricing"
        );
        assert_eq!(packet["pricing"]["evidence"][0]["captured"], true);
        assert_eq!(
            packet["pricing"]["evidence"][0]["shape"],
            json!(["resources", "x402Version"])
        );
        // The configured document's own bytes are compared when its URL is stable.
        let mut same = new.clone();
        same.config = Some(
            resolve("https://example.com/pricing")
                .await
                .unwrap()
                .settings,
        );
        same.pricing[0].source = "https://example.com/pricing".into();
        same.pricing[0].hash = Some(
            store(
                run,
                b"{\"x402Version\":2,\"resources\":[{\"maxAmountRequired\":\"30000\"}]}",
            )
            .unwrap(),
        );
        let packet = compare_provider(run, &same, Some((run, &prior))).unwrap();
        assert_eq!(packet["pricing"]["relocation"]["source_changed"], false);
        assert!(
            !packet["pricing"]["evidence"][0]["comparison"]["changes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // A missing or corrupt pricing blob must not hide the rest of the packet.
        for corrupt in [false, true] {
            let digest = store(run, b"{\"x402Version\":2,\"resources\":[]}").unwrap();
            if corrupt {
                std::fs::write(run.join(&digest), "corrupt").unwrap();
            } else {
                std::fs::remove_file(run.join(&digest)).unwrap();
            }
            same.pricing[0].hash = Some(digest);
            let packet = compare_provider(run, &same, Some((run, &prior))).unwrap();
            assert_eq!(
                packet["pricing"]["evidence"][0]["outcome"],
                "captured bytes unavailable"
            );
            assert!(packet["runtime"]["effective"].is_object());
        }
    }
    #[test]
    fn pricing_signal_pointers_escape_and_bound() {
        let (pointers, truncated) = signal_pointers(&json!({"a/~price":{"cost":1}}));
        assert!(pointers.contains(&"/a~1~0price".to_string()));
        assert!(!truncated);
        let mut keys = serde_json::Map::new();
        for index in 0..PRICING_SIGNAL_LIMIT + 5 {
            keys.insert(format!("cost{index}"), json!(1));
        }
        let (pointers, truncated) = signal_pointers(&Value::Object(keys));
        assert_eq!(pointers.len(), PRICING_SIGNAL_LIMIT);
        assert!(truncated);
    }

    #[test]
    fn failed_prior_capture_uses_complete_pinned_raw_basis() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let original = spec();
        let raw = serde_json::to_vec(&original).unwrap();
        let mut old = provider(run, &raw);
        old.inputs[0].hash = None;
        let mut changed = original.clone();
        changed["paths"]["/one"]["get"]["responses"] =
            json!({"200":{"description":"new response authority"}});
        let mut new = provider(run, &serde_json::to_vec(&changed).unwrap());
        new.asset = Some(captured(run, "pinned.json", "committed asset", &raw));
        new.evidence = Some(Evidence {
            intent: Intent::VendorCopy,
            guidance: "guide".into(),
            sources: vec![],
        });
        let packet = compare_provider(run, &new, Some((run, &old))).unwrap();
        assert_eq!(
            packet["sources"][0]["comparison"]["basis"],
            "committed full-copy asset"
        );
        assert!(
            !packet["sources"][0]["comparison"]["changes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            diff(
                &packet["runtime"]["effective"],
                &packet["upstream"]["effective"]
            )
            .is_empty()
        );
    }
    #[test]
    fn missing_or_corrupt_provider_blob_does_not_block_healthy_candidate() {
        for missing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let run = dir.path();
            std::fs::write(
                run.join("unrelated-evidence.txt"),
                b"preserve unrelated evidence",
            )
            .unwrap();
            let raw = serde_json::to_vec(&spec()).unwrap();
            let mut healthy = provider(run, &raw);
            healthy.asset = Some(captured(
                run,
                "asset.json",
                "committed asset",
                br#"{"openapi":"3.0.0","paths":{},"x-committed":"older"}"#,
            ));
            healthy.evidence = Some(Evidence {
                intent: Intent::VendorCopy,
                guidance: "guide".into(),
                sources: vec![],
            });
            let mut bad = healthy.clone();
            bad.id = "corrupt".into();
            bad.inputs.push(captured(
                run,
                "https://example.com/bad-guide",
                "help",
                b"corrupt only this provider",
            ));
            let blob = run.join(bad.inputs.last().unwrap().hash.as_ref().unwrap());
            if missing {
                std::fs::remove_file(blob).unwrap();
            } else {
                std::fs::write(blob, "corrupt").unwrap();
            }
            let report = Run {
                format: 1,
                version: "test".into(),
                tool_hash: "test".into(),
                baseline: "test".into(),
                dirty: String::new(),
                network: json!({}),
                metadata: captured(run, "metadata", "test", b"valid metadata"),
                selected: BTreeMap::new(),
                providers: vec![bad, healthy],
            };
            std::fs::write(
                run.join("capture.json"),
                serde_json::to_vec(&report).unwrap(),
            )
            .unwrap();
            review(run, None).unwrap();
            let summary: Value =
                serde_json::from_slice(&std::fs::read(run.join("review.json")).unwrap()).unwrap();
            assert_eq!(summary["attempted"], 2);
            assert_eq!(
                summary["providers"][0]["outcome"],
                "review unavailable: input validation or candidate staging failed"
            );
            assert_eq!(
                summary["providers"][1]["outcome"],
                "candidate ready; human review pending"
            );
            assert!(!run.join("candidate-0.json").exists());
            assert_eq!(
                std::fs::read(run.join("unrelated-evidence.txt")).unwrap(),
                b"preserve unrelated evidence"
            );
            assert_eq!(std::fs::read(run.join("candidate-1.json")).unwrap(), raw);
            // A corrupt old sibling must remain inside the same provider boundary.
            let prior = run.join("prior");
            std::fs::create_dir(&prior).unwrap();
            let mut prior_report = report;
            prior_report.metadata = captured(&prior, "metadata", "test", b"valid metadata");
            prior_report.providers = vec![prior_report.providers.remove(0)];
            std::fs::write(
                prior.join("capture.json"),
                serde_json::to_vec(&prior_report).unwrap(),
            )
            .unwrap();
            store(run, b"corrupt only this provider").unwrap();
            review(run, None).unwrap();
            review(run, Some(&prior)).unwrap();
            let prior_summary: Value =
                serde_json::from_slice(&std::fs::read(run.join("review.json")).unwrap()).unwrap();
            assert_eq!(
                prior_summary["providers"][0]["outcome"],
                "review unavailable: input validation or candidate staging failed"
            );
            assert_eq!(
                prior_summary["providers"][1]["outcome"],
                "candidate ready; human review pending"
            );
            assert!(!run.join("candidate-0.json").exists());
            assert_eq!(
                std::fs::read(run.join("unrelated-evidence.txt")).unwrap(),
                b"preserve unrelated evidence"
            );
            assert_eq!(std::fs::read(run.join("candidate-1.json")).unwrap(), raw);
        }
    }
}
