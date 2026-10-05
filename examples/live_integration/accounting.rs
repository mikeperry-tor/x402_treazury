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
    let status = &snapshot["treasury_status"];
    let sync = &status["sync"];
    let data = json!({"treasury_id":status["treasury_id"],"source_budget_entries":snapshot["source_budget_entries"],
        "operations":status["treasury_operations"],"wallets":snapshot["wallet_observations"],
        "refunds":status["refunds"],"payment_attempts":snapshot["payment_attempts"],
        "sync":if sync.is_null(){Value::Null}else{json!({"height":sync["height"],"checked_at":sync["checked_at"],"confirmed_shielded_zatoshis":sync["confirmed_shielded_zatoshis"],"spendable_shielded_zatoshis":sync["spendable_shielded_zatoshis"]})}});
    summarize(&data)?;
    Ok(data)
}
pub fn summarize(data: &Value) -> Result<Value> {
    let mut operations = BTreeMap::new();
    for op in rows(data, "operations")? {
        ensure!(
            operations.insert(id(&op["operation_id"])?, op).is_none(),
            "duplicate source operation"
        );
    }
    let (mut reserved, mut consumed, mut principal, mut fees) = (0, 0, 0, 0);
    let mut entries = BTreeMap::new();
    for entry in rows(data, "source_budget_entries")? {
        let key = id(&entry["id"])?;
        let cost = number(&entry["consumed"])?;
        ensure!(
            entries.insert(key, cost).is_none(),
            "duplicate source budget entry"
        );
        let held = number(&entry["reserved"])?;
        plus(&mut reserved, held)?;
        plus(&mut consumed, cost)?;
        if cost == 0 {
            continue;
        }
        ensure!(held == 0, "consumed source cost remains reserved");
        let op = operations
            .get(key)
            .context("consumed source entry lacks operation")?;
        ensure!(
            op["submission"] == "CONFIRMED",
            "consumed source operation is not recorded confirmed"
        );
        let fee = number(&op["facts"]["fee_zatoshis"])?;
        let value = number(&op["facts"]["amount_zatoshis"])?;
        let total = value.checked_add(fee).context("source facts overflow")?;
        ensure!(
            cost == total || cost == fee,
            "source cost disagrees with transaction facts"
        );
        plus(&mut fees, fee)?;
        if cost == total {
            plus(&mut principal, value)?;
        }
    }
    for (key, op) in &operations {
        ensure!(
            op["submission"] != "CONFIRMED" || entries.get(key).is_some_and(|cost| *cost > 0),
            "confirmed operation lacks consumed budget evidence"
        );
    }
    let mut wallets = BTreeSet::new();
    let mut roles = BTreeMap::<String, (u64, u64, U256)>::new();
    for wallet in rows(data, "wallets")? {
        ensure!(
            wallets.insert(id(&wallet["id"])?),
            "duplicate wallet observation"
        );
        let role = id(&wallet["role"])?;
        ensure!(
            matches!(role, "ALLOCATED" | "ACTIVE" | "READY" | "RETIRED"),
            "unknown wallet role"
        );
        let bucket = roles.entry(role.to_owned()).or_default();
        plus(&mut bucket.0, 1)?;
        let balance = amount(&wallet["balance"])?;
        match (wallet["height"].as_u64(), wallet["hash"].as_str()) {
            (Some(_), Some(hash)) if !hash.is_empty() => {
                plus(&mut bucket.1, 1)?;
                bucket.2 = bucket
                    .2
                    .checked_add(balance)
                    .context("USDC balance sum overflow")?;
            }
            (None, None) if wallet["height"].is_null() && wallet["hash"].is_null() => {}
            _ => anyhow::bail!("incomplete wallet balance anchor"),
        }
    }
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
                Some("ADMITTED" | "POSSIBLY_SUBMITTED" | "RESOLVED")
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
    let mut refunds = BTreeSet::new();
    let mut returned = 0;
    for refund in rows(data, "refunds")? {
        ensure!(
            refunds.insert((id(&refund["txid"])?, number(&refund["output_index"])?)),
            "duplicate refund outpoint"
        );
        ensure!(
            operations.contains_key(id(&refund["operation_id"])?),
            "refund references missing operation"
        );
        number(&refund["height"])?;
        plus(&mut returned, number(&refund["amount"])?)?;
    }
    let sync = &data["sync"];
    if !sync.is_null() {
        number(&sync["confirmed_shielded_zatoshis"])?;
        number(&sync["spendable_shielded_zatoshis"])?;
    }
    Ok(
        json!({"scope":"treasury_wide_persisted","fresh_chain_read":false,
        "source_reserved_zatoshis":reserved,"recorded_consumed_zatoshis":consumed,
        "recorded_principal_zatoshis":principal,"recorded_fees_zatoshis":fees,"recorded_refund_outputs_zatoshis":returned,
        "unresolved_payment_exposure_atomic":exposure.to_string(),"zec_observation":sync,
        "wallet_roles":roles.into_iter().map(|(role,(count,observed,sum))|(role,json!({"wallets":count,"anchored_observations":observed,"unobserved_wallets":count-observed,"saved_balance_atomic":sum.to_string()}))).collect::<BTreeMap<_,_>>() }),
    )
}

