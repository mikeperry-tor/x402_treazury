//! Offline dispatch eligibility. This does not authorize or launch a paid session.
use crate::manifest::{Manifest, Scenario};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// `now` is injected so cross-window and clock-change behavior requires no sleeps.
/// Prior reservations are observed, never turned back into unattempted cases.
pub fn inspect(manifest: &Manifest, report: &Value, now: u64) -> Result<Value> {
    let rows = report["cases"].as_array().context("resume cases missing")?;
    let mut cases = BTreeMap::new();
    for row in rows {
        let id = row["case"].as_str().context("resume case ID missing")?;
        ensure!(cases.insert(id, row).is_none(), "duplicate resume case");
    }
    let declared: BTreeSet<_> = manifest.cases.iter().map(|c| c.id.as_str()).collect();
    ensure!(
        cases.keys().copied().collect::<BTreeSet<_>>() == declared,
        "resume case inventory differs from manifest"
    );
    let events = report["runtime_events"]
        .as_array()
        .context("resume runtime events missing")?;
    let mut first = None;
    for event in events {
        let at = event["at"]
            .as_u64()
            .context("resume event timestamp missing")?;
        ensure!(at <= now, "wall clock moved backwards; resume refused");
        if event["kind"] == "execution_started" {
            ensure!(
                first.replace(at).is_none(),
                "duplicate initial execution timestamp"
            );
        }
    }
    let end = first
        .map(|start| {
            start
                .checked_add(manifest.limits.run_seconds)
                .context("resume deadline overflow")
        })
        .transpose()?;
    let expired = end.is_some_and(|end| now >= end);
    let scheduled: Vec<_> = manifest
        .phases
        .iter()
        .flat_map(|p| p.cases.iter().map(String::as_str))
        .collect();
    ensure!(
        scheduled.len() == declared.len()
            && scheduled.iter().copied().collect::<BTreeSet<_>>() == declared,
        "resume phase case inventory differs from manifest"
    );
    // Validate financial state even when there is no eligible window. Missing proof
    // is not permission to use the balance again or to ignore an old liability.
    let proofs = &report["canonical_api_debits"];
    ensure!(
        proofs["status"] == "validated",
        "resume canonical accounting is invalid or incomplete"
    );
    let proofs = proofs["cases"]
        .as_object()
        .context("resume debit proofs missing")?;
    ensure!(
        proofs.keys().all(|id| declared.contains(id.as_str())),
        "resume debit names unknown case"
    );
    let mut observed = 0usize;
    for case in &manifest.cases {
        let row = cases[case.id.as_str()];
        let state = row["execution"]
            .as_str()
            .context("resume execution missing")?;
        match state {
            "UNATTEMPTED" => ensure!(
                row["settlement"] == "UNOBSERVED" && !proofs.contains_key(&case.id),
                "untouched case has payment evidence"
            ),
            "SKIPPED_TARGET_REACHED" => observed += 1, // Registry validates the immutable skip boundary.
            "COMPLETED" | "TRANSPORT_UNCERTAIN" => {
                observed += 1;
                match row["settlement"].as_str() {
                    Some("NOT_SIGNED" | "EXPIRED_UNUSED") => ensure!(
                        !proofs.contains_key(&case.id),
                        "unused payment has a debit proof"
                    ),
                    Some("USED") => ensure!(
                        !case.unsigned && proofs.contains_key(&case.id),
                        "used authorization lacks canonical debit proof"
                    ),
                    _ => anyhow::bail!(
                        "prior payment remains unresolved; observation/reconciliation only"
                    ),
                }
            }
            // A crash between reservation and dispatch must never imply no work.
            "RESERVED" | "DISPATCHING" => {
                anyhow::bail!("prior dispatch remains unresolved; observation/reconciliation only")
            }
            _ => anyhow::bail!("unknown resume execution state"),
        }
    }
    if observed > 0 {
        let evidence = &report["application_evidence"];
        ensure!(
            evidence["payment_attempts_correlated"].as_u64().is_some()
                && evidence["without_completion"]
                    .as_array()
                    .is_some_and(Vec::is_empty),
            "prior application identity/completion evidence is invalid or incomplete"
        );
    }
    let mut eligible = Vec::new();
    let mut waiting = Vec::new();
    let mut closed = Vec::new();
    for phase in &manifest.phases {
        let window = manifest
            .windows
            .iter()
            .find(|w| w.id == phase.window)
            .context("resume phase window missing")?;
        let untouched: Vec<_> = phase
            .cases
            .iter()
            .filter(|id| cases[id.as_str()]["execution"] == "UNATTEMPTED")
            .collect();
        if matches!(
            phase.scenario,
            Scenario::Rotation { .. } | Scenario::RefillService { .. } | Scenario::Lifecycle { .. }
        ) {
            ensure!(
                untouched.is_empty() || untouched.len() == phase.cases.len(),
                "partially dispatched rotation/lifecycle remains observation-only"
            );
        }
        for id in untouched {
            if expired || now >= window.not_after {
                closed.push(id);
            } else if now < window.not_before {
                waiting.push(id);
            } else {
                eligible.push(id);
            }
        }
    }
    Ok(
        json!({"eligible":eligible,"waiting_window":waiting,"expired_window":closed,
        "observe_only_count":observed,"original_deadline":end,
        "scope":"offline eligibility only; unchanged provenance, authority, clean prior processes, production state and supervised network qualification must still pass before dispatch"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> (Manifest, Value) {
        let mut m = crate::tests::manifest();
        m.limits.run_seconds = 200000;
        m.windows[0].not_before = 90000;
        m.windows[0].not_after = 100000;
        let rows: Vec<_> = m
            .cases
            .iter()
            .map(|c| json!({"case":c.id,"execution":"UNATTEMPTED","settlement":"UNOBSERVED"}))
            .collect();
        (
            m,
            json!({"cases":rows,"canonical_api_debits":{"status":"validated","cases":{}},
            "application_evidence":{"payment_attempts_correlated":0,"without_completion":[]},"runtime_events":[{"kind":"execution_started","at":1}]}),
        )
    }
    #[test]
    fn open_windows_are_half_open_and_never_reset_the_original_deadline() {
        let (m, r) = input();
        assert_eq!(
            inspect(&m, &r, 89999).unwrap()["waiting_window"]
                .as_array()
                .unwrap()
                .len(),
            m.cases.len()
        );
        assert_eq!(
            inspect(&m, &r, 90000).unwrap()["eligible"]
                .as_array()
                .unwrap()
                .len(),
            m.cases.len()
        );
        assert_eq!(
            inspect(&m, &r, 100000).unwrap()["expired_window"]
                .as_array()
                .unwrap()
                .len(),
            m.cases.len()
        );
        let mut m = m;
        m.limits.run_seconds = 89999;
        assert!(
            inspect(&m, &r, 90000).unwrap()["eligible"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(inspect(&m, &r, 0).is_err());
    }
    #[test]
    fn resolved_failures_are_observation_only_and_never_replayed() {
        let (m, mut r) = input();
        r["cases"][0]["execution"] = json!("COMPLETED");
        r["cases"][0]["semantic"] = json!("FAILED");
        r["cases"][0]["settlement"] = json!("NOT_SIGNED");
        let result = inspect(&m, &r, 90000).unwrap();
        assert_eq!(result["observe_only_count"], 1);
        assert!(
            !result["eligible"]
                .as_array()
                .unwrap()
                .contains(&r["cases"][0]["case"])
        );
        r["cases"][0]["settlement"] = json!("USED");
        assert!(inspect(&m, &r, 90000).is_err());
        r["canonical_api_debits"]["cases"][m.cases[0].id.clone()] = json!("14000");
        assert!(inspect(&m, &r, 90000).is_ok());
    }
    #[test]
    fn canonical_transport_uncertain_state_remains_observation_only_after_resolution() {
        let (m,mut r)=input();
        r["cases"][0]["execution"]=json!("TRANSPORT_UNCERTAIN");
        r["cases"][0]["settlement"]=json!("NOT_SIGNED");
        let result=inspect(&m,&r,90000).unwrap();
        assert_eq!(result["observe_only_count"],1);
        assert_eq!(result["eligible"],json!([]));
        r["cases"][0]["settlement"]=json!("UNOBSERVED");
        assert!(inspect(&m,&r,90000).unwrap_err().to_string().contains("unresolved"));
    }
    #[test]
    fn completed_open_phase_does_not_make_future_cases_eligible() {
        let (mut m, mut r) = input();
        let mut later = m.cases[0].clone();
        later.id = "later".into();
        m.cases.push(later);
        let mut phase = m.phases[0].clone();
        phase.id = "later_phase".into();
        phase.window = "later_window".into();
        phase.cases = vec!["later".into()];
        m.phases.push(phase);
        let mut window = m.windows[0].clone();
        window.id = "later_window".into();
        window.not_before = 180000;
        window.not_after = 190000;
        m.windows.push(window);
        r["cases"][0]["execution"] = json!("COMPLETED");
        r["cases"][0]["settlement"] = json!("NOT_SIGNED");
        r["cases"]
            .as_array_mut()
            .unwrap()
            .push(json!({"case":"later","execution":"UNATTEMPTED","settlement":"UNOBSERVED"}));
        let result = inspect(&m, &r, 90000).unwrap();
        assert_eq!(result["eligible"], json!([]));
        assert_eq!(result["waiting_window"], json!(["later"]));
        assert_eq!(
            inspect(&m, &r, 180000).unwrap()["eligible"],
            json!(["later"])
        );
        m.phases[0].cases.push("later".into());
        m.phases.remove(1);
        m.phases[0].scenario = Scenario::Rotation {
            refill_slots: 1,
            rounds: vec![crate::manifest::RotationRound {
                pool: "pool".into(),
                expected_price_usdc: "0.014".into(),
                depletion_cases: vec!["call".into()],
                service_cases: vec!["later".into()],
            }],
        };
        assert!(
            inspect(&m, &r, 90000)
                .unwrap_err()
                .to_string()
                .contains("partially dispatched")
        );
    }
    #[test]
    fn uncertain_dispatches_pending_payments_and_unknown_accounting_refuse_progress() {
        let (m, r) = input();
        for state in [
            "RESERVED",
            "DISPATCHING",
            "UNKNOWN",
            "TRANSPORT_UNCERTAIN",
            "COMPLETED",
        ] {
            let mut r = r.clone();
            r["cases"][0]["execution"] = json!(state);
            r["cases"][0]["settlement"] = json!("PENDING");
            assert!(inspect(&m, &r, 90000).is_err());
        }
        let mut r = r;
        r["canonical_api_debits"]["status"] = json!("invalid_or_incomplete");
        assert!(inspect(&m, &r, 90000).is_err());
    }
}
