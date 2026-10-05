//! Distinguish concurrent MCP/application/payment intervals without claiming wire timing.
use crate::manifest::{Manifest, Scenario};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Interval = (u64, u64);
fn interval(value: &Value, start: &str, end: &str) -> Result<Interval> {
    let begin = value[start]
        .as_u64()
        .context("concurrency start timestamp missing")?;
    let finish = value[end]
        .as_u64()
        .context("concurrency end timestamp missing")?;
    ensure!(finish >= begin, "concurrency interval runs backwards");
    Ok((begin, finish))
}
fn overlap(intervals: &[Interval]) -> bool {
    intervals.len() > 1 && intervals.iter().map(|i| i.0).max() < intervals.iter().map(|i| i.1).min()
}
fn event<'a>(events: &'a [Value], kind: &str, case: &str) -> Result<Option<&'a Value>> {
    let mut matches = events
        .iter()
        .filter(|e| e["kind"] == kind && e["detail"]["case"] == case);
    let found = matches.next().map(|e| &e["detail"]);
    ensure!(
        matches.next().is_none(),
        "duplicate concurrency evidence: {kind}"
    );
    Ok(found)
}
fn batch(m: &Manifest, config: &Value, events: &[Value], ids: &[String]) -> Result<Value> {
    let mut mcp = Vec::new();
    let mut application = Vec::new();
    let mut payment_work = Vec::new();
    let mut signed_requests = Vec::new();
    let mut sessions = BTreeSet::new();
    let mut wallets = BTreeSet::new();
    let mut servers = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut nonces = BTreeSet::new();
    let mut totals: BTreeMap<String, alloy_primitives::U256> = BTreeMap::new();
    for id in ids {
        let case = m
            .cases
            .iter()
            .find(|c| &c.id == id)
            .context("concurrency case missing")?;
        let Some(driver) = event(events, "mcp_finished", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing MCP interval"}));
        };
        let Some(claim) = event(events, "application_claim", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing application acceptance"}));
        };
        let Some(done) = event(events, "application_finished", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing application completion"}));
        };
        let session = claim["session"]
            .as_str()
            .context("application session missing")?;
        ensure!(
            done["session"] == session,
            "application interval session mismatch"
        );
        sessions.insert(session);
        mcp.push(interval(driver, "mcp_started_ms", "mcp_finished_ms")?);
        let app = interval(
            &json!({"start":claim["started_micros"],"end":done["finished_micros"]}),
            "start",
            "end",
        )?;
        application.push(app);
        servers.insert(case.server.clone());
        sources.insert(case.source.clone());
        if case.unsigned {
            continue;
        }
        let Some(payment) = event(events, "application_payment", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing payment admission"}));
        };
        let Some(receipt) = event(events, "application_receipt", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing seller observation"}));
        };
        let Some(debit) = event(events, "application_debit_verified", id)? else {
            return Ok(json!({"status":"incomplete","reason":"missing canonical debit proof"}));
        };
        for correlated in [payment, receipt, debit] {
            ensure!(
                correlated["session"] == session
                    && correlated["attempt_id"] == payment["attempt_id"],
                "concurrent payment correlation mismatch"
            );
        }
        let wallet = config["wallet_bindings"][&case.server][&case.source]["wallet"]
            .as_str()
            .context("reviewed wallet binding missing")?;
        ensure!(
            payment["pool_name"] == wallet,
            "concurrent payment used a different wallet binding"
        );
        wallets.insert(wallet.to_owned());
        if payment.get("balance_evidence").is_none() {
            return Ok(
                json!({"status":"incomplete","reason":"admission balance accounting not recorded by this application build"}),
            );
        }
        x402_treazury::qualification::validate_admission_balance(payment)?;
        let proof = &debit["proof"];
        let payer = proof["payer"]
            .as_str()
            .context("proof payer missing")?
            .parse::<alloy_primitives::Address>()?;
        let nonce = proof["nonce"]
            .as_str()
            .context("proof nonce missing")?
            .parse::<alloy_primitives::B256>()?;
        ensure!(
            nonces.insert((payer, nonce)),
            "concurrent calls reused an authorization nonce"
        );
        let amount = proof["amount_atomic"]
            .as_str()
            .context("proof amount missing")?
            .parse::<alloy_primitives::U256>()?;
        let total = totals.entry(wallet.to_owned()).or_default();
        *total = total
            .checked_add(amount)
            .context("concurrent debit total overflow")?;
        let receipt = &receipt["receipt"];
        // Older evidence is still useful, but cannot prove these interval assertions.
        if payment.get("admitted_micros").is_none()
            || receipt.get("signed_request_started_micros").is_none()
            || receipt.get("response_observed_micros").is_none()
        {
            return Ok(
                json!({"status":"incomplete","reason":"payment timing not recorded by this application build"}),
            );
        }
        let paid = interval(
            &json!({"start":payment["admitted_micros"],"end":receipt["response_observed_micros"]}),
            "start",
            "end",
        )?;
        let signed = interval(
            receipt,
            "signed_request_started_micros",
            "response_observed_micros",
        )?;
        ensure!(
            app.0 <= paid.0 && paid.0 <= signed.0 && signed.1 <= app.1,
            "payment timing lies outside application interval"
        );
        payment_work.push(paid);
        signed_requests.push(signed);
    }
    ensure!(
        sessions.len() == 1,
        "cannot compare concurrency clocks across application sessions"
    );
    let mcp_overlap = overlap(&mcp);
    let app_overlap = overlap(&application);
    Ok(
        json!({"status":if mcp_overlap && app_overlap {"passed"} else {"failed"},
        "mcp_overlap":mcp_overlap,"application_overlap":app_overlap,
        "payment_work_overlap":overlap(&payment_work),"signed_request_future_overlap":overlap(&signed_requests),
        "paid_calls":signed_requests.len(),"wallets":wallets,"servers":servers,"sources":sources,
        "balance_accounting":"each paid admission covered by confirmed balance after retaining all unresolved exposure",
        "verified_debits_atomic":totals.into_iter().map(|(k,v)|(k,v.to_string())).collect::<BTreeMap<_,_>>(),
        "scope":"Interval overlap within one session; signed-request future timing includes connection and final headers, not socket-byte or signature-computation timing"}),
    )
}
pub fn report(m: &Manifest, config: &Value, report: &Value) -> Result<Value> {
    if m.phases
        .iter()
        .any(|p| matches!(p.scenario, Scenario::Concurrency { .. }))
    {
        ensure!(
            report["application_evidence"]["status"] != "invalid_or_incomplete",
            "application/payment evidence failed validation; concurrency cannot qualify"
        );
    }
    let events = report["runtime_events"]
        .as_array()
        .context("runtime events missing")?;
    let mut results = Vec::new();
    for phase in &m.phases {
        if let Scenario::Concurrency { batches } = &phase.scenario {
            for ids in batches {
                let mut result = batch(m, config, events, ids)?;
                result["phase"] = json!(phase.id);
                result["cases"] = json!(ids);
                results.push(result);
            }
        }
    }
    Ok(json!(results))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Value, Value) {
        let mut m = crate::tests::manifest();
        let mut first = m.cases[0].clone();
        first.id = "a".into();
        let mut second = first.clone();
        second.id = "b".into();
        second.server = "other".into();
        m.cases = vec![first, second];
        m.phases[0].cases = vec!["a".into(), "b".into()];
        m.phases[0].scenario = Scenario::Concurrency {
            batches: vec![m.phases[0].cases.clone()],
        };
        let config = json!({"wallet_bindings":{"main":{"api":{"wallet":"pool"}},"other":{"api":{"wallet":"pool"}}}});
        let mut events = Vec::new();
        for (index, case) in ["a", "b"].iter().enumerate() {
            let common = json!({"case":case,"session":"one","attempt_id":case});
            for (kind, extra) in [
                (
                    "mcp_finished",
                    json!({"mcp_started_ms":index,"mcp_finished_ms":10}),
                ),
                ("application_claim", json!({"started_micros":index})),
                ("application_finished", json!({"finished_micros":100})),
                (
                    "application_payment",
                    json!({"pool_name":"pool","admitted_micros":10,"amount":"7","balance_evidence":{"confirmed_balance_atomic":"20","reserved_before_atomic":if index==0 {"0"} else {"7"},"reserved_after_atomic":if index==0 {"7"} else {"14"},"available_after_atomic":if index==0 {"13"} else {"6"},"block_height":10,"block_hash":format!("0x{:064x}",1)}}),
                ),
                (
                    "application_receipt",
                    json!({"receipt":{"signed_request_started_micros":20+index,"response_observed_micros":90}}),
                ),
                (
                    "application_debit_verified",
                    json!({"proof":{"payer":format!("0x{:040x}",1),"nonce":format!("0x{:064x}",index+1),"amount_atomic":"7"}}),
                ),
            ] {
                let mut detail = common.clone();
                detail
                    .as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().unwrap().clone());
                events.push(json!({"kind":kind,"detail":detail}));
            }
        }
        (m, config, json!({"runtime_events":events}))
    }
    #[test]
    fn concurrent_payments_report_distinct_evidence_levels_and_exact_bindings() {
        let (m, config, evidence) = fixture();
        let result = report(&m, &config, &evidence).unwrap();
        assert_eq!(result[0]["status"], "passed");
        assert_eq!(result[0]["signed_request_future_overlap"], true);
        assert_eq!(result[0]["verified_debits_atomic"]["pool"], "14");
        assert_eq!(result[0]["servers"].as_array().unwrap().len(), 2);
        let mut wrong = config.clone();
        wrong["wallet_bindings"]["other"]["api"]["wallet"] = json!("other_pool");
        assert!(report(&m, &wrong, &evidence).is_err());
        let mut separate = evidence.clone();
        separate["runtime_events"][9]["detail"]["pool_name"] = json!("other_pool");
        assert_eq!(
            report(&m, &wrong, &separate).unwrap()[0]["wallets"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn missing_mixed_duplicate_or_nonoverlapping_evidence_cannot_pass() {
        let (m, config, evidence) = fixture();
        let mut invalid = evidence.clone();
        invalid["application_evidence"] = json!({"status":"invalid_or_incomplete"});
        assert!(report(&m, &config, &invalid).is_err());
        let mut missing_balance = evidence.clone();
        missing_balance["runtime_events"][3]["detail"]
            .as_object_mut()
            .unwrap()
            .remove("balance_evidence");
        assert_eq!(
            report(&m, &config, &missing_balance).unwrap()[0]["status"],
            "incomplete"
        );
        let mut overspent = evidence.clone();
        overspent["runtime_events"][9]["detail"]["balance_evidence"]["confirmed_balance_atomic"] =
            json!("13");
        assert!(report(&m, &config, &overspent).is_err());
        let mut missing = evidence.clone();
        missing["runtime_events"].as_array_mut().unwrap().pop();
        assert_eq!(
            report(&m, &config, &missing).unwrap()[0]["status"],
            "incomplete"
        );
        let mut old = evidence.clone();
        old["runtime_events"][3]["detail"]
            .as_object_mut()
            .unwrap()
            .remove("admitted_micros");
        assert_eq!(
            report(&m, &config, &old).unwrap()[0]["status"],
            "incomplete"
        );
        let mut sequential = evidence.clone();
        sequential["runtime_events"][6]["detail"]["mcp_started_ms"] = json!(10);
        assert_eq!(
            report(&m, &config, &sequential).unwrap()[0]["status"],
            "failed"
        );
        let mut mixed = evidence.clone();
        for event in mixed["runtime_events"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .skip(6)
        {
            event["detail"]["session"] = json!("another");
        }
        assert!(report(&m, &config, &mixed).is_err());
        let mut reused = evidence.clone();
        reused["runtime_events"][11]["detail"]["proof"]["nonce"] =
            reused["runtime_events"][5]["detail"]["proof"]["nonce"].clone();
        assert!(report(&m, &config, &reused).is_err());
        let mut duplicate = evidence.clone();
        duplicate["runtime_events"]
            .as_array_mut()
            .unwrap()
            .push(evidence["runtime_events"][0].clone());
        assert!(report(&m, &config, &duplicate).is_err());
    }
    #[test]
    fn overlap_requires_positive_shared_interval() {
        assert!(overlap(&[(0, 10), (2, 8)]));
        assert!(!overlap(&[(0, 2), (2, 8)]));
        assert!(!overlap(&[(0, 3), (2, 5), (4, 6)]));
        assert!(!overlap(&[(0, 3)]));
        assert!(!overlap(&[]));
        assert!(interval(&json!({"s":2,"e":1}), "s", "e").is_err());
    }
}
