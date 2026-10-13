//! Structural lifecycle checks over case-correlated durable observations.
//! These helpers grant no execution, allocation or payment authority.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "lifecycle_restart.rs"]
pub mod restart;

fn indexed<'a>(state: &'a Value, field: &str) -> Result<BTreeMap<&'a str, &'a Value>> {
    let mut result = BTreeMap::new();
    for value in state[field]
        .as_array()
        .context("lifecycle collection missing")?
    {
        let id = value["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("lifecycle identity missing")?;
        ensure!(
            result.insert(id, value).is_none(),
            "duplicate lifecycle identity"
        );
    }
    Ok(result)
}
fn role<'a>(wallets: &BTreeMap<&str, &'a Value>, role: &str) -> Result<&'a Value> {
    let mut matching = wallets.values().filter(|w| w["role"] == role);
    let found = *matching
        .next()
        .context("required lifecycle wallet role missing")?;
    ensure!(matching.next().is_none(), "duplicate lifecycle wallet role");
    Ok(found)
}
/// Verify exactly one natural promotion and return its unique replacement job ID.
pub fn promotion(before: &Value, after: &Value, pool: &str) -> Result<String> {
    if before["schema_version"] == 2 || after["schema_version"] == 2 {
        return capacity_consumption(before, after, pool)?.context("replacement not yet allocated");
    }
    ensure!(
        before["treasury_id"].is_string() && before["treasury_id"] == after["treasury_id"],
        "lifecycle treasury mismatch"
    );
    let old_pools = indexed(before, "pools")?;
    let new_pools = indexed(after, "pools")?;
    let old = old_pools.get(pool).context("previous pool missing")?;
    let new = new_pools.get(pool).context("promoted pool missing")?;
    let generation = old["generation"]
        .as_u64()
        .context("previous generation missing")?;
    ensure!(
        generation.checked_add(1) == new["generation"].as_u64() && old["name"] == new["name"],
        "promotion generation or pool identity mismatch"
    );
    let old_wallets = indexed(old, "addresses")?;
    let new_wallets = indexed(new, "addresses")?;
    let active = role(&old_wallets, "ACTIVE")?;
    let standby = role(&old_wallets, "READY")?;
    ensure!(
        new_wallets.len() == old_wallets.len() + 1,
        "promotion did not retain wallets and allocate exactly one replacement"
    );
    for (id, wallet) in &old_wallets {
        let saved = new_wallets.get(id).context("promotion lost a wallet")?;
        ensure!(
            saved["address"] == wallet["address"] && saved["target"] == wallet["target"],
            "promotion changed an immutable wallet identity/target"
        );
        let expected = if wallet["id"] == active["id"] {
            "RETIRED"
        } else if wallet["id"] == standby["id"] {
            "ACTIVE"
        } else {
            wallet["role"].as_str().context("wallet role missing")?
        };
        ensure!(
            saved["role"] == expected,
            "promotion assigned an unexpected wallet role"
        );
    }
    ensure!(
        role(&new_wallets, "ACTIVE")?["id"] == standby["id"],
        "funded standby did not become active"
    );
    let replacement = new_wallets
        .iter()
        .find(|(id, _)| !old_wallets.contains_key(*id))
        .context("replacement wallet missing")?
        .1;
    let address: alloy_primitives::Address = replacement["address"]
        .as_str()
        .context("replacement address missing")?
        .parse()?;
    for previous_pool in old_pools.values() {
        for wallet in indexed(previous_pool, "addresses")?.values() {
            ensure!(
                wallet["address"]
                    .as_str()
                    .context("previous address missing")?
                    .parse::<alloy_primitives::Address>()?
                    != address,
                "replacement reused an address"
            );
        }
    }
    ensure!(
        matches!(replacement["role"].as_str(), Some("ALLOCATED" | "READY")),
        "invalid replacement role"
    );
    let old_jobs = indexed(before, "funding_jobs")?;
    let new_jobs = indexed(after, "funding_jobs")?;
    for (id, job) in &old_jobs {
        let saved = new_jobs.get(id).context("promotion lost a funding job")?;
        for field in ["pool_id", "wallet_id", "recipient"] {
            ensure!(
                job[field] == saved[field],
                "promotion changed an existing funding binding"
            );
        }
    }
    let mut jobs = new_jobs
        .iter()
        .filter(|(id, j)| !old_jobs.contains_key(*id) && j["pool_id"] == pool);
    let (id, job) = jobs.next().context("replacement funding job missing")?;
    ensure!(
        jobs.next().is_none(),
        "promotion allocated more than one replacement job"
    );
    ensure!(
        job["wallet_id"] == replacement["id"]
            && job["recipient"] == replacement["address"]
            && job["target"] == replacement["target"],
        "replacement funding binding mismatch"
    );
    ensure!(
        replacement["target"]
            .as_str()
            .context("replacement target missing")?
            .parse::<alloy_primitives::U256>()?
            > alloy_primitives::U256::ZERO,
        "replacement target must be positive"
    );
    ensure!(
        (replacement["role"] == "READY") == (job["phase"] == "COMPLETE"),
        "replacement readiness disagrees with completed credit"
    );
    Ok((*id).to_owned())
}
/// Current reports correlate reserve consumption with its exact replacement slot.
/// Old immutable reports retain their original schema-specific interpretation.
pub fn consumed_reserve(before: &Value, after: &Value, pool: &str) -> Result<Option<String>> {
    if before["schema_version"] == 2 || after["schema_version"] == 2 {
        capacity_consumption(before, after, pool)
    } else {
        promotion(before, after, pool).map(Some)
    }
}
fn capacity_consumption(before: &Value, after: &Value, pool: &str) -> Result<Option<String>> {
    ensure!(
        before["schema_version"] == 2
            && after["schema_version"] == 2
            && before["treasury_id"].is_string()
            && before["treasury_id"] == after["treasury_id"],
        "capacity lifecycle schema/treasury mismatch"
    );
    let old_pools = indexed(before, "pools")?;
    let new_pools = indexed(after, "pools")?;
    let old = old_pools.get(pool).context("previous pool missing")?;
    let new = new_pools.get(pool).context("current pool missing")?;
    ensure!(
        old["name"] == new["name"] && old["generation"] == new["generation"],
        "capacity group identity changed"
    );
    let old_wallets = indexed(old, "addresses")?;
    let new_wallets = indexed(new, "addresses")?;
    let payer = role(&new_wallets, "ACTIVE")?;
    let previous = old_wallets
        .get(payer["id"].as_str().context("payer identity missing")?)
        .context("new payer was not existing inventory")?;
    ensure!(
        previous["role"] == "READY" && previous["destination_recorded"] == true,
        "new payer did not consume funded reserve inventory"
    );
    let replacement_claim = consumed_claim(old, new, payer)?;
    preserve_consumption_wallets(&old_wallets, &new_wallets, payer)?;
    if replacement_claim["wallet_id"].is_null() {
        validate_vacant_consumption(before, after, pool, &old_wallets, &new_wallets)?;
        return Ok(None);
    }
    validate_allocated_consumption(
        before,
        after,
        pool,
        &old_pools,
        &old_wallets,
        &new_wallets,
        &replacement_claim,
    )
}

