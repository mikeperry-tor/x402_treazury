//! One presentation model over the existing factual validators. Never grants authority.
use crate::manifest::{Manifest, TorMode};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Completed,
    Failed,
    Incomplete,
    NotAttempted,
    Unobserved,
    Disabled,
    NotRequested,
    Unknown,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Incomplete => "incomplete",
            Self::NotAttempted => "not attempted",
            Self::Unobserved => "unobserved",
            Self::Disabled => "disabled",
            Self::NotRequested => "not requested",
            Self::Unknown => "unknown",
        }
    }
}
fn state(value: &Value) -> State {
    match value.as_str() {
        Some("completed" | "COMPLETED" | "passed" | "PASSED" | "qualified") => State::Completed,
        Some("failed" | "FAILED") => State::Failed,
        Some(
            "incomplete"
            | "cancelled"
            | "PENDING"
            | "RESERVED"
            | "DISPATCHING"
            | "TRANSPORT_UNCERTAIN"
            | "manual_review_required"
            | "invalid"
            | "invalid_or_incomplete",
        ) => State::Incomplete,
        Some("UNATTEMPTED" | "not_started" | "not_executed" | "SKIPPED_TARGET_REACHED") => {
            State::NotAttempted
        }
        Some("not_observed" | "unobserved" | "UNOBSERVED") | None => State::Unobserved,
        Some("disabled") => State::Disabled,
        Some("not_required" | "not_requested") => State::NotRequested,
        _ => State::Unknown,
    }
}
#[derive(Serialize, Deserialize)]
pub struct Observation {
    pub scope: String,
    pub state: State,
    pub http_status: Option<u16>,
    pub pricing_evidence: Option<x402_treazury::pricing::Evidence>,
}
#[derive(Serialize, Deserialize)]
pub struct CaseOutcome {
    pub case: String,
    pub execution: State,
    pub mcp: State,
    pub semantics: State,
    pub payment: State,
    /// Only a canonical debit proof supplies an amount. Missing is never zero.
    pub verified_atomic: Option<u64>,
}
#[derive(Serialize, Deserialize)]
pub struct Provider {
    pub source: String,
    pub catalog: Vec<Observation>,
    pub help: Vec<Observation>,
    pub pricing: Vec<Observation>,
    pub cases: Vec<CaseOutcome>,
}
#[derive(Serialize, Deserialize)]
pub struct SourceFunding {
    pub saved_accounting: State,
    pub observations: usize,
    pub reserved_zatoshis: u64,
    pub reserved_jobs: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkEvidence {
    Direct,
    ExternalProxyOnly,
    OwnedControlAuditRequired,
}
#[derive(Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub source_inventory_complete: bool,
    pub providers: Vec<Provider>,
    pub source_funding: SourceFunding,
    pub lifecycle: Vec<Observation>,
    pub network: NetworkEvidence,
}
fn rows<'a>(r: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    r[key]
        .as_array()
        .with_context(|| format!("missing report section {key}"))
}
fn numeric(r: &Value, key: &str) -> Result<u64> {
    r[key]
        .as_u64()
        .with_context(|| format!("invalid report amount {key}"))
}
fn observation(scope: String, outcome: &Value, code: &Value) -> Result<Observation> {
    let http_status = if code.is_null() {
        None
    } else {
        let code = code.as_u64().context("invalid stage HTTP status")?;
        ensure!((100..=999).contains(&code), "invalid stage HTTP status");
        Some(code as u16)
    };
    Ok(Observation {
        scope,
        state: state(outcome),
        http_status,
        pricing_evidence: None,
    })
}
fn empty_observation(state: State) -> Vec<Observation> {
    vec![Observation {
        scope: "no_observation".into(),
        state,
        http_status: None,
        pricing_evidence: None,
    }]
}
pub fn build(m: &Manifest, config: &Value, r: &Value) -> Result<Report> {
    // Older config-only archives may lack the source inventory. Keep their known
    // case sources visible and explicitly avoid an exhaustive-inventory claim.
    let source_inventory_complete = config["sources"].is_object();
    let mut sources: BTreeSet<String> = config["sources"]
        .as_object()
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default();
    sources.extend(m.cases.iter().map(|c| c.source.clone()));
    ensure!(
        sources.len() <= 10000,
        "provider report exceeds 10000 sources; no partial report"
    );
    let cases: BTreeMap<_, _> = rows(r, "cases")?
        .iter()
        .map(|c| Ok((c["case"].as_str().context("case identity missing")?, c)))
        .collect::<Result<_>>()?;
    ensure!(
        cases.len() == m.cases.len(),
        "report case inventory mismatch"
    );
    let semantics = rows(r, "provider_semantics")?;
    let proofs = &r["canonical_api_debits"];
    let mut providers = Vec::new();
    for source in &sources {
        let mut catalog = Vec::new();
        for phase in rows(r, "catalog_stages")? {
            for row in rows(phase, "sources")?
                .iter()
                .filter(|c| c["source"] == *source)
            {
                catalog.push(observation(
                    phase["phase"]
                        .as_str()
                        .context("catalog phase missing")?
                        .into(),
                    &row["state"],
                    &row["http_status"],
                )?);
            }
        }
        if catalog.is_empty() {
            catalog = empty_observation(State::Unobserved);
        }
        let mut help = Vec::new();
        let mut outcomes = Vec::new();
        for case in m.cases.iter().filter(|c| c.source == *source) {
            let row = cases
                .get(case.id.as_str())
                .context("manifest case missing from report")?;
            let execution = state(&row["execution"]);
            if let Some(h) = rows(r, "help_stages")?
                .iter()
                .find(|h| h["case"] == case.id)
            {
                let mut observed =
                    observation(case.id.clone(), &h["status"], &h["result"]["http_status"])?;
                if execution == State::NotAttempted {
                    observed.state = State::NotAttempted;
                }
                help.push(observed);
            }
            let sem = semantics
                .iter()
                .find(|s| s["case"] == case.id)
                .context("provider semantic assessment missing")?;
            let amount = &proofs["cases"][&case.id];
            let verified_atomic = if proofs["status"] == "validated" && !amount.is_null() {
                Some(
                    amount
                        .as_str()
                        .map(str::parse)
                        .transpose()?
                        .or_else(|| amount.as_u64())
                        .context("invalid canonical debit amount")?,
                )
            } else {
                None
            };
            let payment = if verified_atomic.is_some() {
                State::Completed
            } else if case.unsigned {
                State::NotRequested
            } else if execution == State::NotAttempted {
                State::NotAttempted
            } else {
                State::Incomplete
            };
            outcomes.push(CaseOutcome {
                case: case.id.clone(),
                execution,
                mcp: state(&row["semantic"]),
                semantics: state(&sem["status"]),
                payment,
                verified_atomic,
            });
        }
        if help.is_empty() {
            let unknown = m.cases.iter().any(|c| {
                c.source == *source
                    && c.unsigned
                    && cases
                        .get(c.id.as_str())
                        .is_some_and(|r| r["execution"] != "UNATTEMPTED")
            });
            help = empty_observation(if unknown {
                State::Unobserved
            } else {
                State::NotAttempted
            });
        }
        let mut pricing = Vec::new();
        for row in rows(r, "pricing_stages")?
            .iter()
            .filter(|p| p["source"] == *source)
        {
            let mut observed = observation(
                row["session"].as_str().unwrap_or("no_session").into(),
                &row["stage"],
                &Value::Null,
            )?;
            if !row["evidence"].is_null() {
                let evidence: x402_treazury::pricing::Evidence =
                    serde_json::from_value(row["evidence"].clone())?;
                evidence.validate()?;
                observed.pricing_evidence = Some(evidence);
            }
            pricing.push(observed);
        }
        if pricing.is_empty() {
            pricing = empty_observation(State::Unobserved);
        }
        providers.push(Provider {
            source: source.clone(),
            catalog,
            help,
            pricing,
            cases: outcomes,
        });
    }
    let lifecycle = rows(r, "rotation")?
        .iter()
        .enumerate()
        .map(|(i, row)| observation(format!("rotation_{i}"), &row["status"], &Value::Null))
        .collect::<Result<_>>()?;
    let accounting = &r["treasury_accounting"];
    Ok(Report {
        version: 1,
        source_inventory_complete,
        providers,
        source_funding: SourceFunding {
            saved_accounting: if accounting["status"] == "recorded" {
                State::Completed
            } else {
                state(&accounting["status"])
            },
            observations: accounting["observations"].as_array().map_or(0, Vec::len),
            reserved_zatoshis: numeric(r, "source_reserved_zatoshis")?,
            reserved_jobs: numeric(r, "funding_jobs_reserved")?,
        },
        lifecycle,
        network: match m.network.tor_mode {
            TorMode::Direct => NetworkEvidence::Direct,
            TorMode::External => NetworkEvidence::ExternalProxyOnly,
            TorMode::Owned => NetworkEvidence::OwnedControlAuditRequired,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn provider_failure_payment_and_missing_stages_remain_independent() {
        let m = crate::tests::manifest();
        let config = json!({"sources":{"api":{},"unused":{}}});
        let mut r = json!({"cases":[{"case":"call","execution":"COMPLETED","semantic":"PASSED"}],
            "provider_semantics":[{"case":"call","status":"failed"}],
            "canonical_api_debits":{"status":"validated","cases":{"call":"14000"}},
            "catalog_stages":[{"phase":"initial_inspection","sources":[{"source":"api","state":"failed","http_status":429},{"source":"unused","state":"not_started"}]}],
            "help_stages":[],"pricing_stages":[{"source":"api","stage":"failed"}],"rotation":[],
            "treasury_accounting":{"status":"unobserved"},"source_reserved_zatoshis":20,"funding_jobs_reserved":1});
        let report = build(&m, &config, &r).unwrap();
        let p = &report.providers[0];
        assert_eq!(p.catalog[0].http_status, Some(429));
        assert_eq!(p.help[0].state, State::NotAttempted);
        assert_eq!(p.pricing[0].state, State::Failed);
        assert_eq!(p.cases[0].semantics, State::Failed);
        assert_eq!(p.cases[0].payment, State::Completed);
        assert_eq!(report.source_funding.saved_accounting, State::Unobserved);
        assert_eq!(report.providers[1].catalog[0].state, State::NotAttempted);
        assert!(report.providers[1].cases.is_empty());
        r["canonical_api_debits"]["cases"] = json!({});
        let report = build(&m, &config, &r).unwrap();
        assert_eq!(report.providers[0].cases[0].verified_atomic, None);
        assert_eq!(report.providers[0].cases[0].payment, State::Incomplete);
        r["cases"][0]["execution"] = json!("UNATTEMPTED");
        assert_eq!(
            build(&m, &config, &r).unwrap().providers[0].cases[0].payment,
            State::NotAttempted
        );
    }
}
