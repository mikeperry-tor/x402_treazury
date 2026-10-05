//! Observed offers, admitted prices and canonical debits; no provider-wide price bound.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn report(report: &Value) -> Result<Value> {
    let cases = report["cases"]
        .as_array()
        .context("fee report cases missing")?;
    let known: BTreeSet<_> = cases
        .iter()
        .map(|c| c["case"].as_str().context("fee case missing"))
        .collect::<Result<_>>()?;
    ensure!(known.len() == cases.len(), "duplicate fee report case");
    let events = report["runtime_events"]
        .as_array()
        .context("fee events missing")?;
    let mut admitted = BTreeMap::new();
    // Application correlation validation includes case cap, session, and unsigned refusal.
    let correlated = report["application_evidence"]["payment_attempts_correlated"].as_u64();
    for event in events.iter().filter(|e| e["kind"] == "application_payment") {
        let detail = &event["detail"];
        let case = detail["case"].as_str().context("admission case missing")?;
        ensure!(known.contains(case), "admission names unknown case");
        let amount: u64 = detail["amount"]
            .as_str()
            .context("admission amount missing")?
            .parse()?;
        ensure!(
            admitted.insert(case, amount).is_none(),
            "duplicate admission price"
        );
    }
    let valid = correlated == Some(admitted.len() as u64);
    let offers = crate::offers::report(report)?;
    ensure!(
        offers.keys().all(|id| known.contains(id.as_str())),
        "challenge names unknown case"
    );
    let debit = &report["canonical_api_debits"];
    let proofs = debit["cases"]
        .as_object()
        .context("fee debit map missing")?;
    ensure!(
        proofs.keys().all(|id| known.contains(id.as_str())),
        "unknown fee debit case"
    );
    ensure!(
        debit["status"] == "validated" || proofs.is_empty(),
        "invalid debit prices exposed"
    );
    let rows = cases
        .iter()
        .map(|case| {
            let id = case["case"].as_str().expect("checked above");
            let admission = valid.then(|| admitted.get(id).copied()).flatten();
            let verified = proofs
                .get(id)
                .map(|v| match v {
                    Value::String(s) => Ok(s.parse::<u64>()?),
                    _ => v.as_u64().context("invalid debit amount"),
                })
                .transpose()?;
            Ok(
                json!({"case":id,"admitted_atomic":admission,"verified_debit_atomic":verified,
            "challenge_observations":if valid {offers.get(id).cloned().unwrap_or_default()} else {Vec::new()},
            "admitted_above_one_cent":admission.map(|v|v>10000),
            "verified_debit_above_one_cent":verified.map(|v|v>10000)}),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"asset":"Base USDC", "threshold_atomic":10000,
        "admissions_status":if valid {"validated"} else {"invalid_or_incomplete"},
        "challenges_status":if valid {"validated"} else {"invalid_or_incomplete"},
        "meaning":"Observed challenge offers, managed admissions and canonical debits are separate evidence. Offers may be rejected or unselected and grant no payment authority. Admission precedes signing and is not a debit. Missing evidence is not zero; no provider-wide maximum or minimum wallet size is inferred. Headerless legacy body offers are not captured.","cases":rows}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"cases":[{"case":"a"},{"case":"b"},{"case":"c"}],
            "application_evidence":{"payment_attempts_correlated":2},
            "runtime_events":[{"kind":"application_payment","detail":{"case":"a","amount":"10000"}},
                {"kind":"application_payment","detail":{"case":"b","amount":"10001"}}],
            "canonical_api_debits":{"status":"validated","cases":{"a":"10000"}}})
    }
    #[test]
    fn sampled_threshold_is_strict_and_admission_never_implies_spending() {
        let r = report(&input()).unwrap();
        assert_eq!(r["cases"][0]["admitted_above_one_cent"], false);
        assert_eq!(r["cases"][1]["admitted_above_one_cent"], true);
        assert!(r["cases"][1]["verified_debit_atomic"].is_null());
        assert!(r["cases"][2]["admitted_above_one_cent"].is_null());
    }
    #[test]
    fn invalid_application_correlation_withholds_admissions() {
        let mut r = input();
        r["application_evidence"] = json!({"status":"invalid_or_incomplete"});
        let result = report(&r).unwrap();
        assert_eq!(result["admissions_status"], "invalid_or_incomplete");
        assert!(result["cases"][0]["admitted_atomic"].is_null());
        assert_eq!(result["cases"][0]["verified_debit_atomic"], 10000);
    }
    #[test]
    fn rejected_challenges_have_prices_without_admission_or_debit() {
        let mut r = input();
        r["runtime_events"].as_array_mut().unwrap().extend([
            json!({"kind":"application_claim","detail":{"case":"c","session":"s","started_micros":1}}),
            json!({"kind":"application_challenge","detail":{"case":"c","session":"s","ordinal":0,"observed_micros":2,
                "observation":{"status":"observed","offers":[{"index":0,"base_usdc":true,"scheme":"exact","amount_atomic":"2000000","above_one_cent":true}]}}})]);
        let v = report(&r).unwrap();
        assert_eq!(
            v["cases"][2]["challenge_observations"][0]["offers"][0]["amount_atomic"],
            "2000000"
        );
        assert!(v["cases"][2]["admitted_atomic"].is_null());
        assert!(v["cases"][2]["verified_debit_atomic"].is_null());
        r["application_evidence"] = json!({"status":"invalid_or_incomplete"});
        let v = report(&r).unwrap();
        assert_eq!(v["challenges_status"], "invalid_or_incomplete");
        assert_eq!(v["cases"][2]["challenge_observations"], json!([]));
    }
    #[test]
    fn duplicate_unknown_or_malformed_financial_evidence_is_refused() {
        let original = input();
        for field in ["case", "amount"] {
            let mut r = original.clone();
            r["runtime_events"][0]["detail"][field] = json!("bad");
            assert!(report(&r).is_err());
        }
        let mut r = original.clone();
        r["runtime_events"][1] = r["runtime_events"][0].clone();
        assert!(report(&r).is_err());
        let mut r = original;
        r["canonical_api_debits"]["status"] = json!("invalid");
        assert!(report(&r).is_err());
    }
}
