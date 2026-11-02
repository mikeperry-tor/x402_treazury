//! Persisted treasury-wide bookkeeping, never fresh chain evidence or run attribution.
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn number(v: &Value) -> Result<u64> {
    v.as_u64().context("invalid accounting amount")
}
fn plus(total: &mut u64, amount: u64) -> Result<()> {
    *total = total
        .checked_add(amount)
        .context("accounting sum overflow")?;
    Ok(())
}
fn rows<'a>(v: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    v[field].as_array().context("missing accounting rows")
}
fn id(v: &Value) -> Result<&str> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .context("missing accounting identity")
}
fn amount(v: &Value) -> Result<U256> {
    let text = v.as_str().context("missing atomic balance")?;
    ensure!(
        !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()),
        "invalid atomic balance"
    );
    Ok(text.parse()?)
}
pub fn capture(snapshot: &Value) -> Result<Value> {
    ensure!(
        snapshot["lifecycle"]["schema_version"] == 2,
        "unsupported accounting lifecycle"
    );
    let status = &snapshot["treasury_status"];
    let sync = &status["sync"];
    let data = json!({"schema_version":2,"treasury_id":status["treasury_id"],
        "source_preparations":snapshot["source_preparations"],
        "operations":snapshot["lifecycle"]["source_operations"],"wallets":snapshot["wallet_observations"],
        "payment_attempts":snapshot["payment_attempts"],
        "sync":if sync.is_null(){Value::Null}else{json!({"height":sync["height"],"checked_at":sync["checked_at"],"confirmed_shielded_zatoshis":sync["confirmed_shielded_zatoshis"],"spendable_shielded_zatoshis":sync["spendable_shielded_zatoshis"]})}});
    summarize(&data)?;
    Ok(data)
}
#[derive(Default)]
struct SourceTotals {
    consumed: u64,
    principal: u64,
    fees: u64,
}

// Original calculation identities and actual confirmed transaction facts own this projection.
fn source_accounting(data: &Value) -> Result<SourceTotals> {
    let mut owners = BTreeSet::new();
    for entry in rows(data, "source_preparations")? {
        ensure!(
            owners.insert(id(&entry["operation_id"])?),
            "duplicate source preparation"
        );
        let digest = id(&entry["preparation_sha256"])?;
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid preparation digest"
        );
    }
    let mut totals = SourceTotals::default();
    let mut operations = BTreeSet::new();
    for op in rows(data, "operations")? {
        let key = id(&op["operation_id"])?;
        ensure!(operations.insert(key), "duplicate source operation");
        ensure!(
            owners.contains(key),
            "prepared operation lacks original calculation"
        );
        let principal = number(&op["facts"]["amount_zatoshis"])?;
        let fee = number(&op["facts"]["fee_zatoshis"])?;
        if op["submission"] == "CONFIRMED" {
            plus(&mut totals.principal, principal)?;
            plus(&mut totals.fees, fee)?;
            plus(
                &mut totals.consumed,
                principal
                    .checked_add(fee)
                    .context("source facts overflow")?,
            )?;
        }
    }
    Ok(totals)
}

#[derive(Default)]
struct RoleBalances {
    wallets: u64,
    estimates: u64,
    saved_balance: U256,
}

struct WalletObservations<'a> {
    identities: BTreeSet<&'a str>,
    roles: BTreeMap<String, RoleBalances>,
}

fn wallet_observations(data: &Value) -> Result<WalletObservations<'_>> {
    let mut wallets = BTreeSet::new();
    let mut roles = BTreeMap::<String, RoleBalances>::new();
    for wallet in rows(data, "wallets")? {
        ensure!(
            wallets.insert(id(&wallet["id"])?),
            "duplicate wallet observation"
        );
        let role = id(&wallet["role"])?;
        ensure!(
            matches!(
                role,
                "ALLOCATED" | "ACTIVE" | "READY" | "RETAINED" | "RETIRED"
            ),
            "unknown wallet role"
        );
        let bucket = roles.entry(role.to_owned()).or_default();
        plus(&mut bucket.wallets, 1)?;
        if let Ok(estimate) = amount(&wallet["balance"]) {
            plus(&mut bucket.estimates, 1)?;
            bucket.saved_balance = bucket
                .saved_balance
                .checked_add(estimate)
                .context("USDC balance sum overflow")?;
        }
    }
    Ok(WalletObservations {
        identities: wallets,
        roles,
    })
}

