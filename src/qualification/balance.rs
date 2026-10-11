//! Admission-time accounting; never infer free balance from seller receipts.
use alloy_primitives::{B256, U256};
use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub fn validate_admission_balance(payment: &Value) -> Result<()> {
    if let Some(version) = payment.get("ownership_evidence_version") {
        ensure!(
            version.as_u64() == Some(1),
            "unknown admission ownership evidence version"
        );
        let slot = payment
            .get("reserve_slot")
            .context("admission reserve slot missing")?;
        let generation = payment
            .get("claim_generation")
            .context("admission claim generation missing")?;
        match payment["payer_role"].as_str() {
            Some("READY") => ensure!(
                matches!(slot.as_str(), Some("small" | "large")) && generation.as_u64().is_some(),
                "READY admission lacks exact reserve claim"
            ),
            Some("ACTIVE" | "RETAINED") => ensure!(
                slot.is_null() && generation.is_null(),
                "non-reserve admission carries a reserve claim"
            ),
            _ => anyhow::bail!("admission payer role is not payment eligible"),
        }
    }
    let evidence = &payment["balance_evidence"];
    let amount = |value: &Value| -> Result<U256> {
        Ok(value
            .as_str()
            .context("admission accounting amount missing")?
            .parse()?)
    };
    let cost = amount(&payment["amount"])?;
    let balance = amount(&evidence["confirmed_balance_atomic"])?;
    let before = amount(&evidence["reserved_before_atomic"])?;
    let after = amount(&evidence["reserved_after_atomic"])?;
    let available = amount(&evidence["available_after_atomic"])?;
    ensure!(
        cost > U256::ZERO && before.checked_add(cost) == Some(after),
        "admission exposure does not include exactly this payment"
    );
    ensure!(
        balance.checked_sub(after) == Some(available),
        "admission exposure exceeds confirmed balance or remaining balance disagrees"
    );
    let _: B256 = evidence["block_hash"]
        .as_str()
        .context("admission balance block hash missing")?
        .parse()?;
    ensure!(
        evidence["block_height"].as_u64().is_some(),
        "admission balance block height missing"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versioned_ownership_requires_exact_original_role_and_claim() {
        let mut payment = json!({"amount":"7","ownership_evidence_version":1,"payer_role":"READY","reserve_slot":"small","claim_generation":2,"balance_evidence":{"confirmed_balance_atomic":"20","reserved_before_atomic":"5","reserved_after_atomic":"12","available_after_atomic":"8","block_height":12,"block_hash":format!("0x{:064x}",1)}});
        validate_admission_balance(&payment).unwrap();
        for (field, value) in [
            ("ownership_evidence_version", json!(2)),
            ("payer_role", json!("RETIRED")),
            ("reserve_slot", Value::Null),
            ("claim_generation", json!(-1)),
        ] {
            let mut wrong = payment.clone();
            wrong[field] = value;
            assert!(validate_admission_balance(&wrong).is_err(), "{field}");
        }
        payment["payer_role"] = json!("RETAINED");
        assert!(validate_admission_balance(&payment).is_err());
        payment["reserve_slot"] = Value::Null;
        payment["claim_generation"] = Value::Null;
        validate_admission_balance(&payment).unwrap();
        payment.as_object_mut().unwrap().remove("reserve_slot");
        assert!(validate_admission_balance(&payment).is_err());
    }
    #[test]
    fn accounting_rejects_overspending_missing_exposure_and_arithmetic_overflow() {
        let payment = json!({"amount":"7","balance_evidence":{"confirmed_balance_atomic":"20","reserved_before_atomic":"5","reserved_after_atomic":"12","available_after_atomic":"8","block_height":12,"block_hash":format!("0x{:064x}",1)}});
        validate_admission_balance(&payment).unwrap();
        for (field, value) in [
            ("confirmed_balance_atomic", json!("11")),
            ("reserved_before_atomic", json!("0")),
            ("reserved_after_atomic", json!("7")),
            ("available_after_atomic", json!("9")),
            ("block_hash", Value::Null),
            ("block_height", json!(-1)),
            ("reserved_before_atomic", json!(U256::MAX.to_string())),
        ] {
            let mut wrong = payment.clone();
            wrong["balance_evidence"][field] = value;
            assert!(validate_admission_balance(&wrong).is_err(), "{field}");
        }
        assert!(validate_admission_balance(&json!({"amount":"7"})).is_err());
        let mut zero = payment;
        zero["amount"] = json!("0");
        assert!(validate_admission_balance(&zero).is_err());
    }
}
