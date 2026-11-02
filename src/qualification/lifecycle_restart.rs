//! Durable checkpoint/identity checks. These do not prove that a process restarted.
//! The supervisor must separately authenticate sessions and successful drained exit.
use super::{indexed, pending};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
pub enum Checkpoint {
    QueuedRefill,
    SubmittedDeposit,
}

fn selected<'a>(
    state: &'a Value,
    pool: &str,
    job: &str,
) -> Result<(&'a Value, &'a Value, &'a Value)> {
    ensure!(
        state["schema_version"] == 2,
        "unsupported lifecycle evidence version"
    );
    let pools = indexed(state, "pools")?;
    let jobs = indexed(state, "funding_jobs")?;
    let selected_pool = *pools.get(pool).context("restart pool missing")?;
    let selected_job = *jobs.get(job).context("restart funding job missing")?;
    ensure!(
        selected_job["pool_id"] == pool,
        "restart job belongs to another pool"
    );
    let wallets = indexed(selected_pool, "addresses")?;
    let wallet = wallets
        .get(
            selected_job["wallet_id"]
                .as_str()
                .context("restart wallet ID missing")?,
        )
        .context("restart funding wallet missing")?;
    ensure!(
        wallet["address"] == selected_job["recipient"]
            && wallet["target"] == selected_job["target"],
        "restart funding wallet binding differs"
    );
    ensure!(
        (wallet["role"] == "READY" && wallet["destination_recorded"] == true)
            || (wallet["role"] == "ALLOCATED" && pending(selected_job)),
        "restart destination readiness differs"
    );
    let claim = selected_pool["reserve_claims"]
        .as_array()
        .context("restart reserve claims missing")?
        .iter()
        .find(|claim| claim["wallet_id"] == wallet["id"])
        .context("restart replacement claim missing")?;
    ensure!(
        matches!(claim["slot"].as_str(), Some("small" | "large"))
            && claim["generation"].as_u64().is_some(),
        "restart claim identity missing"
    );
    ensure!(
        claim["enabled"] == true
            && (wallet["role"] != "READY" || claim["destination_ready"] == true),
        "restart claim not ready"
    );
    Ok((selected_pool, selected_job, claim))
}