fn payment_exposure(data: &Value, wallets: &BTreeSet<&str>) -> Result<U256> {
    let mut exposure = U256::ZERO;
    let mut attempts = BTreeSet::new();
    for attempt in rows(data, "payment_attempts")? {
        ensure!(
            attempts.insert(id(&attempt["id"])?),
            "duplicate payment attempt"
        );
        ensure!(
            matches!(
                attempt["state"].as_str(),
                Some("POSSIBLY_SUBMITTED" | "RESOLVED")
            ),
            "unknown payment state"
        );
        ensure!(
            wallets.contains(id(&attempt["wallet"])?),
            "payment references missing wallet"
        );
        amount(&attempt["amount"])?;
        if attempt["state"] != "RESOLVED" {
            exposure = exposure
                .checked_add(amount(&attempt["amount"])?)
                .context("payment exposure overflow")?;
        }
    }
    Ok(exposure)
}

pub fn summarize(data: &Value) -> Result<Value> {
    ensure!(data["schema_version"] == 2, "unsupported accounting schema");
    let source = source_accounting(data)?;
    let wallets = wallet_observations(data)?;
    let exposure = payment_exposure(data, &wallets.identities)?;
    let sync = &data["sync"];
    if !sync.is_null() {
        number(&sync["confirmed_shielded_zatoshis"])?;
        number(&sync["spendable_shielded_zatoshis"])?;
    }
    Ok(
        json!({"scope":"treasury_wide_persisted","fresh_chain_read":false,
        "source_reserved_zatoshis":null,"recorded_consumed_zatoshis":source.consumed,
        "recorded_principal_zatoshis":source.principal,"recorded_fees_zatoshis":source.fees,"recorded_refund_outputs_zatoshis":null,
        "unresolved_payment_exposure_atomic":exposure.to_string(),"zec_observation":sync,
        "wallet_roles":wallets.roles.into_iter().map(|(role,balance)|(role,json!({"wallets":balance.wallets,"observation_model":"advisory_estimate","available_estimates":balance.estimates,"unobserved_wallets":balance.wallets-balance.estimates,"saved_balance_atomic":balance.saved_balance.to_string()}))).collect::<BTreeMap<_,_>>() }),
    )
}

