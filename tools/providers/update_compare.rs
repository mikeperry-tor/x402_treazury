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
    let operations = catalog::operations(root, cfg.pricing_key.as_deref())?
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
        .collect::<serde_json::Map<String, Value>>();
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
        let missing_params: Vec<_> = patch["params"].as_object().into_iter().flatten().filter(|(key,_)|tool.is_some_and(|t|t.input_schema["properties"].get(key).is_none()) || missing==Some(true)).map(|(key,_)|key).collect();
        json!({"name":name,"authored_override":patch,"missing_tool":missing,"missing_parameter_notes":missing_params,"unoverridden":tool,"effective":effective.as_ref().ok().and_then(|items|items.iter().find(|t|&t.name==name))})
    }).collect::<Vec<_>>();
    Ok(
        json!({"operations":operations,"effective":tool_view(cfg,&effective)?,"unoverridden":tool_view(cfg,&plain)?,"override_diagnostic_inventory":tool_view(cfg,&diagnostic)?,"overrides":diagnostics,"config":cfg}),
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
fn compare_provider(
    run: &Path,
    provider: &Provider,
    prior: Option<(&Path, &Provider)>,
) -> Result<Value> {
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
                json!({"basis":label,"changes":diff(&document(&bytes(path,old)?),&document(&bytes(run,read)?))})
            }
            (Some(_), _) => json!({"basis":"baseline established; no historical upstream capture"}),
            _ => json!({"basis":"source unavailable","outcome":read.outcome}),
        };
        sources.push(json!({"source":read.source,"purpose":read.purpose,"comparison":comparison}));
    }
    let current_schema = schema(provider).filter(|r| r.hash.is_some());

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
    let outcome = if provider.config.is_none() || provider.inputs.iter().any(|r| r.hash.is_none()) {
        "source unavailable"
    } else if provider.asset.is_some() && provider.evidence.is_none() {
        "missing maintenance metadata; authored review needed"
    } else if provider.asset.is_some() {
        "authored review needed"
    } else if previous.is_null() {
        "baseline established; current consistency review needed"
    } else {
        "comparison ready; human review pending"
    };
    Ok(
        json!({"provider":provider.id,"outcome":outcome,"accepted":false,"sources":sources,"runtime":runtime,"previous_runtime":previous_runtime,"runtime_changes":runtime_changes,"asset_changes":asset_changes,"upstream":upstream,"previous_upstream":previous,"tool_changes":tool_changes,"tool_basis":tool_basis,"guidance":provider.evidence.as_ref().map(|e| &e.guidance)}),
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
            &json!({"attempted":packets.len(),"accepted":0,"providers":packets}),
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
    fn replay_packets_keep_failed_siblings_and_are_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let valid = provider(run, &serde_json::to_vec(&spec()).unwrap());
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
            providers: vec![failed, valid],
        };
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
            healthy.asset = Some(captured(run, "asset.json", "committed asset", &raw));
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