fn reserve_claims(value: &Value) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    for claim in value["reserve_claims"]
        .as_array()
        .context("reserve claims missing")?
    {
        let slot = claim["slot"]
            .as_str()
            .filter(|s| matches!(*s, "small" | "large"))
            .context("invalid reserve slot")?;
        ensure!(
            result.insert(slot.to_owned(), claim.clone()).is_none(),
            "duplicate reserve slot"
        );
    }
    Ok(result)
}

fn consumed_claim(old: &Value, new: &Value, payer: &Value) -> Result<Value> {
    let old_claims = reserve_claims(old)?;
    let new_claims = reserve_claims(new)?;
    let (slot, claim) = old_claims
        .iter()
        .find(|(_, c)| c["wallet_id"] == payer["id"])
        .context("consumed payer has no original claim")?;
    ensure!(
        claim["enabled"] == true && claim["destination_ready"] == true,
        "consumed reserve was not ready"
    );
    let replacement_claim = new_claims
        .get(slot)
        .context("consumed reserve slot disappeared")?;
    ensure!(
        claim["generation"].as_u64().and_then(|g| g.checked_add(1))
            == replacement_claim["generation"].as_u64()
            && replacement_claim["enabled"] == true,
        "replacement claim generation mismatch"
    );
    for (other, saved) in &old_claims {
        if other != slot {
            ensure!(
                new_claims.get(other) == Some(saved),
                "unrelated reserve changed during measured consumption"
            );
        }
    }
    Ok(replacement_claim.clone())
}

