//! Per-round lifecycle assertions. Missing timing or fast refill is incomplete, never success.
use crate::{
    manifest::{Manifest, Phase, Scenario},
    registry::Registry,
};
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

fn selected<'a>(state: &'a Value, pool: &str) -> Result<&'a Value> {
    state["pools"]
        .as_array()
        .context("rotation pools missing")?
        .iter()
        .find(|p| p["id"] == pool)
        .context("rotation pool missing")
}
pub(crate) fn refill_ready(state: &Value, pool: &str, job: &str, generation: u64) -> Result<bool> {
    let selected = selected(state, pool)?;
    ensure!(
        selected["generation"].as_u64() == Some(generation),
        "unexpected promotion while awaiting refill"
    );
    let job = state["funding_jobs"]
        .as_array()
        .context("refill jobs missing")?
        .iter()
        .find(|j| j["id"] == job)
        .context("replacement job disappeared")?;
    ensure!(job["pool_id"] == pool, "refill job changed pool");
    if job["phase"] != "COMPLETE" {
        return Ok(false);
    }
    let wallet = selected["addresses"]
        .as_array()
        .context("refill wallets missing")?
        .iter()
        .find(|w| w["id"] == job["wallet_id"])
        .context("replacement wallet missing")?;
    ensure!(
        wallet["role"] == "READY"
            && wallet["address"] == job["recipient"]
            && wallet["target"] == job["target"],
        "completed refill wallet binding/role mismatch"
    );
    let target = job["target"]
        .as_str()
        .context("refill target missing")?
        .parse::<U256>()?;
    ensure!(
        target > U256::ZERO
            && wallet["confirmed_balance"]
                .as_str()
                .context("refill balance missing")?
                .parse::<U256>()?
                >= target,
        "completed refill lacks confirmed target credit"
    );
    let source = state["source_operations"]
        .as_array()
        .context("refill source evidence missing")?
        .iter()
        .find(|o| o["operation_id"] == job["operation_id"])
        .context("completed refill source operation missing")?;
    ensure!(
        source["submission"] == "CONFIRMED" && source["attempts"].as_u64() == Some(1),
        "refill requires one confirmed source submission"
    );
    Ok(true)
}
fn event<'a>(events: &'a [Value], kind: &str, field: &str, id: &str) -> Result<Option<&'a Value>> {
    let mut found = events
        .iter()
        .filter(|e| e["kind"] == kind && e["detail"][field] == id);
    let value = found.next().map(|e| &e["detail"]);
    ensure!(
        found.next().is_none(),
        "duplicate rotation evidence: {kind}"
    );
    Ok(value)
}
fn round_index(detail: &Value) -> Result<usize> {
    // Historical single-rotation start/readiness events had no round field.
    match detail.get("round") {
        None => Ok(0),
        Some(value) => Ok(usize::try_from(
            value.as_u64().context("invalid rotation round index")?,
        )?),
    }
}
fn round_event<'a>(
    events: &'a [Value],
    kind: &str,
    phase: &str,
    index: usize,
    count: usize,
) -> Result<Option<&'a Value>> {
    ensure!(index < count, "requested rotation round is not declared");
    let mut found = None;
    for event in events
        .iter()
        .filter(|e| e["kind"] == kind && e["detail"]["phase"] == phase)
    {
        let detail = &event["detail"];
        let recorded = round_index(detail)?;
        ensure!(
            recorded < count,
            "evidence names an undeclared rotation round"
        );
        if recorded == index {
            ensure!(
                found.replace(detail).is_none(),
                "duplicate rotation evidence for round {index}: {kind}"
            );
        }
    }
    Ok(found)
}
pub(crate) fn report(registry: &Registry, m: &Manifest, report: &Value) -> Result<Value> {
    let events = report["runtime_events"]
        .as_array()
        .context("runtime events missing")?;
    let cases = report["cases"]
        .as_array()
        .context("rotation cases missing")?;
    let proofs = registry.verified_debits(&m.run_id)?;
    let config = registry.pins(&m.run_id)?.resolved_config;
    let mut results = Vec::new();
    for phase in &m.phases {
        let (rounds, lifecycle) = match &phase.scenario {
            Scenario::Rotation { rounds, .. } | Scenario::RefillService { rounds, .. } => {
                (rounds, false)
            }
            Scenario::Lifecycle { rounds, .. } => (rounds, true),
            _ => continue,
        };
        let mut round_results = Vec::new();
        for index in 0..rounds.len() {
            let mut result =
                match round_event(events, "rotation_started", &phase.id, index, rounds.len())? {
                    Some(start) => {
                        let observation = registry
                            .pool_observation(
                                &m.run_id,
                                start["request"]
                                    .as_str()
                                    .context("rotation start request missing")?,
                            )?
                            .context("rotation baseline observation missing")?;
                        phase_result(phase, events, cases, start, &observation, &config, &proofs)?
                    }
                    None => json!({"status":"incomplete","reason":"rotation not started"}),
                };
            result["round"] = json!(index);
            round_results.push(result);
        }
        let mut result = if lifecycle {
            let restart = crate::restart_report::report(events, m, phase)?;
            json!({"status":if round_results.iter().any(|r| r["rotation_status"] != "passed") || restart["status"] != "passed" {"incomplete"}
                else if round_results.iter().any(|r| r["status"] == "failed") {"failed"} else {"passed"},
                "rounds":round_results,"restart":restart})
        } else {
            round_results
                .into_iter()
                .next()
                .context("rotation round missing")?
        };
        result["phase"] = json!(phase.id);
        results.push(result);
    }
    Ok(json!(results))
}
fn phase_result(
    phase: &Phase,
    events: &[Value],
    cases: &[Value],
    start: &Value,
    observation: &Value,
    config: &Value,
    proofs: &std::collections::BTreeMap<String, Value>,
) -> Result<Value> {
    let rounds = match &phase.scenario {
        Scenario::Rotation { rounds, .. }
        | Scenario::RefillService { rounds, .. }
        | Scenario::Lifecycle { rounds, .. } => rounds,
        _ => anyhow::bail!("unsupported rotation report"),
    };
    let index = round_index(start)?;
    let round = rounds.get(index).context("rotation round missing")?;
    let estimate = x402_treazury::qualification::depletion_estimate(
        observation,
        &crate::manifest::usdc(&round.expected_price_usdc)?.to_string(),
        round.depletion_cases.len(),
    )?;
    ensure!(
        start["estimate"] == estimate,
        "recorded depletion estimate changed"
    );
    let Some(boundary) = round_event(events, "rotation_promotion", &phase.id, index, rounds.len())?
    else {
        return Ok(json!({"status":"incomplete","reason":"promotion not observed"}));
    };
    ensure!(
        boundary["before"] == observation["state"]
            && boundary["round"].as_u64() == Some(index as u64),
        "promotion baseline differs from fresh observation"
    );
    let pool = observation["pool"]
        .as_str()
        .context("rotation pool ID missing")?;
    let job = x402_treazury::qualification::lifecycle::promotion(
        &boundary["before"],
        &boundary["after"],
        pool,
    )?;
    ensure!(
        boundary["replacement_job"] == job,
        "replacement job mismatch"
    );
    let promoted = selected(&boundary["after"], pool)?;
    let generation = promoted["generation"]
        .as_u64()
        .context("promoted generation missing")?;
    let active = promoted["addresses"]
        .as_array()
        .context("promoted wallets missing")?
        .iter()
        .find(|w| w["role"] == "ACTIVE")
        .context("promoted active missing")?;
    let mut overlap = Vec::new();
    let mut failures = Vec::new();
    let mut unresolved = Vec::new();
    let mut service_wallets = std::collections::BTreeSet::new();
    let mut overlap_wallets = std::collections::BTreeSet::new();
    for id in round.depletion_cases.iter().chain(&round.service_cases) {
        let case = cases
            .iter()
            .find(|c| c["case"] == *id)
            .context("phase case missing")?;
        if case["execution"] == "SKIPPED_TARGET_REACHED" {
            continue;
        } // registry independently validates skipped suffixes
        if case["semantic"] == "FAILED" {
            failures.push(id.clone());
        }
        if case["execution"] != "COMPLETED" {
            unresolved.push(id.clone());
            continue;
        }
        let claim =
            event(events, "application_claim", "case", id)?.context("rotation claim missing")?;
        let wallet = &config["wallet_bindings"]
            [claim["server"].as_str().context("claim listener missing")?]
            [claim["source"].as_str().context("claim source missing")?]["wallet"];
        ensure!(wallet.is_string(), "rotation wallet binding missing");
        if round.service_cases.contains(id) {
            service_wallets.insert(wallet.as_str().unwrap().to_owned());
        }
        if case["semantic"] == "FAILED" {
            if !matches!(
                case["settlement"].as_str(),
                Some("NOT_SIGNED" | "EXPIRED_UNUSED")
            ) && !(case["settlement"] == "USED" && proofs.contains_key(id))
            {
                unresolved.push(id.clone());
            }
            continue;
        }
        if case["semantic"] != "PASSED" || case["settlement"] != "USED" || !proofs.contains_key(id)
        {
            unresolved.push(id.clone());
            continue;
        }
        let payment = event(events, "application_payment", "case", id)?
            .context("rotation admission missing")?;
        ensure!(
            payment["session"] == observation["session"]
                && payment["admitted_micros"]
                    .as_u64()
                    .zip(observation["observed_micros"].as_u64())
                    .is_some_and(|(p, b)| p >= b),
            "rotation payment predates baseline or uses another application session"
        );
        ensure!(
            wallet.is_string() && payment["pool_name"] == *wallet,
            "rotation payment wallet scope mismatch"
        );
        if round.service_cases.contains(id) && *wallet == round.pool {
            ensure!(
                payment["pool"] == pool
                    && payment["generation"].as_u64() == Some(generation)
                    && payment["address"]
                        .as_str()
                        .context("service payer missing")?
                        .parse::<Address>()?
                        == active["address"]
                            .as_str()
                            .context("promoted address missing")?
                            .parse::<Address>()?,
                "post-promotion service used another wallet generation"
            );
        }
        if round.service_cases.contains(id) {
            let wallet_name = wallet
                .as_str()
                .context("service wallet missing")?
                .to_owned();
            service_wallets.insert(wallet_name.clone());
            let receipt = event(events, "application_receipt", "case", id)?
                .context("service receipt missing")?;
            let debit = event(events, "application_debit_verified", "case", id)?
                .context("service debit missing")?;
            if let Some(evidence) = x402_treazury::qualification::lifecycle::refill_service(
                payment, receipt, debit, &job,
            )? {
                overlap_wallets.insert(wallet_name);
                overlap.push(evidence);
            }
        }
    }
    let Some(ready) = round_event(
        events,
        "rotation_refill_ready",
        &phase.id,
        index,
        rounds.len(),
    )?
    else {
        return Ok(
            json!({"status":"incomplete","reason":"confirmed refill readiness not observed","pending_service":overlap,"failed_cases":failures,"unresolved_cases":unresolved}),
        );
    };
    ensure!(
        ready["job"] == job && ready["state"]["treasury_id"] == observation["state"]["treasury_id"],
        "refill readiness identity mismatch"
    );
    let original_job = boundary["after"]["funding_jobs"]
        .as_array()
        .context("promotion jobs missing")?
        .iter()
        .find(|j| j["id"] == job)
        .context("promoted job missing")?;
    let completed_job = ready["state"]["funding_jobs"]
        .as_array()
        .context("ready jobs missing")?
        .iter()
        .find(|j| j["id"] == job)
        .context("completed job missing")?;
    for field in ["pool_id", "wallet_id", "recipient", "operation_id"] {
        ensure!(
            !original_job[field].is_null() && original_job[field] == completed_job[field],
            "refill changed its original funding operation/binding"
        );
    }
    ensure!(
        refill_ready(&ready["state"], pool, &job, generation)?,
        "recorded refill is not ready"
    );
    let unobserved: Vec<_> = service_wallets.difference(&overlap_wallets).collect();
    Ok(
        json!({"status":if !unobserved.is_empty() || !unresolved.is_empty(){"incomplete"}else if !failures.is_empty(){"failed"}else{"passed"},"failed_cases":failures,"unresolved_cases":unresolved,"rotation_status":if !unobserved.is_empty() || !unresolved.is_empty(){"incomplete"}else{"passed"},"pending_service_status":if !unobserved.is_empty(){"not_observed"}else{"observed"},"pending_service":overlap,"unobserved_service_wallets":unobserved,"replacement_job":job,"generation":generation,"promotion":true,"refill_ready":true,"source_submissions":1}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wallet(id: &str, n: u64, role: &str, balance: &str) -> Value {
        json!({"id":id,"address":format!("0x{n:040x}"),"role":role,"target":"100","confirmed_balance":balance})
    }
    type Fixture = (
        Phase,
        Vec<Value>,
        Vec<Value>,
        Value,
        Value,
        Value,
        std::collections::BTreeMap<String, Value>,
    );
    fn fixture() -> Fixture {
        let phase:Phase=serde_json::from_value(json!({"id":"rotation","window":"w","cases":["d","s"],"pools":["pool"],"required":true,"scenario":{"kind":"rotation","refill_slots":1,"rounds":[{"pool":"pool","expected_price_usdc":"0.000010","depletion_cases":["d"],"service_cases":["s"]}]}})).unwrap();
        let before = json!({"treasury_id":"t","pools":[{"id":"p","name":"pool","generation":0,"addresses":[wallet("a",1,"ACTIVE","0"),wallet("b",2,"READY","100")]}],"funding_jobs":[],"source_operations":[]});
        let after = json!({"treasury_id":"t","pools":[{"id":"p","name":"pool","generation":1,"addresses":[wallet("a",1,"RETIRED","0"),wallet("b",2,"ACTIVE","90"),wallet("c",3,"ALLOCATED","0")]}],"funding_jobs":[{"id":"j","pool_id":"p","wallet_id":"c","recipient":format!("0x{:040x}",3),"target":"100","operation_id":"op","phase":"ALLOCATED"}],"source_operations":[]});
        let observation = json!({"session":"session","pool":"p","pool_name":"pool","observed_micros":10,"state":before,"balances":{"block_height":12,"block_hash":format!("0x{:064x}",1),"confirmed":{"a":"0","b":"100"},"unresolved":{}}});
        let start = json!({"request":"request","estimate":x402_treazury::qualification::depletion_estimate(&observation,"10",1).unwrap()});
        let mut ready = after.clone();
        ready["pools"][0]["addresses"][2] = wallet("c", 3, "READY", "100");
        ready["funding_jobs"][0]["phase"] = json!("COMPLETE");
        ready["source_operations"] =
            json!([{"operation_id":"op","submission":"CONFIRMED","attempts":1}]);
        let mut events = vec![
            json!({"kind":"rotation_promotion","detail":{"phase":"rotation","round":0,"before":before,"after":after,"replacement_job":"j","skipped":[]}}),
            json!({"kind":"rotation_refill_ready","detail":{"phase":"rotation","job":"j","state":ready}}),
        ];
        let mut cases = Vec::new();
        let mut proofs = std::collections::BTreeMap::new();
        for id in ["d", "s"] {
            cases.push(
                json!({"case":id,"execution":"COMPLETED","semantic":"PASSED","settlement":"USED"}),
            );
            events.push(json!({"kind":"application_claim","detail":{"case":id,"server":"main","source":"api","session":"session"}}));
            let payment = json!({"case":id,"session":"session","attempt_id":id,"pool":"p","pool_name":"pool","generation":1,"address":format!("0x{:040x}",2),"amount":"10","admitted_micros":20,"lifecycle":after});
            let receipt = json!({"case":id,"session":"session","attempt_id":id,"receipt":{"classification":"seller_success","transaction":id,"signed_request_started_micros":30,"response_observed_micros":40,"lifecycle":{"observed_micros":50,"state":after}}});
            let debit = json!({"case":id,"session":"session","attempt_id":id,"status":"verified","proof":{"payer":payment["address"],"amount_atomic":"10","transaction":id}});
            proofs.insert(id.into(), debit["proof"].clone());
            for (kind, detail) in [
                ("application_payment", payment),
                ("application_receipt", receipt),
                ("application_debit_verified", debit),
            ] {
                events.push(json!({"kind":kind,"detail":detail}));
            }
        }
        (
            phase,
            events,
            cases,
            start,
            observation,
            json!({"wallet_bindings":{"main":{"api":{"wallet":"pool"}}}}),
            proofs,
        )
    }
    #[test]
    fn promotion_service_and_confirmed_refill_are_all_required() {
        let (phase, events, cases, start, observation, config, mut proofs) = fixture();
        let result = phase_result(
            &phase,
            &events,
            &cases,
            &start,
            &observation,
            &config,
            &proofs,
        )
        .unwrap();
        assert_eq!(result["status"], "passed");
        assert_eq!(result["pending_service"].as_array().unwrap().len(), 1);
        proofs.remove("s");
        assert_eq!(
            phase_result(
                &phase,
                &events,
                &cases,
                &start,
                &observation,
                &config,
                &proofs
            )
            .unwrap()["status"],
            "incomplete"
        );
    }
    #[test]
    fn fast_refill_is_not_observed_and_wrong_payer_or_clock_cannot_pass() {
        let (phase, events, cases, start, observation, config, proofs) = fixture();
        let mut fast = events.clone();
        let receipt = fast
            .iter_mut()
            .find(|e| e["kind"] == "application_receipt" && e["detail"]["case"] == "s")
            .unwrap();
        receipt["detail"]["receipt"]["lifecycle"]["state"]["funding_jobs"][0]["phase"] =
            json!("COMPLETE");
        let result = phase_result(
            &phase,
            &fast,
            &cases,
            &start,
            &observation,
            &config,
            &proofs,
        )
        .unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["pending_service_status"], "not_observed");
        for (field, value) in [
            ("generation", json!(0)),
            ("admitted_micros", json!(9)),
            ("session", json!("another")),
            ("pool_name", json!("another")),
        ] {
            let mut wrong = events.clone();
            let payment = wrong
                .iter_mut()
                .find(|e| e["kind"] == "application_payment" && e["detail"]["case"] == "s")
                .unwrap();
            payment["detail"][field] = value;
            assert!(
                phase_result(
                    &phase,
                    &wrong,
                    &cases,
                    &start,
                    &observation,
                    &config,
                    &proofs
                )
                .is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn each_service_wallet_must_be_observed_during_the_pending_refill() {
        let (mut phase, mut events, mut cases, start, observation, mut config, mut proofs) =
            fixture();
        phase.cases.push("peer".into());
        phase.pools.push("other".into());
        if let Scenario::Rotation { rounds, .. } = &mut phase.scenario {
            rounds[0].service_cases.push("peer".into());
        }
        config["wallet_bindings"]["main"]["peer"] = json!({"wallet":"other"});
        cases.push(
            json!({"case":"peer","execution":"COMPLETED","semantic":"PASSED","settlement":"USED"}),
        );
        let originals: Vec<_> = events
            .iter()
            .filter(|e| e["detail"]["case"] == "s")
            .cloned()
            .collect();
        for mut event in originals {
            let kind = event["kind"].as_str().unwrap().to_owned();
            let detail = &mut event["detail"];
            detail["case"] = json!("peer");
            match kind.as_str() {
                "application_claim" => detail["source"] = json!("peer"),
                "application_payment" => {
                    detail["attempt_id"] = json!("peer");
                    detail["pool"] = json!("q");
                    detail["pool_name"] = json!("other");
                    detail["generation"] = json!(0);
                    detail["address"] = json!(format!("0x{:040x}", 9));
                }
                "application_receipt" => {
                    detail["attempt_id"] = json!("peer");
                    detail["receipt"]["transaction"] = json!("peer");
                }
                "application_debit_verified" => {
                    detail["attempt_id"] = json!("peer");
                    detail["proof"]["transaction"] = json!("peer");
                    detail["proof"]["payer"] = json!(format!("0x{:040x}", 9));
                    proofs.insert("peer".into(), detail["proof"].clone());
                }
                _ => unreachable!(),
            }
            events.push(event);
        }
        assert_eq!(
            phase_result(
                &phase,
                &events,
                &cases,
                &start,
                &observation,
                &config,
                &proofs
            )
            .unwrap()["status"],
            "passed"
        );
        let receipt = events
            .iter_mut()
            .find(|e| e["kind"] == "application_receipt" && e["detail"]["case"] == "peer")
            .unwrap();
        receipt["detail"]["receipt"]["lifecycle"]["state"]["funding_jobs"][0]["phase"] =
            json!("COMPLETE");
        let result = phase_result(
            &phase,
            &events,
            &cases,
            &start,
            &observation,
            &config,
            &proofs,
        )
        .unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["unobserved_service_wallets"], json!(["other"]));
    }
    #[test]
    fn readiness_requires_credit_and_exactly_one_confirmed_source_submission() {
        let (_, events, _, _, _, _, _) = fixture();
        let state = &events[1]["detail"]["state"];
        assert!(refill_ready(state, "p", "j", 1).unwrap());
        for (pointer, value) in [
            ("/pools/0/addresses/2/confirmed_balance", json!("99")),
            ("/source_operations/0/attempts", json!(2)),
            (
                "/source_operations/0/submission",
                json!("BROADCAST_REQUESTED"),
            ),
            ("/pools/0/generation", json!(2)),
        ] {
            let mut wrong = state.clone();
            *wrong.pointer_mut(pointer).unwrap() = value;
            assert!(refill_ready(&wrong, "p", "j", 1).is_err(), "{pointer}");
        }
        let mut pending = state.clone();
        pending["funding_jobs"][0]["phase"] = json!("VERIFYING_CREDIT");
        assert!(!refill_ready(&pending, "p", "j", 1).unwrap());
    }
    #[test]
    fn each_lifecycle_round_uses_only_its_own_cases_and_boundary() {
        let (mut phase, events, cases, start, observation, config, proofs) = fixture();
        let Scenario::Rotation { rounds, .. } = &phase.scenario else {
            unreachable!()
        };
        let first = rounds[0].clone();
        let mut second = first.clone();
        second.depletion_cases = vec!["d2".into()];
        second.service_cases = vec!["s2".into()];
        phase.cases.extend(["d2".into(), "s2".into()]);
        phase.scenario = Scenario::Lifecycle {
            refill_slots: 2,
            restart: crate::manifest::Restart::SubmittedDeposit,
            rounds: vec![first, second],
        };
        assert_eq!(
            phase_result(
                &phase,
                &events,
                &cases,
                &start,
                &observation,
                &config,
                &proofs
            )
            .unwrap()["status"],
            "passed"
        );
        let mut next = start.clone();
        next["round"] = json!(1);
        assert_eq!(
            phase_result(
                &phase,
                &events,
                &cases,
                &next,
                &observation,
                &config,
                &proofs
            )
            .unwrap()["reason"],
            "promotion not observed"
        );
        assert!(
            round_event(&events, "rotation_refill_ready", "rotation", 1, 2)
                .unwrap()
                .is_none()
        );
        let mut invalid = events.clone();
        invalid.push(events[0].clone());
        assert!(round_event(&invalid, "rotation_promotion", "rotation", 0, 2).is_err());
        invalid.last_mut().unwrap()["detail"]["round"] = json!(2);
        assert!(round_event(&invalid, "rotation_promotion", "rotation", 0, 2).is_err());
        invalid.last_mut().unwrap()["detail"]["round"] = json!("1");
        assert!(round_event(&invalid, "rotation_promotion", "rotation", 0, 2).is_err());
    }
    #[test]
    fn provider_failure_does_not_discard_later_verified_rotation_and_refill() {
        let (phase, events, mut cases, start, observation, config, mut proofs) = fixture();
        cases[0]["semantic"] = json!("FAILED");
        cases[0]["settlement"] = json!("NOT_SIGNED");
        proofs.remove("d");
        let result = phase_result(
            &phase,
            &events,
            &cases,
            &start,
            &observation,
            &config,
            &proofs,
        )
        .unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["failed_cases"], json!(["d"]));
        assert_eq!(result["promotion"], true);
        assert_eq!(result["refill_ready"], true);
        assert_eq!(result["pending_service_status"], "observed");
        // A failed service cannot be counted as a successful overlap sample.
        cases[1]["semantic"] = json!("FAILED");
        let result = phase_result(
            &phase,
            &events,
            &cases,
            &start,
            &observation,
            &config,
            &proofs,
        )
        .unwrap();
        assert_eq!(result["pending_service_status"], "not_observed");
    }
}