#[cfg(test)]
pub(crate) fn fixture() -> Value {
    json!({"schema_version":2,"treasury_id":"private-treasury",
        "source_preparations":[
            {"operation_id":"deposit","preparation_sha256":"a".repeat(64),"prepared_record_sha256":"d".repeat(64)},
            {"operation_id":"shield","preparation_sha256":"b".repeat(64),"prepared_record_sha256":"e".repeat(64)},
            {"operation_id":"calculation-only","preparation_sha256":"c".repeat(64),"prepared_record_sha256":null}],
        "operations":[{"operation_id":"deposit","submission":"CONFIRMED","facts":{"amount_zatoshis":100,"fee_zatoshis":5}},
        {"operation_id":"shield","submission":"CONFIRMED","facts":{"amount_zatoshis":0,"fee_zatoshis":3}}],
        "wallets":[{"id":"private-active","pool":"pool","role":"ACTIVE","balance":"0","height":null,"hash":null,"observation_model":"advisory_estimate"},
        {"id":"private-allocated","pool":"pool","role":"ALLOCATED","balance":null,"height":null,"hash":null,"observation_model":"advisory_estimate"},
        {"id":"private-retired","pool":"pool","role":"RETIRED","balance":"7","height":null,"hash":null,"observation_model":"advisory_estimate"}],
        "payment_attempts":[{"id":"a","wallet":"private-active","state":"POSSIBLY_SUBMITTED","amount":"2"},{"id":"b","wallet":"private-retired","state":"POSSIBLY_SUBMITTED","amount":"3"},{"id":"c","wallet":"private-active","state":"RESOLVED","amount":"100"}],
        "sync":{"height":200,"checked_at":10,"confirmed_shielded_zatoshis":60,"spendable_shielded_zatoshis":50}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_facts_keep_deleted_projections_unavailable_and_shield_fee_only() {
        let mut data = fixture();
        let result = summarize(&data).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 108);
        assert_eq!(result["recorded_principal_zatoshis"], 100);
        assert_eq!(result["recorded_fees_zatoshis"], 8);
        assert!(result["source_reserved_zatoshis"].is_null());
        assert!(result["recorded_refund_outputs_zatoshis"].is_null());
        assert_eq!(result["unresolved_payment_exposure_atomic"], "5");
        assert_eq!(result["fresh_chain_read"], false);
        data["operations"][0]["submission"] = json!("UNKNOWN");
        assert_eq!(summarize(&data).unwrap()["recorded_consumed_zatoshis"], 3);
    }
    #[test]
    fn empty_current_inventory_does_not_invent_saved_balances_or_costs() {
        let data = json!({"schema_version":2,"source_preparations":[],"operations":[],
            "wallets":[],"payment_attempts":[],"sync":null});
        let summary = summarize(&data).unwrap();
        assert_eq!(summary["recorded_consumed_zatoshis"], 0);
        assert_eq!(summary["unresolved_payment_exposure_atomic"], "0");
        assert_eq!(summary["wallet_roles"], json!({}));
        assert!(summary["zec_observation"].is_null());
        assert!(summary["source_reserved_zatoshis"].is_null());
        assert!(summary["recorded_refund_outputs_zatoshis"].is_null());
    }
    #[test]
    fn advisory_zero_unknown_and_retained_balances_stay_distinct_from_retired() {
        let mut data = fixture();
        data["wallets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"retained","role":"RETAINED","balance":"9"}));
        let report = summarize(&data).unwrap();
        assert_eq!(report["wallet_roles"]["ACTIVE"]["available_estimates"], 1);
        assert_eq!(report["wallet_roles"]["ALLOCATED"]["unobserved_wallets"], 1);
        assert_eq!(
            report["wallet_roles"]["RETAINED"]["saved_balance_atomic"],
            "9"
        );
        assert_eq!(
            report["wallet_roles"]["RETIRED"]["saved_balance_atomic"],
            "7"
        );
        for malformed in [json!("-1"), json!("9".repeat(100))] {
            data["wallets"][0]["balance"] = malformed;
            assert_eq!(
                summarize(&data).unwrap()["wallet_roles"]["ACTIVE"]["unobserved_wallets"],
                1
            );
        }
    }
    #[test]
    fn unsupported_or_contradictory_evidence_fails_without_mutation() {
        for (path, value) in [
            ("/schema_version", json!(1)),
            (
                "/source_preparations/0/preparation_sha256",
                json!("not-a-digest"),
            ),
            ("/source_preparations/0/operation_id", json!("missing")),
            ("/operations/1/operation_id", json!("deposit")),
            ("/operations/0/facts/fee_zatoshis", json!(-1)),
            ("/wallets/1/id", json!("private-active")),
            ("/wallets/0/role", json!("OTHER")),
            ("/payment_attempts/1/id", json!("a")),
            ("/payment_attempts/0/state", json!("ADMITTED")),
            ("/payment_attempts/0/amount", json!("9".repeat(100))),
            ("/payment_attempts/0/wallet", json!("unknown")),
            ("/sync/spendable_shielded_zatoshis", Value::Null),
        ] {
            let mut data = fixture();
            *data.pointer_mut(path).unwrap() = value;
            let retained = data.clone();
            assert!(summarize(&data).is_err(), "{path}");
            assert_eq!(data, retained);
        }
        let mut data = fixture();
        let entry = data["source_preparations"][0].clone();
        data["source_preparations"]
            .as_array_mut()
            .unwrap()
            .push(entry);
        assert!(summarize(&data).is_err());
    }
    #[test]
    fn capture_requires_current_lifecycle_and_original_calculations() {
        let data = fixture();
        let mut snapshot = crate::accounting_baseline::snapshot(&data);
        assert_eq!(capture(&snapshot).unwrap(), data);
        snapshot["lifecycle"]["schema_version"] = json!(5);
        let retained = snapshot.clone();
        assert!(capture(&snapshot).is_err());
        assert_eq!(snapshot, retained);
        snapshot["lifecycle"]["schema_version"] = json!(2);
        snapshot
            .as_object_mut()
            .unwrap()
            .remove("source_preparations");
        assert!(capture(&snapshot).is_err());
    }
    #[test]
    fn checked_source_totals_and_unresolved_atomic_exposure_reject_overflow() {
        let mut data = fixture();
        data["operations"][0]["facts"]["amount_zatoshis"] = json!(u64::MAX - 5);
        let shield = data["operations"].as_array_mut().unwrap().pop().unwrap();
        assert_eq!(
            summarize(&data).unwrap()["recorded_consumed_zatoshis"],
            u64::MAX
        );
        data["operations"].as_array_mut().unwrap().push(shield);
        assert!(summarize(&data).is_err());
        data["operations"].as_array_mut().unwrap().pop();
        data["operations"][0]["facts"]["amount_zatoshis"] = json!(u64::MAX);
        assert!(summarize(&data).is_err());
        let mut data = fixture();
        data["wallets"][0]["balance"] = json!(U256::MAX.to_string());
        data["wallets"][1]["role"] = json!("ACTIVE");
        data["wallets"][1]["balance"] = json!("1");
        assert!(summarize(&data).is_err());
        data["wallets"][1]["balance"] = Value::Null;
        data["payment_attempts"][0]["amount"] = json!(U256::MAX.to_string());
        data["payment_attempts"][1]["state"] = json!("RESOLVED");
        assert_eq!(
            summarize(&data).unwrap()["unresolved_payment_exposure_atomic"],
            U256::MAX.to_string()
        );
        data["payment_attempts"][1]["state"] = json!("POSSIBLY_SUBMITTED");
        assert!(summarize(&data).is_err());
    }
}