fn preserve_consumption_wallets(
    old_wallets: &BTreeMap<&str, &Value>,
    new_wallets: &BTreeMap<&str, &Value>,
    payer: &Value,
) -> Result<()> {
    for (id, wallet) in old_wallets {
        let saved = new_wallets.get(id).context("consumption lost wallet")?;
        ensure!(
            saved["address"] == wallet["address"] && saved["target"] == wallet["target"],
            "consumption changed immutable wallet identity"
        );
        let expected = if wallet["id"] == payer["id"] {
            "ACTIVE"
        } else if wallet["role"] == "ACTIVE" {
            "RETAINED"
        } else {
            wallet["role"].as_str().context("wallet role missing")?
        };
        let later_retirement = if expected == "RETAINED" && saved["role"] == "RETIRED" {
            let record = &saved["retirement"];
            let amount = |field: &str| -> Result<alloy_primitives::U256> {
                Ok(record[field]
                    .as_str()
                    .context("retirement support missing")?
                    .parse()?)
            };
            let remainder = amount("confirmed_amount")?.max(amount("latest_amount")?);
            ensure!(
                remainder == amount("observed_bound")? && remainder <= amount("accepted_maximum")?,
                "retirement remainder support mismatch"
            );
            true
        } else {
            false
        };
        ensure!(
            saved["role"] == expected || later_retirement,
            "unexpected consumption role transition"
        );
    }
    Ok(())
}

fn validate_vacant_consumption(
    before: &Value,
    after: &Value,
    pool: &str,
    old_wallets: &BTreeMap<&str, &Value>,
    new_wallets: &BTreeMap<&str, &Value>,
) -> Result<()> {
    ensure!(
        new_wallets.len() == old_wallets.len(),
        "vacant consumption changed inventory"
    );
    let old_jobs = indexed(before, "funding_jobs")?;
    let new_jobs = indexed(after, "funding_jobs")?;
    ensure!(
        !new_jobs
            .iter()
            .any(|(id, job)| !old_jobs.contains_key(*id) && job["pool_id"] == pool),
        "vacant consumption unexpectedly allocated selected inventory"
    );
    for (id, job) in &old_jobs {
        let saved = new_jobs
            .get(id)
            .context("vacant consumption lost funding history")?;
        for field in ["pool_id", "wallet_id", "recipient", "target"] {
            ensure!(
                job[field] == saved[field],
                "vacant consumption changed funding identity"
            );
        }
    }
    Ok(())
}