fn source<'a>(state: &'a Value, job: &Value) -> Result<Option<&'a Value>> {
    let id = job["operation_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("restart operation ID missing")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut found = None;
    for operation in state["source_operations"]
        .as_array()
        .context("restart source operations missing")?
    {
        let operation_id = operation["operation_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("restart source operation identity missing")?;
        ensure!(
            seen.insert(operation_id),
            "duplicate restart source operation"
        );
        if operation_id == id {
            found = Some(operation);
        }
    }
    if matches!(
        job["phase"].as_str(),
        Some("PREPARED" | "DEPOSIT_PENDING" | "SWAPPING" | "VERIFYING_CREDIT" | "COMPLETE")
    ) {
        ensure!(
            found.is_some(),
            "restart prepared job lacks source operation"
        );
    }
    if job["phase"] == "COMPLETE" {
        ensure!(
            found.is_some_and(|o| o["submission"] == "CONFIRMED"),
            "restart completed refill lacks source confirmation"
        );
    }
    Ok(found)
}

fn attempts(operation: Option<&Value>) -> Result<u64> {
    let Some(operation) = operation else {
        return Ok(0);
    };
    let attempts = operation["attempts"]
        .as_u64()
        .context("restart submission count missing")?;
    ensure!(
        attempts <= 1,
        "restart observed duplicate source submission"
    );
    ensure!(
        matches!(
            (operation["submission"].as_str(), attempts),
            (Some("PREPARED"), 0)
                | (
                    Some("BROADCAST_REQUESTED" | "BROADCAST" | "UNKNOWN" | "CONFIRMED"),
                    1
                )
        ),
        "restart source submission state/count disagree"
    );
    Ok(attempts)
}

/// False means this exact checkpoint was not observed. Never manufacture a hit
/// from a later COMPLETE state, even if the operation must have passed through it.
pub fn observed(state: &Value, pool: &str, job: &str, checkpoint: Checkpoint) -> Result<bool> {
    let (_, job, _) = selected(state, pool, job)?;
    let operation = source(state, job)?;
    let submissions = attempts(operation)?;
    match checkpoint {
        Checkpoint::QueuedRefill => Ok(job["phase"] == "ALLOCATED" && operation.is_none()),
        Checkpoint::SubmittedDeposit => Ok(job["phase"] == "DEPOSIT_PENDING"
            && submissions == 1
            && operation.is_some_and(|o| {
                matches!(o["submission"].as_str(), Some("BROADCAST" | "CONFIRMED"))
            })),
    }
}

fn unchanged(before: &Value, after: &Value, pool: &str, job: &str) -> Result<()> {
    ensure!(
        before["treasury_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
            && before["treasury_id"] == after["treasury_id"],
        "restart treasury changed"
    );
    let (old_pool, old_job, old_claim) = selected(before, pool, job)?;
    let (new_pool, new_job, new_claim) = selected(after, pool, job)?;
    for field in ["slot", "generation", "wallet_id"] {
        ensure!(
            old_claim[field] == new_claim[field],
            "restart changed the replacement reserve claim"
        );
    }
    let old_jobs = indexed(before, "funding_jobs")?;
    let new_jobs = indexed(after, "funding_jobs")?;
    let bindings = |jobs: &std::collections::BTreeMap<&str, &Value>| {
        jobs.iter()
            .filter(|(_, value)| value["pool_id"] == pool)
            .map(|(id, value)| {
                (
                    (*id).to_owned(),
                    (
                        value["wallet_id"].clone(),
                        value["recipient"].clone(),
                        value["operation_id"].clone(),
                    ),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    ensure!(
        bindings(&old_jobs) == bindings(&new_jobs),
        "restart changed the pool's funding jobs"
    );
    ensure!(
        old_pool["name"].is_string()
            && old_pool["name"] == new_pool["name"]
            && old_pool["generation"].as_u64().is_some()
            && old_pool["generation"] == new_pool["generation"],
        "restart pool identity/generation changed"
    );
    for field in ["pool_id", "wallet_id", "recipient", "operation_id"] {
        ensure!(
            old_job[field].as_str().is_some_and(|s| !s.is_empty())
                && old_job[field] == new_job[field],
            "restart funding binding/operation changed"
        );
    }
    let old_wallets = indexed(old_pool, "addresses")?;
    let new_wallets = indexed(new_pool, "addresses")?;
    let previous = source(before, old_job)?;
    ensure!(
        old_wallets.len() == new_wallets.len(),
        "restart allocated or removed wallets"
    );
    for (id, old) in old_wallets {
        let new = new_wallets.get(id).context("restart lost a wallet")?;
        ensure!(
            old["address"].is_string() && old["address"] == new["address"],
            "restart changed a wallet address"
        );
        if old["id"] == old_job["wallet_id"] {
            let old_target: alloy_primitives::U256 = old["target"]
                .as_str()
                .context("old target missing")?
                .parse()?;
            let new_target: alloy_primitives::U256 = new["target"]
                .as_str()
                .context("new target missing")?
                .parse()?;
            ensure!(
                old_target > alloy_primitives::U256::ZERO && new_target >= old_target,
                "restart decreased the replacement funding target"
            );
            ensure!(
                previous.is_none() || old_target == new_target,
                "restart changed an already prepared funding target"
            );
            ensure!(
                old["role"] == new["role"]
                    || (old["role"] == "ALLOCATED" && new["role"] == "READY"),
                "restart regressed replacement readiness"
            );
        } else {
            ensure!(
                old["role"] == new["role"] && old["target"] == new["target"],
                "restart changed an existing wallet role/target"
            );
        }
    }
    let current = source(after, new_job)?;
    ensure!(
        previous.is_none() || current.is_some(),
        "restart lost prepared source operation"
    );
    ensure!(
        attempts(current)? >= attempts(previous)?,
        "restart submission count regressed"
    );
    for operation in [previous, current].into_iter().flatten() {
        let _: crate::rotation::transaction::TransactionFacts =
            serde_json::from_value(operation["facts"].clone())
                .context("restart transaction facts missing or malformed")?;
        ensure!(
            operation["prepared_record_sha256"]
                .as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "restart prepared record digest missing or malformed"
        );
    }
    if let Some(previous) = previous {
        let current = current.context("restart lost prepared record")?;
        ensure!(
            previous["facts"] == current["facts"]
                && previous["prepared_record_sha256"] == current["prepared_record_sha256"],
            "restart changed prepared transaction identity"
        );
    }
    if previous.is_some_and(|o| o["submission"] == "CONFIRMED") {
        ensure!(
            current.is_some_and(|o| o["submission"] == "CONFIRMED"),
            "restart lost source confirmation"
        );
    }
    Ok(())
}

/// Normal graceful draining may finish preparation, submit once or complete the
/// refill. Compare all three observations without insisting that background work
/// freezes when shutdown is requested. Authentication and process evidence belong
/// to the caller; these snapshots alone never qualify a restart.
pub fn continuity(
    observed_state: &Value,
    drained: &Value,
    reopened: &Value,
    pool: &str,
    job: &str,
    checkpoint: Checkpoint,
) -> Result<Value> {
    ensure!(
        observed(observed_state, pool, job, checkpoint)?,
        "restart checkpoint not observed"
    );
    unchanged(observed_state, drained, pool, job)?;
    unchanged(drained, reopened, pool, job)?;
    let (_, completed, _) = selected(reopened, pool, job)?;
    Ok(
        json!({"pool":pool,"job":job,"operation_id":completed["operation_id"],
        "checkpoint":match checkpoint { Checkpoint::QueuedRefill => "queued_refill", Checkpoint::SubmittedDeposit => "submitted_deposit" },
        "submission_attempts":attempts(source(reopened, completed)?)?,
        "scope":"durable funding continuity only; process/session restart evidence is required separately"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued() -> Value {
        json!({"schema_version":2,"treasury_id":"t","pools":[{"id":"p","name":"pool","generation":1,"reserve_claims":[{"slot":"small","generation":1,"wallet_id":"replacement","enabled":true,"destination_ready":false}],"addresses":[
            {"id":"active","address":"0x01","role":"ACTIVE","target":"2000000"},
            {"id":"replacement","address":"0x02","role":"ALLOCATED","target":"2000000"}
        ]}],"funding_jobs":[{"id":"j","pool_id":"p","wallet_id":"replacement","recipient":"0x02",
            "operation_id":"op","phase":"ALLOCATED","target":"2000000"}],"source_operations":[]})
    }
    fn submitted() -> Value {
        let mut state = queued();
        state["funding_jobs"][0]["phase"] = json!("DEPOSIT_PENDING");
        state["source_operations"] = json!([{"operation_id":"op","submission":"BROADCAST","attempts":1,"facts":{"txid":"tx","expiry_height":200,"amount_zatoshis":100,"fee_zatoshis":10,"deadline":5000},"prepared_record_sha256":"ab".repeat(32)}]);
        state
    }
    fn ready() -> Value {
        let mut state = submitted();
        state["funding_jobs"][0]["phase"] = json!("COMPLETE");
        state["pools"][0]["addresses"][1]["role"] = json!("READY");
        state["pools"][0]["addresses"][1]["destination_recorded"] = json!(true);
        state["pools"][0]["reserve_claims"][0]["destination_ready"] = json!(true);
        state["source_operations"][0]["submission"] = json!("CONFIRMED");
        state
    }
    #[test]
    fn requires_exact_observed_checkpoint_not_an_inferred_past_transition() {
        assert!(observed(&queued(), "p", "j", Checkpoint::QueuedRefill).unwrap());
        assert!(observed(&submitted(), "p", "j", Checkpoint::SubmittedDeposit).unwrap());
        for checkpoint in [Checkpoint::QueuedRefill, Checkpoint::SubmittedDeposit] {
            assert!(!observed(&ready(), "p", "j", checkpoint).unwrap());
            assert!(continuity(&ready(), &ready(), &ready(), "p", "j", checkpoint).is_err());
        }
        let mut unsupported = queued();
        unsupported["schema_version"] = Value::Null;
        assert!(observed(&unsupported, "p", "j", Checkpoint::QueuedRefill).is_err());
        for pointer in [
            "/pools/0/addresses/1/destination_recorded",
            "/pools/0/reserve_claims/0/enabled",
            "/pools/0/reserve_claims/0/destination_ready",
        ] {
            let mut invalid = ready();
            *invalid.pointer_mut(pointer).unwrap() = json!(false);
            assert!(
                observed(&invalid, "p", "j", Checkpoint::SubmittedDeposit).is_err(),
                "{pointer}"
            );
        }
        for (field, value) in [("slot", json!("unknown")), ("generation", Value::Null)] {
            let mut invalid = queued();
            invalid["pools"][0]["reserve_claims"][0][field] = value;
            assert!(
                observed(&invalid, "p", "j", Checkpoint::QueuedRefill).is_err(),
                "{field}"
            );
        }
        let mut missing = ready();
        missing["pools"][0]["reserve_claims"] = json!([]);
        assert!(observed(&missing, "p", "j", Checkpoint::SubmittedDeposit).is_err());
        let mut ambiguous = submitted();
        ambiguous["source_operations"][0]["submission"] = json!("UNKNOWN");
        assert!(!observed(&ambiguous, "p", "j", Checkpoint::SubmittedDeposit).unwrap());
        ambiguous["source_operations"][0]["submission"] = json!("BROADCAST_REQUESTED");
        assert!(!observed(&ambiguous, "p", "j", Checkpoint::SubmittedDeposit).unwrap());
    }
    #[test]
    fn graceful_drain_may_advance_but_preserves_one_operation_and_submission() {
        let proof = continuity(
            &queued(),
            &submitted(),
            &ready(),
            "p",
            "j",
            Checkpoint::QueuedRefill,
        )
        .unwrap();
        assert_eq!(proof["submission_attempts"], 1);
        assert_eq!(proof["operation_id"], "op");
        continuity(
            &submitted(),
            &ready(),
            &ready(),
            "p",
            "j",
            Checkpoint::SubmittedDeposit,
        )
        .unwrap();
        // This proof binds the selected replacement; it does not freeze other
        // reserve settings. Destination readiness may advance during the drain.
        let mut observed = queued();
        observed["pools"][0]["reserve_claims"].as_array_mut().unwrap().push(
            json!({"slot":"large","generation":0,"wallet_id":null,"enabled":false,"destination_ready":false})
        );
        let mut progressed = ready();
        progressed["pools"][0]["reserve_claims"].as_array_mut().unwrap().push(
            json!({"slot":"large","generation":0,"wallet_id":null,"enabled":true,"destination_ready":false})
        );
        continuity(
            &observed,
            &progressed,
            &progressed,
            "p",
            "j",
            Checkpoint::QueuedRefill,
        )
        .unwrap();
        // An unsigned bridge minimum can increase before preparation.
        let mut larger = ready();
        larger["funding_jobs"][0]["target"] = json!("2100000");
        larger["pools"][0]["addresses"][1]["target"] = json!("2100000");
        continuity(
            &queued(),
            &larger,
            &larger,
            "p",
            "j",
            Checkpoint::QueuedRefill,
        )
        .unwrap();
        assert!(
            continuity(
                &queued(),
                &submitted(),
                &larger,
                "p",
                "j",
                Checkpoint::QueuedRefill
            )
            .is_err()
        );
    }
    #[test]
    fn newly_prepared_and_historical_records_require_identity_evidence() {
        for field in ["facts", "prepared_record_sha256"] {
            let mut missing = submitted();
            missing["source_operations"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                continuity(
                    &queued(),
                    &queued(),
                    &missing,
                    "p",
                    "j",
                    Checkpoint::QueuedRefill
                )
                .is_err()
            );
            assert!(
                continuity(
                    &missing,
                    &missing,
                    &missing,
                    "p",
                    "j",
                    Checkpoint::SubmittedDeposit
                )
                .is_err()
            );
            assert!(
                continuity(
                    &submitted(),
                    &missing,
                    &submitted(),
                    "p",
                    "j",
                    Checkpoint::SubmittedDeposit
                )
                .is_err()
            );
        }
    }
    #[test]
    fn rejects_rebinding_duplicate_submission_and_state_loss() {
        for (pointer, value) in [
            ("/treasury_id", json!("another")),
            ("/pools/0/generation", json!(2)),
            ("/pools/0/reserve_claims/0/generation", json!(2)),
            ("/pools/0/reserve_claims/0/slot", json!("large")),
            ("/pools/0/addresses/0/role", json!("RETIRED")),
            ("/pools/0/addresses/0/address", json!("changed")),
            ("/funding_jobs/0/operation_id", json!("new_operation")),
            ("/funding_jobs/0/recipient", json!("another")),
            ("/source_operations/0/facts/txid", json!("replacement")),
            ("/source_operations/0/facts/deadline", json!(6000)),
            ("/source_operations/0/facts/fee_zatoshis", json!(0)),
            ("/source_operations/0/facts", Value::Null),
            (
                "/source_operations/0/prepared_record_sha256",
                json!("cd".repeat(32)),
            ),
            ("/source_operations/0/prepared_record_sha256", Value::Null),
            ("/source_operations/0/attempts", json!(2)),
            ("/source_operations/0/attempts", json!(0)),
            ("/source_operations", json!([])),
        ] {
            let mut changed = submitted();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(
                continuity(
                    &submitted(),
                    &submitted(),
                    &changed,
                    "p",
                    "j",
                    Checkpoint::SubmittedDeposit
                )
                .is_err(),
                "{pointer}"
            );
        }
        let mut duplicate = submitted();
        let operation = duplicate["source_operations"][0].clone();
        duplicate["source_operations"]
            .as_array_mut()
            .unwrap()
            .push(operation);
        assert!(observed(&duplicate, "p", "j", Checkpoint::SubmittedDeposit).is_err());
        let mut extra = submitted();
        let mut job = extra["funding_jobs"][0].clone();
        job["id"] = json!("extra_job");
        job["operation_id"] = json!("extra_operation");
        extra["funding_jobs"].as_array_mut().unwrap().push(job);
        assert!(
            continuity(
                &submitted(),
                &submitted(),
                &extra,
                "p",
                "j",
                Checkpoint::SubmittedDeposit
            )
            .is_err()
        );
        let mut lower = submitted();
        lower["funding_jobs"][0]["target"] = json!("1999999");
        lower["pools"][0]["addresses"][1]["target"] = json!("1999999");
        assert!(
            continuity(
                &queued(),
                &lower,
                &lower,
                "p",
                "j",
                Checkpoint::QueuedRefill
            )
            .is_err()
        );
        assert!(
            continuity(
                &submitted(),
                &ready(),
                &submitted(),
                "p",
                "j",
                Checkpoint::SubmittedDeposit
            )
            .is_err()
        );
    }
}
