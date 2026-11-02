//! Differences from registry authorization are bookkeeping changes, not run debits.
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn rows<'a>(data: &'a Value, key: &str, id: &str) -> BTreeMap<&'a str, &'a Value> {
    data[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[id].as_str().unwrap(), v))
        .collect()
}
fn difference(before: U256, after: U256) -> String {
    if after >= before {
        (after - before).to_string()
    } else {
        format!("-{}", before - after)
    }
}
fn total(summary: &Value) -> Result<U256> {
    summary["wallet_roles"]
        .as_object()
        .unwrap()
        .values()
        .try_fold(U256::ZERO, |sum, row| {
            sum.checked_add(
                row["saved_balance_atomic"]
                    .as_str()
                    .unwrap()
                    .parse::<U256>()?,
            )
            .context("baseline wallet total overflow")
        })
}
pub fn compare(snapshot: &Value, current: &Value) -> Result<Value> {
    ensure!(
        snapshot["treasury_status"]["treasury_id"] == current["treasury_id"],
        "baseline accounting treasury mismatch"
    );
    let before = crate::accounting::capture(snapshot)?;
    let old = crate::accounting::summarize(&before)?;
    let new = crate::accounting::summarize(current)?;
    let owners = rows(current, "source_preparations", "operation_id");
    for entry in before["source_preparations"].as_array().unwrap() {
        ensure!(
            owners
                .get(entry["operation_id"].as_str().unwrap())
                .is_some_and(|v| v["preparation_sha256"] == entry["preparation_sha256"]
                    && (entry["prepared_record_sha256"].is_null()
                        || v["prepared_record_sha256"] == entry["prepared_record_sha256"])),
            "baseline calculation changed or disappeared"
        );
    }
    let ops = rows(current, "operations", "operation_id");
    for op in before["operations"].as_array().unwrap() {
        let saved = ops
            .get(op["operation_id"].as_str().unwrap())
            .context("baseline source operation disappeared")?;
        ensure!(
            saved["facts"] == op["facts"],
            "baseline transaction facts changed"
        );
    }
    let mut deltas = serde_json::Map::new();
    for field in [
        "recorded_principal_zatoshis",
        "recorded_fees_zatoshis",
        "recorded_consumed_zatoshis",
        "source_reserved_zatoshis",
        "recorded_refund_outputs_zatoshis",
    ] {
        if old[field].is_null() || new[field].is_null() {
            deltas.insert(
                field.into(),
                json!({"baseline":old[field],"observed":new[field],"change":null}),
            );
            continue;
        }
        let a = old[field].as_u64().unwrap();
        let b = new[field].as_u64().unwrap();
        deltas.insert(
            field.into(),
            json!({"baseline":a,"observed":b,"change":(i128::from(b)-i128::from(a)).to_string()}),
        );
    }
    let wallets = {
        let old_wallets = rows(&before, "wallets", "id");
        let new_wallets = rows(current, "wallets", "id");
        ensure!(
            old_wallets.keys().all(|id| new_wallets.contains_key(id)),
            "baseline wallet disappeared"
        );
        ensure!(
            old_wallets
                .iter()
                .all(|(id, w)| new_wallets[id]["pool"] == w["pool"]),
            "baseline wallet pool changed"
        );
        let all_observed = |summary: &Value| {
            summary["wallet_roles"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v["unobserved_wallets"] == 0)
        };
        json!({"status":"recorded","baseline_roles":old["wallet_roles"],"observed_roles":new["wallet_roles"],
            "added_wallets":new_wallets.len()-old_wallets.len(),
            "aggregate_change_atomic":if all_observed(&old)&&all_observed(&new){json!(difference(total(&old)?,total(&new)?))}else{Value::Null}})
    };
    let zec = if !old["zec_observation"].is_null() && !new["zec_observation"].is_null() {
        let mut values = serde_json::Map::new();
        for key in ["confirmed_shielded_zatoshis", "spendable_shielded_zatoshis"] {
            let a = old["zec_observation"][key].as_u64().unwrap();
            let b = new["zec_observation"][key].as_u64().unwrap();
            values.insert(key.into(),json!({"baseline":a,"observed":b,"change":(i128::from(b)-i128::from(a)).to_string()}));
        }
        json!({"status":"recorded","balances":values})
    } else {
        json!({"status":"unobserved"})
    };
    Ok(
        json!({"scope":"registry_authorization_to_snapshot","fresh_chain_read":false,"source":deltas,"wallets":wallets,"zec":zec}),
    )
}