fn validate_allocated_consumption(
    before: &Value,
    after: &Value,
    pool: &str,
    old_pools: &BTreeMap<&str, &Value>,
    old_wallets: &BTreeMap<&str, &Value>,
    new_wallets: &BTreeMap<&str, &Value>,
    replacement_claim: &Value,
) -> Result<Option<String>> {
    let replacement_id = replacement_claim["wallet_id"]
        .as_str()
        .context("replacement not yet allocated")?;
    let replacement = new_wallets
        .get(replacement_id)
        .context("replacement wallet missing")?;
    ensure!(
        !old_wallets.contains_key(replacement_id) && new_wallets.len() == old_wallets.len() + 1,
        "consumption did not retain inventory and allocate exactly its replacement"
    );
    let address: alloy_primitives::Address = replacement["address"]
        .as_str()
        .context("replacement address missing")?
        .parse()?;
    for previous_pool in old_pools.values() {
        for wallet in indexed(previous_pool, "addresses")?.values() {
            ensure!(
                wallet["address"]
                    .as_str()
                    .context("previous address missing")?
                    .parse::<alloy_primitives::Address>()?
                    != address,
                "replacement reused an address"
            );
        }
    }
    let old_jobs = indexed(before, "funding_jobs")?;
    let new_jobs = indexed(after, "funding_jobs")?;
    for (id, job) in &old_jobs {
        let saved = new_jobs.get(id).context("consumption lost funding job")?;
        for field in ["pool_id", "wallet_id", "recipient", "target"] {
            ensure!(job[field] == saved[field], "funding identity changed");
        }
    }
    let mut additions = new_jobs
        .iter()
        .filter(|(id, j)| !old_jobs.contains_key(*id) && j["pool_id"] == pool);
    let (id, job) = additions
        .next()
        .context("replacement funding job missing")?;
    ensure!(
        additions.next().is_none()
            && job["wallet_id"] == replacement_id
            && job["recipient"] == replacement["address"]
            && job["target"] == replacement["target"],
        "replacement funding correlation differs"
    );
    ensure!(
        matches!(replacement["role"].as_str(), Some("ALLOCATED" | "READY")),
        "invalid replacement role"
    );
    ensure!(
        (replacement["role"] == "READY") == (replacement["destination_recorded"] == true)
            && (replacement["role"] != "READY" || replacement_claim["destination_ready"] == true),
        "replacement readiness lacks exact destination support"
    );
    Ok(Some((*id).to_owned()))
}

