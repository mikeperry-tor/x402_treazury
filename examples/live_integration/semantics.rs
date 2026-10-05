//! Explicit provider-content assertions, separate from MCP delivery and settlement.
use crate::{
    manifest::{Manifest, Scenario},
    registry::Registry,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    JsonEquals { pointer: String, value: Value },
    JsonExists { pointer: String },
    TextContains { text: String },
}
pub fn validate(checks: &[Check]) -> Result<()> {
    ensure!(
        checks.len() <= 32,
        "provider checks exceed 32-assertion limit; no checks dropped"
    );
    for check in checks {
        ensure!(
            serde_json::to_vec(check)?.len() <= 4096,
            "provider check exceeds 4096-byte limit; no truncation permitted"
        );
        match check {
            Check::JsonEquals { pointer, .. } | Check::JsonExists { pointer } => ensure!(
                (pointer.is_empty() || pointer.starts_with('/'))
                    && pointer
                        .split('~')
                        .skip(1)
                        .all(|part| part.starts_with('0') || part.starts_with('1')),
                "invalid provider-check JSON pointer"
            ),
            Check::TextContains { text } => {
                ensure!(!text.is_empty(), "text assertion must not be empty")
            }
        }
    }
    Ok(())
}
fn outcome(status: &str, category: &str) -> Value {
    json!({"status":status,"category":category})
}
fn nested_error(values: &[Value]) -> Result<bool> {
    let mut stack: Vec<_> = values.iter().map(|v| (v, 0)).collect();
    let mut nodes = 0usize;
    let mut failed = false;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        ensure!(
            nodes <= 100000 && depth <= 64,
            "provider semantic scan exceeds 100000 nodes or 64 levels; no partial classification"
        );
        match value {
            Value::Object(map) => {
                failed |= map.get("isError") == Some(&Value::Bool(true));
                ensure!(
                    nodes + stack.len() + map.len() <= 100000,
                    "provider semantic scan exceeds 100000 nodes; no partial classification"
                );
                stack.extend(map.values().map(|v| (v, depth + 1)));
            }
            Value::Array(items) => {
                ensure!(
                    nodes + stack.len() + items.len() <= 100000,
                    "provider semantic scan exceeds 100000 nodes; no partial classification"
                );
                stack.extend(items.iter().map(|v| (v, depth + 1)));
            }
            _ => (),
        }
    }
    Ok(failed)
}
pub fn evaluate(response: &Value, checks: &[Check]) -> Result<Value> {
    validate(checks)?;
    let Some(result) = response.get("result").filter(|v| v.is_object()) else {
        return Ok(outcome("manual_review_required", "missing_mcp_result"));
    };
    if result["isError"] == true {
        return Ok(outcome("failed", "mcp_is_error"));
    }
    if result.get("isError").is_some_and(|v| !v.is_boolean()) {
        return Ok(outcome(
            "manual_review_required",
            "malformed_mcp_error_flag",
        ));
    }
    let Some(content) = result["content"].as_array() else {
        return Ok(outcome("manual_review_required", "unsupported_mcp_content"));
    };
    if content.len() > 1000 {
        return Ok(outcome(
            "manual_review_required",
            "content_block_limit_1000_exceeded",
        ));
    }
    let mut json_values = Vec::new();
    let mut texts = Vec::new();
    if let Some(value) = result.get("structuredContent") {
        json_values.push(value.clone());
    }
    for block in content {
        if block["type"] == "text" {
            if let Some(text) = block["text"].as_str() {
                texts.push(text);
                if let Ok(value) = serde_json::from_str::<Value>(text)
                    && !json_values.contains(&value)
                {
                    json_values.push(value);
                }
            } else {
                return Ok(outcome("manual_review_required", "malformed_text_content"));
            }
        }
    }
    match nested_error(&json_values) {
        Ok(true) => return Ok(outcome("failed", "nested_is_error")),
        Err(_) => {
            return Ok(outcome(
                "manual_review_required",
                "semantic_scan_limit_100000_nodes_64_levels_exceeded",
            ));
        }
        _ => (),
    }
    if checks.is_empty() {
        return Ok(outcome(
            "manual_review_required",
            "no_declared_provider_assertions",
        ));
    }
    let mut results = Vec::new();
    for (index, check) in checks.iter().enumerate() {
        let tested = match check {
            Check::JsonEquals { pointer, value } => {
                (json_values.len() == 1).then(|| json_values[0].pointer(pointer) == Some(value))
            }
            Check::JsonExists { pointer } => {
                (json_values.len() == 1).then(|| json_values[0].pointer(pointer).is_some())
            }
            Check::TextContains { text } => {
                (!texts.is_empty()).then(|| texts.iter().any(|actual| actual.contains(text)))
            }
        };
        results.push(json!({"index":index,"status":match tested {Some(true)=>"passed",Some(false)=>"failed",None=>"manual_review_required"}}));
    }
    let status = if results.iter().any(|r| r["status"] == "failed") {
        "failed"
    } else if results
        .iter()
        .any(|r| r["status"] == "manual_review_required")
    {
        "manual_review_required"
    } else {
        "passed"
    };
    Ok(json!({"status":status,"category":"declared_assertions","checks":results}))
}
pub fn report(registry: &Registry, manifest: &Manifest, report: &Value) -> Result<Value> {
    let cases = report["cases"]
        .as_array()
        .context("semantic case report missing")?;
    let mut results = Vec::new();
    for case in &manifest.cases {
        let recorded = cases
            .iter()
            .find(|r| r["case"] == case.id)
            .context("semantic case missing")?;
        let phase = manifest
            .phases
            .iter()
            .find(|p| p.cases.contains(&case.id))
            .context("semantic phase missing")?;
        let skipped = recorded["execution"] == "SKIPPED_TARGET_REACHED";
        let mut evaluation = if skipped {
            outcome("not_executed", "validated_rotation_target_reached")
        } else if recorded["execution"] == "COMPLETED" {
            match registry.response(&manifest.run_id, &case.id) {
                Ok(body) => evaluate(&body, &case.checks)?,
                Err(_) => outcome("invalid", "missing_or_changed_response_evidence"),
            }
        } else {
            outcome("unobserved", "response_not_completed")
        };
        evaluation["case"] = json!(case.id);
        evaluation["required"] = json!(
            !skipped
                && (evaluation["status"] == "invalid"
                    || evaluation["category"] == "nested_is_error"
                    || !case.checks.is_empty()
                    || matches!(
                        phase.scenario,
                        Scenario::ProviderSweep {} | Scenario::Reliability { .. }
                    ))
        );
        results.push(evaluation);
    }
    Ok(Value::Array(results))
}