#[cfg(test)]
pub(crate) fn snapshot(data: &Value) -> Value {
    json!({"treasury_status":{"schema_version":6,"treasury_id":data["treasury_id"],"sync":data["sync"]},
        "lifecycle":{"schema_version":2,"source_operations":data["operations"]},
        "source_preparations":data["source_preparations"],"payment_attempts":data["payment_attempts"],"wallet_observations":data["wallets"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_preserves_original_calculation_and_prepared_record_identity() {
        let current = crate::accounting::fixture();
        let baseline = snapshot(&current);
        let unchanged = compare(&baseline, &current).unwrap();
        assert_eq!(
            unchanged["source"]["recorded_consumed_zatoshis"]["change"],
            "0"
        );
        assert!(unchanged["source"]["source_reserved_zatoshis"]["change"].is_null());
        assert!(unchanged["source"]["recorded_refund_outputs_zatoshis"]["change"].is_null());

        for field in ["preparation_sha256", "prepared_record_sha256"] {
            let mut changed = current.clone();
            changed["source_preparations"][0][field] = json!("f".repeat(64));
            assert_eq!(
                compare(&baseline, &changed).unwrap_err().to_string(),
                "baseline calculation changed or disappeared",
                "a well-formed but substituted {field} must not preserve continuity"
            );
        }
        let mut missing = current.clone();
        missing["source_preparations"].as_array_mut().unwrap().pop();
        assert_eq!(
            compare(&baseline, &missing).unwrap_err().to_string(),
            "baseline calculation changed or disappeared",
            "an accepted calculation without signed bytes must remain owned"
        );
    }

    #[test]
    fn pending_source_confirmation_adds_cost_without_recounting_history() {
        let after = crate::accounting::fixture();
        let mut before = after.clone();
        before["operations"][0]["submission"] = json!("PREPARED");
        let result = compare(&snapshot(&before), &after).unwrap();
        assert_eq!(
            result["source"]["recorded_consumed_zatoshis"]["change"],
            "105"
        );
        assert_eq!(result["source"]["recorded_fees_zatoshis"]["change"], "5");
        assert!(result["source"]["recorded_refund_outputs_zatoshis"]["change"].is_null());
        assert!(result["source"]["source_reserved_zatoshis"]["change"].is_null());
        let mut missing = after.clone();
        missing["operations"].as_array_mut().unwrap().remove(0);
        assert!(compare(&snapshot(&after), &missing).is_err());
    }
    #[test]
    fn saved_costs_and_unknown_estimates_are_not_run_debits() {
        let mut before = crate::accounting::fixture();
        before["wallets"][0]["balance"] = json!("20");
        let mut after = crate::accounting::fixture();
        after["sync"]["spendable_shielded_zatoshis"] = json!(40);
        let result = compare(&snapshot(&before), &after).unwrap();
        assert_eq!(
            result["source"]["recorded_consumed_zatoshis"]["change"],
            "0"
        );
        assert!(result["source"]["source_reserved_zatoshis"]["change"].is_null());
        assert_eq!(
            result["zec"]["balances"]["spendable_shielded_zatoshis"]["change"],
            "-10"
        );
        assert!(result["wallets"]["aggregate_change_atomic"].is_null());
        for data in [&mut before, &mut after] {
            data["wallets"][1]["balance"] = json!("0");
        }
        assert_eq!(
            compare(&snapshot(&before), &after).unwrap()["wallets"]["aggregate_change_atomic"],
            "-20"
        );
        let mut missing = snapshot(&before);
        missing
            .as_object_mut()
            .unwrap()
            .remove("wallet_observations");
        let retained = missing.clone();
        assert!(compare(&missing, &after).is_err());
        assert_eq!(missing, retained);
        after["operations"][0]["facts"]["fee_zatoshis"] = json!(6);
        assert!(compare(&snapshot(&before), &after).is_err());
        let mut changed = crate::accounting::fixture();
        changed["wallets"][0]["pool"] = json!("other-pool");
        assert!(compare(&snapshot(&crate::accounting::fixture()), &changed).is_err());
    }
}