fn pending(job: &Value) -> bool {
    matches!(
        job["phase"].as_str(),
        Some(
            "ALLOCATED"
                | "QUOTED"
                | "PREPARING"
                | "PREPARED"
                | "DEPOSIT_PENDING"
                | "SWAPPING"
                | "VERIFYING_CREDIT"
        )
    )
}
/// A conservative full signed-request interval inside an observed pending job.
/// A fast completion or missing observation returns None, never manufactured success.
/// The caller must authenticate the canonical debit record against the run journal;
/// this structural check cannot establish chain provenance from JSON alone.
pub fn refill_service(
    payment: &Value,
    receipt: &Value,
    debit: &Value,
    job: &str,
) -> Result<Option<Value>> {
    for field in ["case", "session", "attempt_id"] {
        ensure!(
            payment[field].as_str().is_some_and(|s| !s.is_empty())
                && payment[field] == receipt[field]
                && payment[field] == debit[field],
            "refill service payment correlation mismatch"
        );
    }
    let response = &receipt["receipt"];
    ensure!(
        response["classification"] == "seller_success"
            && debit["status"] == "verified"
            && debit["proof"].is_object(),
        "refill service requires a verified paid success"
    );
    let proof = &debit["proof"];
    let payer: alloy_primitives::Address = proof["payer"]
        .as_str()
        .context("refill proof payer missing")?
        .parse()?;
    ensure!(
        payer
            == payment["address"]
                .as_str()
                .context("refill admission payer missing")?
                .parse::<alloy_primitives::Address>()?
            && proof["amount_atomic"] == payment["amount"]
            && proof["transaction"] == response["transaction"],
        "refill debit differs from payment or receipt"
    );
    let before = &payment["lifecycle"];
    let observation = &response["lifecycle"];
    if before.is_null() || observation.get("state").is_none() {
        return Ok(None);
    }
    let after = &observation["state"];
    ensure!(
        before["treasury_id"].is_string() && before["treasury_id"] == after["treasury_id"],
        "refill observation treasury mismatch"
    );
    let old_jobs = indexed(before, "funding_jobs")?;
    let new_jobs = indexed(after, "funding_jobs")?;
    let Some(old) = old_jobs.get(job) else {
        return Ok(None);
    };
    let new = new_jobs
        .get(job)
        .context("observed refill job disappeared")?;
    for field in ["pool_id", "wallet_id", "recipient", "operation_id"] {
        ensure!(
            !old[field].is_null() && old[field] == new[field],
            "refill job binding or operation changed during call"
        );
    }
    if !pending(old) || !pending(new) {
        return Ok(None);
    }
    let times = [
        payment["admitted_micros"].as_u64(),
        response["signed_request_started_micros"].as_u64(),
        response["response_observed_micros"].as_u64(),
        observation["observed_micros"].as_u64(),
    ];
    let Some(times) = times.into_iter().collect::<Option<Vec<_>>>() else {
        return Ok(None);
    };
    ensure!(
        times.windows(2).all(|w| w[0] <= w[1]),
        "refill observation clocks disagree"
    );
    ensure!(
        times[1] < times[2],
        "refill signed-request interval has no duration"
    );
    Ok(Some(
        json!({"case":payment["case"],"job":job,"pool":old["pool_id"],"payment_pool":payment["pool"],"pending_before":old["phase"],"pending_after":new["phase"],"scope":"verified paid signed-request future through final headers bracketed by same-job pending observations in one application session"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("lifecycle_capacity_tests.rs");
    fn wallet(id: &str, n: u64, role: &str) -> Value {
        json!({"id":id,"address":format!("0x{n:040x}"),"role":role,"target":"2000000"})
    }
    fn state(generation: u64) -> Value {
        json!({"treasury_id":"treasury","pools":[{"id":"p","name":"pool","generation":generation,"addresses":[wallet("old",1,"ACTIVE"),wallet("ready",2,"READY")]}],"funding_jobs":[]})
    }
    fn promoted() -> (Value, Value) {
        let before = state(0);
        let mut after = state(1);
        after["pools"][0]["addresses"] = json!([
            wallet("old", 1, "RETIRED"),
            wallet("ready", 2, "ACTIVE"),
            wallet("new", 3, "ALLOCATED")
        ]);
        after["funding_jobs"] = json!([{"id":"replacement","pool_id":"p","wallet_id":"new","recipient":format!("0x{:040x}",3),"target":"2000000","operation_id":"op","phase":"ALLOCATED"}]);
        (before, after)
    }
    #[test]
    fn capacity_consumption_uses_exact_slot_and_destination_not_source_completion() {
        let (before, mut after) = capacity_fixture();
        let mut vacant = after.clone();
        vacant["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .pop();
        vacant["pools"][0]["reserve_claims"][0]["wallet_id"] = Value::Null;
        vacant["funding_jobs"] = json!([]);
        assert_eq!(consumed_reserve(&before, &vacant, "p").unwrap(), None);
        assert!(promotion(&before, &vacant, "p").is_err());
        assert_eq!(promotion(&before, &after, "p").unwrap(), "replacement");
        after["pools"][0]["addresses"][2]["role"] = json!("READY");
        after["pools"][0]["addresses"][2]["destination_recorded"] = json!(true);
        after["pools"][0]["reserve_claims"][0]["destination_ready"] = json!(true);
        after["funding_jobs"][0]["phase"] = json!("SWAPPING");
        assert_eq!(promotion(&before, &after, "p").unwrap(), "replacement");
        // A fresh reserve-only pool has no previous ACTIVE payer to retain.
        let mut fresh_before = before.clone();
        let mut fresh_after = after.clone();
        fresh_before["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        fresh_after["pools"][0]["addresses"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert_eq!(
            promotion(&fresh_before, &fresh_after, "p").unwrap(),
            "replacement"
        );
        after["pools"][0]["reserve_claims"][0]["generation"] = json!(4);
        assert!(promotion(&before, &after, "p").is_err());
    }
    #[test]
    fn exactly_one_promotion_preserves_old_identity_and_binds_new_recipient() {
        let (before, after) = promoted();
        assert_eq!(promotion(&before, &after, "p").unwrap(), "replacement");
        for pointer in [
            "/pools/0/generation",
            "/pools/0/addresses/0/role",
            "/pools/0/addresses/1/address",
            "/funding_jobs/0/wallet_id",
            "/funding_jobs/0/recipient",
        ] {
            let mut wrong = after.clone();
            *wrong.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(promotion(&before, &wrong, "p").is_err(), "{pointer}");
        }
        let mut wrong = after.clone();
        wrong["pools"][0]["addresses"][2]["address"] =
            before["pools"][0]["addresses"][0]["address"].clone();
        assert!(promotion(&before, &wrong, "p").is_err());
        let mut wrong = after.clone();
        let mut job = wrong["funding_jobs"][0].clone();
        job["id"] = json!("extra");
        wrong["funding_jobs"].as_array_mut().unwrap().push(job);
        assert!(promotion(&before, &wrong, "p").is_err());
        let mut fast = after.clone();
        fast["pools"][0]["addresses"][2]["role"] = json!("READY");
        fast["funding_jobs"][0]["phase"] = json!("COMPLETE");
        promotion(&before, &fast, "p").unwrap();
    }
    #[test]
    fn promotion_rejects_reusing_another_pools_active_or_historical_address() {
        for role in ["ACTIVE", "READY", "RETIRED"] {
            let (mut before, mut after) = promoted();
            let other = json!({"id":"other","name":"other_pool","generation":0,
                "addresses":[wallet("other_wallet",3,role)]});
            before["pools"].as_array_mut().unwrap().push(other.clone());
            after["pools"].as_array_mut().unwrap().push(other);
            let error = promotion(&before, &after, "p").unwrap_err();
            assert!(
                error.to_string().contains("reused an address"),
                "{role}: {error}"
            );

            // Independent pools may remain present without blocking a fresh address.
            before["pools"][1]["addresses"][0] = wallet("other_wallet", 4, role);
            after["pools"][1]["addresses"][0] = wallet("other_wallet", 4, role);
            assert_eq!(promotion(&before, &after, "p").unwrap(), "replacement");
        }
    }
    #[test]
    fn paid_service_requires_both_pending_observations_and_ordered_correlated_times() {
        let (_, state) = promoted();
        let payment = json!({"case":"call","session":"s","attempt_id":"a","pool":"other_pool","admitted_micros":10,"amount":"7","address":format!("0x{:040x}",4),"lifecycle":state});
        let receipt = json!({"case":"call","session":"s","attempt_id":"a","receipt":{"classification":"seller_success","transaction":"tx","signed_request_started_micros":20,"response_observed_micros":30,"lifecycle":{"observed_micros":40,"state":state}}});
        let debit = json!({"case":"call","session":"s","attempt_id":"a","status":"verified","proof":{"payer":payment["address"],"amount_atomic":"7","transaction":"tx"}});
        let result = refill_service(&payment, &receipt, &debit, "replacement")
            .unwrap()
            .unwrap();
        assert_eq!(result["payment_pool"], "other_pool");
        let mut fast = receipt.clone();
        fast["receipt"]["lifecycle"]["state"]["funding_jobs"][0]["phase"] = json!("COMPLETE");
        assert!(
            refill_service(&payment, &fast, &debit, "replacement")
                .unwrap()
                .is_none()
        );
        let mut missing = receipt.clone();
        missing["receipt"]["lifecycle"] = json!({"status":"unavailable"});
        assert!(
            refill_service(&payment, &missing, &debit, "replacement")
                .unwrap()
                .is_none()
        );
        for pointer in [
            "/session",
            "/receipt/lifecycle/state/funding_jobs/0/operation_id",
            "/receipt/lifecycle/state/funding_jobs/0/recipient",
        ] {
            let mut wrong = receipt.clone();
            *wrong.pointer_mut(pointer).unwrap() = json!("changed");
            assert!(
                refill_service(&payment, &wrong, &debit, "replacement").is_err(),
                "{pointer}"
            );
        }
        let mut wrong = receipt.clone();
        wrong["receipt"]["lifecycle"]["observed_micros"] = json!(25);
        assert!(refill_service(&payment, &wrong, &debit, "replacement").is_err());
        let mut wrong = debit.clone();
        wrong["proof"]["amount_atomic"] = json!("8");
        assert!(refill_service(&payment, &receipt, &wrong, "replacement").is_err());
    }
}
