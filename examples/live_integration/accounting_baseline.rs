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
    let has_wallets = snapshot.get("wallet_observations").is_some();
    let mut historical = snapshot.clone();
    // Old baselines retain source facts but not wallet observation anchors.
    // Exclude wallet/payment projection rather than inventing anchored balances.
    if !has_wallets {
        historical["wallet_observations"] = json!([]);
        historical["payment_attempts"] = json!([]);
    }
    let before = crate::accounting::capture(&historical)?;
    let old = crate::accounting::summarize(&before)?;
    let new = crate::accounting::summarize(current)?;
    let entries = rows(current, "source_budget_entries", "id");
    let ops = rows(current, "operations", "operation_id");
    for entry in before["source_budget_entries"].as_array().unwrap() {
        if entry["consumed"].as_u64().unwrap() > 0 {
            let saved = entries
                .get(entry["id"].as_str().unwrap())
                .context("baseline consumed operation disappeared")?;
            ensure!(
                saved["consumed"] == entry["consumed"],
                "baseline consumed cost changed"
            );
        }
    }
    for op in before["operations"].as_array().unwrap() {
        let saved = ops
            .get(op["operation_id"].as_str().unwrap())
            .context("baseline source operation disappeared")?;
        ensure!(
            saved["facts"] == op["facts"],
            "baseline transaction facts changed"
        );
    }
    let refunds: BTreeMap<_, _> = current["refunds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                (
                    v["txid"].as_str().unwrap(),
                    v["output_index"].as_u64().unwrap(),
                ),
                v,
            )
        })
        .collect();
    for refund in before["refunds"].as_array().unwrap() {
        let saved = refunds
            .get(&(
                refund["txid"].as_str().unwrap(),
                refund["output_index"].as_u64().unwrap(),
            ))
            .context("baseline refund disappeared")?;
        ensure!(
            saved["amount"] == refund["amount"] && saved["operation_id"] == refund["operation_id"],
            "baseline refund changed"
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
        let a = old[field].as_u64().unwrap();
        let b = new[field].as_u64().unwrap();
        deltas.insert(
            field.into(),
            json!({"baseline":a,"observed":b,"change":(i128::from(b)-i128::from(a)).to_string()}),
        );
    }
    let wallets = if has_wallets {
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
    } else {
        json!({"status":"unobserved","reason":"historical_baseline_lacks_wallet_anchors"})
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
    json!({"treasury_status":{"treasury_id":data["treasury_id"],"treasury_operations":data["operations"],"refunds":data["refunds"],"sync":data["sync"]},"source_budget_entries":data["source_budget_entries"],"payment_attempts":data["payment_attempts"],"wallet_observations":data["wallets"]})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_source_confirmation_adds_cost_without_recounting_history() {
        let after = crate::accounting::fixture();
        let mut before = after.clone();
        before["source_budget_entries"][0]["consumed"] = json!(0);
        before["source_budget_entries"][0]["reserved"] = json!(110);
        before["operations"][0]["submission"] = json!("PREPARED");
        before["refunds"] = json!([]);
        let result = compare(&snapshot(&before), &after).unwrap();
        assert_eq!(
            result["source"]["recorded_consumed_zatoshis"]["change"],
            "105"
        );
        assert_eq!(result["source"]["recorded_fees_zatoshis"]["change"], "5");
        assert_eq!(
            result["source"]["recorded_refund_outputs_zatoshis"]["change"],
            "10"
        );
        assert_eq!(
            result["source"]["source_reserved_zatoshis"]["change"],
            "-110"
        );
        let mut missing = after.clone();
        missing["refunds"] = json!([]);
        assert!(compare(&snapshot(&after), &missing).is_err());
    }
    #[test]
    fn historical_costs_and_unobserved_wallets_are_not_run_debits() {
        let mut before = crate::accounting::fixture();
        before["wallets"][0]["balance"] = json!("20");
        before["source_budget_entries"][2]["reserved"] = json!(100);
        let mut after = crate::accounting::fixture();
        after["sync"]["spendable_shielded_zatoshis"] = json!(40);
        let result = compare(&snapshot(&before), &after).unwrap();
        assert_eq!(
            result["source"]["recorded_consumed_zatoshis"]["change"],
            "0"
        );
        assert_eq!(
            result["source"]["source_reserved_zatoshis"]["change"],
            "-30"
        );
        assert_eq!(
            result["zec"]["balances"]["spendable_shielded_zatoshis"]["change"],
            "-10"
        );
        assert!(result["wallets"]["aggregate_change_atomic"].is_null());
        for data in [&mut before, &mut after] {
            data["wallets"][1]["height"] = json!(100);
            data["wallets"][1]["hash"] = json!("block");
        }
        assert_eq!(
            compare(&snapshot(&before), &after).unwrap()["wallets"]["aggregate_change_atomic"],
            "-20"
        );
        let mut historical = snapshot(&before);
        historical
            .as_object_mut()
            .unwrap()
            .remove("wallet_observations");
        assert_eq!(
            compare(&historical, &after).unwrap()["wallets"]["status"],
            "unobserved"
        );
        after["operations"][0]["facts"]["fee_zatoshis"] = json!(6);
        after["source_budget_entries"][0]["consumed"] = json!(106);
        assert!(compare(&snapshot(&before), &after).is_err());
    }
}