#[cfg(test)]
pub(crate) fn fixture() -> Value {
    json!({"treasury_id":"private-treasury","source_budget_entries":[
        {"id":"deposit","reserved":0,"consumed":105},{"id":"shield","reserved":0,"consumed":3},{"id":"queued","reserved":70,"consumed":0}],
        "operations":[{"operation_id":"deposit","submission":"CONFIRMED","facts":{"amount_zatoshis":100,"fee_zatoshis":5}},
        {"operation_id":"shield","submission":"CONFIRMED","facts":{"amount_zatoshis":80,"fee_zatoshis":3}}],
        "wallets":[{"id":"private-active","role":"ACTIVE","balance":"0","height":100,"hash":"private-hash"},
        {"id":"private-allocated","role":"ALLOCATED","balance":"0","height":null,"hash":null},
        {"id":"private-retired","role":"RETIRED","balance":"7","height":90,"hash":"private-hash"}],
        "payment_attempts":[{"id":"a","wallet":"private-active","state":"ADMITTED","amount":"2"},{"id":"b","wallet":"private-retired","state":"POSSIBLY_SUBMITTED","amount":"3"},{"id":"c","wallet":"private-active","state":"RESOLVED","amount":"100"}],
        "refunds":[{"txid":"private-refund","output_index":0,"amount":10,"operation_id":"deposit","height":100}],
        "sync":{"height":200,"checked_at":10,"confirmed_shielded_zatoshis":60,"spendable_shielded_zatoshis":50}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn costs_refunds_exposure_and_observed_zero_are_not_conflated() {
        let result = summarize(&fixture()).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"], 108);
        assert_eq!(result["recorded_principal_zatoshis"], 100);
        assert_eq!(result["recorded_fees_zatoshis"], 8);
        assert_eq!(result["source_reserved_zatoshis"], 70);
        assert_eq!(result["recorded_refund_outputs_zatoshis"], 10);
        assert_eq!(result["unresolved_payment_exposure_atomic"], "5");
        assert_eq!(result["wallet_roles"]["ACTIVE"]["anchored_observations"], 1);
        assert_eq!(result["wallet_roles"]["ALLOCATED"]["unobserved_wallets"], 1);
        assert_eq!(
            result["wallet_roles"]["RETIRED"]["saved_balance_atomic"],
            "7"
        );
        assert_eq!(result["fresh_chain_read"], false);
    }
    #[test]
    fn contradictory_or_overflowing_bookkeeping_is_rejected() {
        for (path, value) in [
            ("/source_budget_entries/0/consumed", json!(104)),
            ("/source_budget_entries/0/consumed", json!(0)),
            ("/source_budget_entries/0/reserved", json!(1)),
            ("/operations/0/submission", json!("BROADCAST")),
            ("/operations/1/operation_id", json!("deposit")),
            ("/wallets/1/id", json!("private-active")),
            ("/wallets/1/hash", json!("unexpected")),
            ("/wallets/0/balance", json!("-1")),
            ("/payment_attempts/1/id", json!("a")),
            ("/payment_attempts/0/amount", json!("9".repeat(100))),
            ("/refunds/0/amount", json!(-1)),
            ("/refunds/0/operation_id", json!("unknown")),
            ("/payment_attempts/0/wallet", json!("unknown")),
        ] {
            let mut data = fixture();
            *data.pointer_mut(path).unwrap() = value;
            assert!(summarize(&data).is_err(), "{path}");
        }
    }
}
