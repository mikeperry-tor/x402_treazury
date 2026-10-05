//! Checked estimates only. Canonical observations and actual challenges remain authoritative.
use alloy_primitives::{B256, U256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Estimate calls through the first promotion-triggering payment at a reviewed fixed price.
/// Caller authenticates the observation and checks freshness before dispatch; this
/// calculation alone grants no spending or assertion of future provider pricing.
pub fn depletion_estimate(
    observation: &Value,
    expected_price_atomic: &str,
    planned_calls: usize,
) -> Result<Value> {
    let price = number(expected_price_atomic)?;
    ensure!(price > U256::ZERO, "depletion price must be positive");
    for field in ["pool", "pool_name"] {
        ensure!(
            observation[field].as_str().is_some_and(|s| !s.is_empty()),
            "depletion observation pool binding missing"
        );
    }
    let state = &observation["state"];
    let pools = state["pools"]
        .as_array()
        .context("observation pools missing")?;
    let mut matches = pools
        .iter()
        .filter(|p| p["id"] == observation["pool"] && p["name"] == observation["pool_name"]);
    let pool = matches.next().context("observed rotation pool missing")?;
    ensure!(
        pool["generation"].as_u64().is_some(),
        "observed generation missing"
    );
    ensure!(matches.next().is_none(), "duplicate observed rotation pool");
    let wallets = pool["addresses"]
        .as_array()
        .context("observed wallets missing")?;
    let active = wallet(wallets, "ACTIVE")?;
    let ready = wallet(wallets, "READY")?;
    ensure!(
        active["id"] != ready["id"],
        "active and standby identities coincide"
    );
    let balances = &observation["balances"];
    ensure!(
        balances["block_height"].as_u64().is_some_and(|h| h > 0),
        "fresh balance block missing"
    );
    balances["block_hash"]
        .as_str()
        .context("fresh balance hash missing")?
        .parse::<B256>()?;
    let available = available_balance(balances, active)?;
    let standby = available_balance(balances, ready)?;
    ensure!(
        standby >= price,
        "funded standby cannot cover the reviewed depletion price"
    );
    let needed = available
        .checked_div(price)
        .context("invalid depletion price")?
        .checked_add(U256::from(1))
        .context("depletion call count overflow")?;
    ensure!(
        needed <= U256::from(planned_calls),
        "infeasible depletion schedule: requires an estimated {needed} calls including the promotion trigger, but only {planned_calls} reviewed calls exist; expand the manifest and reservations explicitly"
    );
    let fees = needed
        .checked_mul(price)
        .context("depletion fee estimate overflow")?;
    Ok(
        json!({"pool":observation["pool"],"pool_name":observation["pool_name"],
        "active":active["id"],"standby":ready["id"],"generation":pool["generation"],
        "active_available_atomic":available.to_string(),"standby_available_atomic":standby.to_string(),
        "expected_price_atomic":price.to_string(),"estimated_calls":needed.to::<u64>(),
        "estimated_depletion_fees_atomic":fees.to_string(),"planned_calls":planned_calls,
        "scope":"estimate at reviewed price, including promotion-triggering call; no payment authority or guarantee against price drift"}),
    )
}
fn number(s: &str) -> Result<U256> {
    Ok(s.parse()?)
}
fn wallet<'a>(wallets: &'a [Value], role: &str) -> Result<&'a Value> {
    let mut matching = wallets.iter().filter(|w| w["role"] == role);
    let found = matching
        .next()
        .context("rotation requires active and funded standby")?;
    ensure!(
        matching.next().is_none(),
        "duplicate wallet role in observation"
    );
    Ok(found)
}
fn available_balance(balances: &Value, wallet: &Value) -> Result<U256> {
    let id = wallet["id"]
        .as_str()
        .context("observed wallet ID missing")?;
    let confirmed = number(
        balances["confirmed"][id]
            .as_str()
            .context("wallet was not refreshed by observed RPC view")?,
    )?;
    let unresolved = balances["unresolved"]
        .as_object()
        .context("unresolved exposure missing")?
        .get(id)
        .map(|v| number(v.as_str().context("invalid unresolved exposure")?))
        .transpose()?
        .unwrap_or(U256::ZERO);
    let available = confirmed
        .checked_sub(unresolved)
        .context("unresolved exposure exceeds confirmed balance")?;
    ensure!(
        unresolved == U256::ZERO,
        "depletion baseline has unresolved authorizations; await canonical reconciliation before estimating rotation"
    );
    Ok(available)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> Value {
        json!({"pool":"p","pool_name":"pool","state":{"pools":[{"id":"p","name":"pool","generation":3,"addresses":[{"id":"a","role":"ACTIVE"},{"id":"b","role":"READY"}]}]},"balances":{"block_height":12,"block_hash":format!("0x{:064x}",1),"confirmed":{"a":"80","b":"200"},"unresolved":{}}})
    }
    #[test]
    fn estimate_includes_trigger_reservations_and_exact_division_boundary() {
        let value = observation();
        assert_eq!(
            depletion_estimate(&value, "20", 5).unwrap()["estimated_calls"],
            5
        );
        assert!(depletion_estimate(&value, "20", 4).is_err());
        assert_eq!(
            depletion_estimate(&value, "21", 4).unwrap()["estimated_calls"],
            4
        );
        let mut empty = value;
        empty["balances"]["confirmed"]["a"] = json!("0");
        assert_eq!(
            depletion_estimate(&empty, "20", 1).unwrap()["estimated_calls"],
            1
        );
    }
    #[test]
    fn missing_or_inconsistent_balances_cannot_estimate_depletion() {
        for (pointer, value) in [
            ("/balances/confirmed/a", Value::Null),
            ("/balances/confirmed/b", Value::Null),
            ("/balances/confirmed/a", json!("not-a-number")),
            ("/balances/confirmed/b", json!("1")),
            ("/balances/block_hash", json!("invalid")),
            ("/state/pools/0/addresses/1/role", json!("ALLOCATED")),
        ] {
            let mut o = observation();
            *o.pointer_mut(pointer).unwrap() = value;
            assert!(depletion_estimate(&o, "20", 10).is_err(), "{pointer}");
        }
        let mut pending = observation();
        pending["balances"]["unresolved"]["a"] = json!("20");
        assert!(
            depletion_estimate(&pending, "20", 10)
                .unwrap_err()
                .to_string()
                .contains("unresolved authorizations")
        );
        pending["balances"]["unresolved"]["a"] = json!("101");
        assert!(depletion_estimate(&pending, "20", 10).is_err());
        assert!(depletion_estimate(&observation(), "0", 10).is_err());
        let mut huge = observation();
        huge["balances"]["confirmed"]["a"] = json!(U256::MAX.to_string());
        huge["balances"]["unresolved"] = json!({});
        assert!(depletion_estimate(&huge, "1", usize::MAX).is_err());
    }
}
