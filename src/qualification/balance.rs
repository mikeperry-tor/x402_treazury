//! Admission-time accounting; never infer free balance from seller receipts.
use alloy_primitives::{B256, U256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Version two distinguishes exact block balances from the conservative amount
/// used by admission. Both block anchors retain their original observations.
pub(crate) fn observed_balance(
    observation: &crate::rotation::base::evidence::BalanceObservation,
) -> Value {
    json!({
        "schema_version": 2,
        "confirmed_balance_atomic": observation.confirmed.amount.to_string(),
        "latest_balance_atomic": observation.latest.amount.to_string(),
        "conservative_balance_atomic": observation.available().to_string(),
        "block_height": observation.confirmed.anchor.height,
        "block_hash": observation.confirmed.anchor.hash,
        "latest_block_height": observation.latest.anchor.height,
        "latest_block_hash": observation.latest.anchor.hash,
    })
}

pub(crate) fn conservative_balance(evidence: &Value) -> Result<U256> {
    let amount = |field: &str| -> Result<U256> {
        Ok(evidence[field]
            .as_str()
            .context("balance amount missing")?
            .parse()?)
    };
    match evidence.get("schema_version") {
        // Historical records used this name for min(confirmed, latest). Reading
        // them must neither relabel it as an exact value nor increase capacity.
        None => amount("confirmed_balance_atomic"),
        Some(version) if version.as_u64() == Some(2) => {
            let confirmed = amount("confirmed_balance_atomic")?;
            let latest = amount("latest_balance_atomic")?;
            let conservative = amount("conservative_balance_atomic")?;
            ensure!(
                conservative == confirmed.min(latest),
                "conservative balance disagrees with exact observations"
            );
            let height = evidence["block_height"]
                .as_u64()
                .context("confirmed balance height missing")?;
            let latest_height = evidence["latest_block_height"]
                .as_u64()
                .context("latest balance height missing")?;
            ensure!(
                latest_height > height,
                "invalid balance observation heights"
            );
            for field in ["block_hash", "latest_block_hash"] {
                evidence[field]
                    .as_str()
                    .context("balance hash missing")?
                    .parse::<B256>()?;
            }
            Ok(conservative)
        }
        _ => anyhow::bail!("unknown balance evidence version"),
    }
}

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
    if payment["balance_evidence"]["schema_version"] == 3 {
        let cost: U256 = payment["amount"]
            .as_str()
            .context("admitted amount missing")?
            .parse()?;
        ensure!(cost > U256::ZERO, "zero payment amount");
        return Ok(());
    }
    let evidence = &payment["balance_evidence"];
    let amount = |value: &Value| -> Result<U256> {
        Ok(value
            .as_str()
            .context("admission accounting amount missing")?
            .parse()?)
    };
    let cost = amount(&payment["amount"])?;
    let balance = conservative_balance(evidence)?;
    let before = amount(&evidence["reserved_before_atomic"])?;
    let after = amount(&evidence["reserved_after_atomic"])?;
    let available = amount(&evidence["available_after_atomic"])?;
    ensure!(
        cost > U256::ZERO && before.checked_add(cost) == Some(after),
        "admission exposure does not include exactly this payment"
    );
    ensure!(
        balance.checked_sub(after) == Some(available),
        "admission exposure exceeds conservative balance or remaining balance disagrees"
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
    fn exact_balances_do_not_replace_conservative_admission_arithmetic() {
        let evidence = json!({"schema_version":2,"confirmed_balance_atomic":"80","latest_balance_atomic":"30","conservative_balance_atomic":"30","reserved_before_atomic":"5","reserved_after_atomic":"12","available_after_atomic":"18","block_height":12,"block_hash":format!("0x{:064x}",1),"latest_block_height":24,"latest_block_hash":format!("0x{:064x}",2)});
        let payment = json!({"amount":"7","balance_evidence":evidence});
        validate_admission_balance(&payment).unwrap();
        for (field, value) in [
            ("schema_version", json!(3)),
            ("latest_balance_atomic", Value::Null),
            ("conservative_balance_atomic", json!("80")),
            ("available_after_atomic", json!("68")),
            ("latest_block_height", json!(12)),
            ("latest_block_hash", Value::Null),
        ] {
            let mut wrong = payment.clone();
            wrong["balance_evidence"][field] = value;
            assert!(validate_admission_balance(&wrong).is_err(), "{field}");
        }
        let mut incoming = payment;
        incoming["balance_evidence"]["confirmed_balance_atomic"] = json!("30");
        incoming["balance_evidence"]["latest_balance_atomic"] = json!("80");
        validate_admission_balance(&incoming).unwrap();
    }

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
