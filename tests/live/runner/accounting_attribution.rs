//! Source costs are attributed by immutable funding permits, never timestamp proximity.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn source(data: &Value, attempts: &[Value]) -> Result<Value> {
    crate::accounting::summarize(data)?;
    let operations: BTreeSet<_> = data["operations"]
        .as_array()
        .context("missing operations")?
        .iter()
        .map(|v| v["operation_id"].as_str().unwrap())
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
        // Retained for the registry schema/report contract: this is the first
        // proposal observation, which can precede Store acceptance and a retry
        // with different actual inputs. Only Store facts establish source cost.
        let proposed = attempt["input_zatoshis"]
            .as_u64()
            .filter(|v| *v > 0)
            .context("invalid first proposed source input")?;
        let retired = attempt["released"]
            .as_bool()
            .context("invalid release flag")?;
        let sum = if retired { &mut released } else { &mut held };
        *sum = sum.checked_add(proposed).context("source input total overflow")?;
        ensure!(
            !retired || !owners.contains(id),
            "released permit retains accepted calculation"
        );
        if !operations.contains(id) {
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
        "registry_unreleased_input_zatoshis":held,"registry_released_input_zatoshis":released,
        "attempts_without_transaction_facts":unknown,"recorded_consumed_zatoshis":summary["recorded_consumed_zatoshis"],
        "recorded_principal_zatoshis":summary["recorded_principal_zatoshis"],"recorded_fees_zatoshis":summary["recorded_fees_zatoshis"],
        "source_reserved_zatoshis":null,"recorded_refund_outputs_zatoshis":null}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permit_identity_excludes_history_and_keeps_proposals_separate_from_actual_costs() {
        let data = crate::accounting::fixture();
        let attempts = vec![
            json!({"operation":"deposit","input_zatoshis":104,"released":false}),
            json!({"operation":"unprepared","input_zatoshis":90,"released":false}),
        ];
        let result = source(&data, &attempts).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 105);
        assert_eq!(result["recorded_principal_zatoshis"], 100);
        assert_eq!(result["recorded_fees_zatoshis"], 5);
        assert_eq!(result["registry_unreleased_input_zatoshis"], 194);
        assert_eq!(result["attempts_without_transaction_facts"], 1);
        assert_eq!(source(&data, &[]).unwrap()["recorded_consumed_zatoshis"], 0);
        let mut duplicate = attempts.clone();
        duplicate.push(attempts[0].clone());
        assert!(source(&data, &duplicate).is_err());
        assert!(
            source(
                &data,
                &[json!({"operation":"deposit","input_zatoshis":110,"released":true})]
            )
            .is_err()
        );
    }
    #[test]
    fn calculation_only_ownership_and_released_observations_remain_distinct() {
        let data = crate::accounting::fixture();
        let attempts = [
            json!({"operation":"calculation-only","input_zatoshis":90,"released":false}),
            json!({"operation":"unused-permit","input_zatoshis":30,"released":true}),
        ];
        let result = source(&data, &attempts).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 0);
        assert_eq!(result["recorded_principal_zatoshis"], 0);
        assert_eq!(result["recorded_fees_zatoshis"], 0);
        assert_eq!(result["registry_unreleased_input_zatoshis"], 90);
        assert_eq!(result["registry_released_input_zatoshis"], 30);
        assert_eq!(result["attempts_without_transaction_facts"], 2);
        let mut released = attempts.clone();
        released[0]["released"] = json!(true);
        assert!(source(&data, &released).is_err());
        for invalid in [json!(0), json!(-1), json!("90")] {
            let mut malformed = attempts.clone();
            malformed[0]["input_zatoshis"] = invalid;
            assert!(source(&data, &malformed).is_err());
        }
        let mut overflowing = attempts;
        overflowing[0]["input_zatoshis"] = json!(u64::MAX);
        overflowing[1]["released"] = json!(false);
        let retained = data.clone();
        assert!(source(&data, &overflowing).is_err());
        assert_eq!(data, retained);
    }
}
