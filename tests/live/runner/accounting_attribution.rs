//! Source costs are attributed by immutable funding permits, never timestamp proximity.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn source(data: &Value, attempts: &[Value]) -> Result<Value> {
    crate::accounting::summarize(data)?;
    let operations: BTreeMap<_, _> = data["operations"]
        .as_array()
        .context("missing operations")?
        .iter()
        .map(|v| (v["operation_id"].as_str().unwrap(), v))
        .collect();
    let owners: BTreeSet<_> = data["source_preparations"]
        .as_array()
        .context("missing preparations")?
        .iter()
        .map(|v| v["operation_id"].as_str().unwrap())
        .collect();
    let mut selected = BTreeSet::new();
    let (mut held, mut released, mut unknown) = (0u64, 0u64, 0u64);
    for attempt in attempts {
        let id = attempt["operation"]
            .as_str()
            .context("missing attributed operation")?;
        ensure!(selected.insert(id), "duplicate attributed operation");
        let bound = attempt["source_bound"]
            .as_u64()
            .filter(|v| *v > 0)
            .context("invalid source bound")?;
        let retired = attempt["released"]
            .as_bool()
            .context("invalid release flag")?;
        let sum = if retired { &mut released } else { &mut held };
        *sum = sum.checked_add(bound).context("source bound overflow")?;
        ensure!(
            !retired || !owners.contains(id),
            "released permit retains accepted calculation"
        );
        if let Some(op) = operations.get(id) {
            let cost = op["facts"]["amount_zatoshis"]
                .as_u64()
                .zip(op["facts"]["fee_zatoshis"].as_u64())
                .and_then(|(a, b)| a.checked_add(b))
                .context("invalid source facts")?;
            ensure!(
                cost <= bound,
                "actual source cost exceeds attributed permit"
            );
        } else {
            unknown += 1;
        }
    }
    let mut projected = data.clone();
    projected["operations"]
        .as_array_mut()
        .unwrap()
        .retain(|v| selected.contains(v["operation_id"].as_str().unwrap()));
    projected["source_preparations"]
        .as_array_mut()
        .unwrap()
        .retain(|v| selected.contains(v["operation_id"].as_str().unwrap()));
    let summary = crate::accounting::summarize(&projected)?;
    Ok(
        json!({"scope":"run_funding_permits","operation_count":attempts.len(),
        "registry_unreleased_bounds_zatoshis":held,"registry_released_bounds_zatoshis":released,
        "attempts_without_transaction_facts":unknown,"recorded_consumed_zatoshis":summary["recorded_consumed_zatoshis"],
        "recorded_principal_zatoshis":summary["recorded_principal_zatoshis"],"recorded_fees_zatoshis":summary["recorded_fees_zatoshis"],
        "source_reserved_zatoshis":null,"recorded_refund_outputs_zatoshis":null}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permit_identity_excludes_history_and_keeps_unrecorded_exposure() {
        let data = crate::accounting::fixture();
        let attempts = vec![
            json!({"operation":"deposit","source_bound":110,"released":false}),
            json!({"operation":"unprepared","source_bound":90,"released":false}),
        ];
        let result = source(&data, &attempts).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 105);
        assert_eq!(result["recorded_fees_zatoshis"], 5);
        assert_eq!(result["registry_unreleased_bounds_zatoshis"], 200);
        assert_eq!(result["attempts_without_transaction_facts"], 1);
        assert_eq!(source(&data, &[]).unwrap()["recorded_consumed_zatoshis"], 0);
        let mut duplicate = attempts.clone();
        duplicate.push(attempts[0].clone());
        assert!(source(&data, &duplicate).is_err());
        assert!(
            source(
                &data,
                &[json!({"operation":"deposit","source_bound":104,"released":false})]
            )
            .is_err()
        );
        assert!(
            source(
                &data,
                &[json!({"operation":"deposit","source_bound":110,"released":true})]
            )
            .is_err()
        );
    }
    #[test]
    fn calculation_only_ownership_and_released_bounds_remain_distinct() {
        let data = crate::accounting::fixture();
        let attempts = [
            json!({"operation":"calculation-only","source_bound":90,"released":false}),
            json!({"operation":"unused-permit","source_bound":30,"released":true}),
        ];
        let result = source(&data, &attempts).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 0);
        assert_eq!(result["registry_unreleased_bounds_zatoshis"], 90);
        assert_eq!(result["registry_released_bounds_zatoshis"], 30);
        assert_eq!(result["attempts_without_transaction_facts"], 2);
        let mut released = attempts.clone();
        released[0]["released"] = json!(true);
        assert!(source(&data, &released).is_err());
        let mut overflowing = attempts;
        overflowing[0]["source_bound"] = json!(u64::MAX);
        overflowing[1]["released"] = json!(false);
        let retained = data.clone();
        assert!(source(&data, &overflowing).is_err());
        assert_eq!(data, retained);
    }
}