/// Missing/manual evidence is incomplete; an observed semantic rejection is a failure.
pub fn qualification(report: &Value, selected: Option<&[String]>) -> Result<(bool, bool)> {
    let rows = report["provider_semantics"]
        .as_array()
        .context("provider semantic report missing")?;
    let mut complete = true;
    let mut passed = true;
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        let id = row["case"]
            .as_str()
            .context("provider assessment case missing")?;
        ensure!(
            seen.insert(id) && row["required"].is_boolean(),
            "duplicate or malformed provider assessment"
        );
    }
    if let Some(ids) = selected {
        ensure!(
            ids.iter().all(|id| seen.contains(id.as_str())),
            "dependency provider assessment missing"
        );
    }
    for row in rows.iter().filter(|row| {
        row["required"] == true
            && selected.is_none_or(|ids| ids.iter().any(|id| row["case"] == *id))
    }) {
        complete &= matches!(row["status"].as_str(), Some("passed" | "failed"));
        passed &= row["status"] == "passed";
    }
    Ok((complete, passed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_manual_or_missing_assessments_cannot_qualify_or_satisfy_dependencies() {
        let mut report = json!({"provider_semantics":[
            {"case":"one","required":true,"status":"passed"},
            {"case":"two","required":true,"status":"manual_review_required"},
            {"case":"skipped","required":false,"status":"not_executed"}]});
        assert_eq!(qualification(&report, None).unwrap(), (false, false));
        assert_eq!(
            qualification(&report, Some(&["one".into()])).unwrap(),
            (true, true)
        );
        report["provider_semantics"][1]["status"] = json!("failed");
        assert_eq!(qualification(&report, None).unwrap(), (true, false));
        report["provider_semantics"][1]["status"] = json!("passed");
        assert_eq!(qualification(&report, None).unwrap(), (true, true));
        assert!(qualification(&json!({}), None).is_err());
        assert!(qualification(&report, Some(&["missing".into()])).is_err());
    }
    fn body(value: Value) -> Value {
        json!({"result":{"content":[{"type":"text","text":value.to_string()}],"isError":false}})
    }
    #[test]
    fn outer_delivery_cannot_hide_nested_failure_or_certify_undeclared_semantics() {
        assert_eq!(
            evaluate(&body(json!({"data":{"isError":true}})), &[]).unwrap()["category"],
            "nested_is_error"
        );
        assert_eq!(
            evaluate(&body(json!({"error":"ordinary data"})), &[]).unwrap()["status"],
            "manual_review_required"
        );
        let checks = [Check::JsonEquals {
            pointer: "/error".into(),
            value: json!("ordinary data"),
        }];
        assert_eq!(
            evaluate(&body(json!({"error":"ordinary data"})), &checks).unwrap()["status"],
            "passed"
        );
    }
    #[test]
    fn reviewed_pointers_and_text_checks_preserve_degraded_and_unsupported_outcomes() {
        let checks = [
            Check::JsonEquals {
                pointer: "/degraded".into(),
                value: json!(false),
            },
            Check::JsonExists {
                pointer: "/a~1b/~0".into(),
            },
        ];
        assert_eq!(
            evaluate(&body(json!({"degraded":false,"a/b":{"~":null}})), &checks).unwrap()["status"],
            "passed"
        );
        assert_eq!(
            evaluate(&body(json!({"degraded":true,"a/b":{"~":null}})), &checks).unwrap()["status"],
            "failed"
        );
        let text = json!({"result":{"content":[{"type":"text","text":"Usage: search"}]}});
        assert_eq!(
            evaluate(
                &text,
                &[Check::TextContains {
                    text: "Usage:".into()
                }]
            )
            .unwrap()["status"],
            "passed"
        );
        assert_eq!(
            evaluate(&text, &checks).unwrap()["status"],
            "manual_review_required"
        );
        let mut ambiguous = body(json!({"ok":true}));
        ambiguous["result"]["structuredContent"] = json!({"other":true});
        assert_eq!(
            evaluate(
                &ambiguous,
                &[Check::JsonExists {
                    pointer: "/ok".into()
                }]
            )
            .unwrap()["status"],
            "manual_review_required"
        );
    }
    #[test]
    fn malformed_and_oversized_checks_or_content_are_explicit_and_never_truncated() {
        assert!(
            validate(&[Check::JsonExists {
                pointer: "/bad~2".into()
            }])
            .is_err()
        );
        assert!(validate(&vec![Check::TextContains { text: "x".into() }; 33]).is_err());
        assert!(
            validate(&[Check::TextContains {
                text: "x".repeat(5000)
            }])
            .is_err()
        );
        let too_many = json!({"result":{"content":vec![json!({"type":"text","text":"{}"});1001]}});
        assert_eq!(
            evaluate(&too_many, &[]).unwrap()["category"],
            "content_block_limit_1000_exceeded"
        );
        let mut deep = Value::Null;
        for _ in 0..66 {
            deep = json!([deep]);
        }
        assert_eq!(
            evaluate(&body(deep), &[]).unwrap()["category"],
            "semantic_scan_limit_100000_nodes_64_levels_exceeded"
        );
        let mut plain = body(json!({"ok":true}));
        plain["result"]["isError"] = json!(true);
        assert_eq!(evaluate(&plain, &[]).unwrap()["category"], "mcp_is_error");
    }
}
