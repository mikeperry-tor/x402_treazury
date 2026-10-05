//! Source costs are attributed by immutable funding permits, never timestamp proximity.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn source(data: &Value, attempts: &[Value]) -> Result<Value> {
    crate::accounting::summarize(data)?;
    let entries: BTreeMap<_, _> = data["source_budget_entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["id"].as_str().unwrap(), v))
        .collect();
    let operations: BTreeSet<_> = data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["operation_id"].as_str().unwrap())
        .collect();
    let mut selected = BTreeSet::new();
    let (mut held, mut released, mut unrecorded) = (0u64, 0u64, 0u64);
    for attempt in attempts {
        let id = attempt["operation"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("missing attributed operation")?;
        ensure!(selected.insert(id), "duplicate attributed operation");
        let bound = attempt["source_bound"]
            .as_u64()
            .filter(|n| *n > 0)
            .context("invalid attributed source bound")?;
        let retired = attempt["released"]
            .as_bool()
            .context("invalid source release flag")?;
        let sum = if retired { &mut released } else { &mut held };
        *sum = sum
            .checked_add(bound)
            .context("attributed source bound overflow")?;
        if let Some(entry) = entries.get(id) {
            let cost = entry["consumed"].as_u64().unwrap();
            let reservation = entry["reserved"].as_u64().unwrap();
            ensure!(
                cost <= bound && reservation <= bound,
                "source accounting exceeds attributed permit"
            );
            ensure!(
                !retired || (cost == 0 && reservation == 0 && !operations.contains(id)),
                "released permit retains source liability"
            );
        } else {
            ensure!(
                !operations.contains(id),
                "attributed operation lacks budget evidence"
            );
            unrecorded += 1;
        }
    }
    let mut projected = data.clone();
    projected["source_budget_entries"]
        .as_array_mut()
        .unwrap()
        .retain(|v| selected.contains(v["id"].as_str().unwrap()));
    projected["operations"]
        .as_array_mut()
        .unwrap()
        .retain(|v| selected.contains(v["operation_id"].as_str().unwrap()));
    projected["refunds"]
        .as_array_mut()
        .unwrap()
        .retain(|v| selected.contains(v["operation_id"].as_str().unwrap()));
    let summary = crate::accounting::summarize(&projected)?;
    Ok(
        json!({"scope":"run_funding_permits", "operation_count":attempts.len(),
        "registry_unreleased_bounds_zatoshis":held,"registry_released_bounds_zatoshis":released,
        "attempts_without_treasury_budget":unrecorded,
        "recorded_consumed_zatoshis":summary["recorded_consumed_zatoshis"],
        "recorded_principal_zatoshis":summary["recorded_principal_zatoshis"],
        "recorded_fees_zatoshis":summary["recorded_fees_zatoshis"],
        "source_reserved_zatoshis":summary["source_reserved_zatoshis"],
        "recorded_refund_outputs_zatoshis":summary["recorded_refund_outputs_zatoshis"]}),
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
        assert_eq!(result["attempts_without_treasury_budget"], 1);
        assert_eq!(source(&data, &[]).unwrap()["recorded_consumed_zatoshis"], 0);
        for change in [
            json!({"operation":"deposit","source_bound":104,"released":false}),
            json!({"operation":"deposit","source_bound":110,"released":true}),
            attempts[0].clone(),
        ] {
            let mut invalid = attempts.clone();
            invalid.push(change);
            assert!(source(&data, &invalid).is_err());
        }
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
}
