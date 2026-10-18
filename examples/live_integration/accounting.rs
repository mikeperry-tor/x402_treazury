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
    let mut data = json!({"treasury_id":status["treasury_id"],"source_budget_entries":snapshot["source_budget_entries"],
        "operations":status["treasury_operations"],"wallets":snapshot["wallet_observations"],
        "refunds":status["refunds"],"payment_attempts":snapshot["payment_attempts"],
        "sync":if sync.is_null(){Value::Null}else{json!({"height":sync["height"],"checked_at":sync["checked_at"],"confirmed_shielded_zatoshis":sync["confirmed_shielded_zatoshis"],"spendable_shielded_zatoshis":sync["spendable_shielded_zatoshis"]})}});
    if snapshot.get("source_preparations").is_some() {
        data["schema_version"] = json!(2);
        data["source_preparations"] = snapshot["source_preparations"].clone();
        data["operations"] = snapshot["lifecycle"]["source_operations"].clone();
    }
    summarize(&data)?;
    Ok(data)
}
#[derive(Default)]
struct SourceTotals {
    reserved: u64,
    consumed: u64,
    principal: u64,
    fees: u64,
}

struct SourceAccounting<'a> {
    operations: BTreeMap<&'a str, &'a Value>,
    totals: SourceTotals,
}

// Indexing, consumed facts and confirmed-operation reconciliation are one stage.
fn source_accounting(data: &Value) -> Result<SourceAccounting<'_>> {
    let mut operations = BTreeMap::new();
    for op in rows(data, "operations")? {
        ensure!(
            operations.insert(id(&op["operation_id"])?, op).is_none(),
            "duplicate source operation"
        );
    }
    let mut totals = SourceTotals::default();
    if data["schema_version"] == 2 {
        let mut owners = BTreeSet::new();
        for entry in rows(data, "source_preparations")? {
            ensure!(owners.insert(id(&entry["operation_id"])?), "duplicate source preparation");
            let digest = id(&entry["preparation_sha256"])?;
            ensure!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "invalid preparation digest");
        }
        for (key, op) in &operations {
            ensure!(owners.contains(key), "prepared operation lacks original calculation");
            let principal = number(&op["facts"]["amount_zatoshis"])?;
            let fee = number(&op["facts"]["fee_zatoshis"])?;
            if op["submission"] == "CONFIRMED" {
                plus(&mut totals.principal, principal)?;
                plus(&mut totals.fees, fee)?;
                plus(&mut totals.consumed, principal.checked_add(fee).context("source facts overflow")?)?;
            }
        }
        return Ok(SourceAccounting { operations, totals });
    }

    let mut entries = BTreeMap::new();
    for entry in rows(data, "source_budget_entries")? {
        let key = id(&entry["id"])?;
        let cost = number(&entry["consumed"])?;
        ensure!(
            entries.insert(key, cost).is_none(),
            "duplicate source budget entry"
        );
        let held = number(&entry["reserved"])?;
        plus(&mut totals.reserved, held)?;
        plus(&mut totals.consumed, cost)?;
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
        plus(&mut totals.fees, fee)?;
        if cost == total {
            plus(&mut totals.principal, value)?;
        }
    }
    for (key, op) in &operations {
        ensure!(
            op["submission"] != "CONFIRMED" || entries.get(key).is_some_and(|cost| *cost > 0),
            "confirmed operation lacks consumed budget evidence"
        );
    }
    Ok(SourceAccounting { operations, totals })
}

#[derive(Default)]
struct RoleBalances {
    wallets: u64,
    anchored: u64,
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
        if data["schema_version"] == 2 {
            if let Ok(estimate) = amount(&wallet["balance"]) {
                plus(&mut bucket.anchored, 1)?;
                bucket.saved_balance = bucket.saved_balance.checked_add(estimate).context("USDC balance sum overflow")?;
            }
            continue;
        }
        let balance = amount(&wallet["balance"])?;
        match (wallet["height"].as_u64(), wallet["hash"].as_str()) {
            (Some(_), Some(hash)) if !hash.is_empty() => {
                plus(&mut bucket.anchored, 1)?;
                bucket.saved_balance = bucket
                    .saved_balance
                    .checked_add(balance)
                    .context("USDC balance sum overflow")?;
            }
            (None, None) if wallet["height"].is_null() && wallet["hash"].is_null() => {}
            _ => anyhow::bail!("incomplete wallet balance anchor"),
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
    Ok(exposure)
}

fn refund_outputs(data: &Value, operations: &BTreeMap<&str, &Value>) -> Result<u64> {
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
    Ok(returned)
}

pub fn summarize(data: &Value) -> Result<Value> {
    let source = source_accounting(data)?;
    let wallets = wallet_observations(data)?;
    let exposure = payment_exposure(data, &wallets.identities)?;
    let current = data["schema_version"] == 2;
    let returned = if current { None } else { Some(refund_outputs(data, &source.operations)?) };
    let sync = &data["sync"];
    if !sync.is_null() {
        number(&sync["confirmed_shielded_zatoshis"])?;
        number(&sync["spendable_shielded_zatoshis"])?;
    }
    Ok(
        json!({"scope":"treasury_wide_persisted","fresh_chain_read":false,
        "source_reserved_zatoshis":if current {None}else{Some(source.totals.reserved)},"recorded_consumed_zatoshis":source.totals.consumed,
        "recorded_principal_zatoshis":source.totals.principal,"recorded_fees_zatoshis":source.totals.fees,"recorded_refund_outputs_zatoshis":returned,
        "unresolved_payment_exposure_atomic":exposure.to_string(),"zec_observation":sync,
        "wallet_roles":wallets.roles.into_iter().map(|(role,balance)|(role,if current {json!({"wallets":balance.wallets,"observation_model":"advisory_estimate","available_estimates":balance.anchored,"unobserved_wallets":balance.wallets-balance.anchored,"saved_balance_atomic":balance.saved_balance.to_string()})} else {json!({"wallets":balance.wallets,"anchored_observations":balance.anchored,"unobserved_wallets":balance.wallets-balance.anchored,"saved_balance_atomic":balance.saved_balance.to_string()})})).collect::<BTreeMap<_,_>>() }),
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
    fn sdk_fact_model_keeps_deleted_projections_unavailable_and_shield_fee_only() {
        let mut data=fixture();
        data["schema_version"]=json!(2);
        data["source_budget_entries"]=Value::Null;
        data["refunds"]=Value::Null;
        data["operations"][1]["facts"]["amount_zatoshis"]=json!(0);
        data["source_preparations"]=json!([{"operation_id":"deposit","preparation_sha256":"a".repeat(64)},{"operation_id":"shield","preparation_sha256":"b".repeat(64)},{"operation_id":"calculation-only","preparation_sha256":"c".repeat(64)}]);
        let result=summarize(&data).unwrap();
        assert_eq!(result["recorded_consumed_zatoshis"],108);
        assert_eq!(result["recorded_principal_zatoshis"],100);
        assert_eq!(result["recorded_fees_zatoshis"],8);
        assert!(result["source_reserved_zatoshis"].is_null());
        assert!(result["recorded_refund_outputs_zatoshis"].is_null());
        data["operations"][0]["submission"]=json!("UNKNOWN");
        assert_eq!(summarize(&data).unwrap()["recorded_consumed_zatoshis"],3);
    }
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
    fn retained_saved_balances_remain_separate_from_retired_inventory() {
        let mut data = fixture();
        data["wallets"].as_array_mut().unwrap().push(
            json!({"id":"retained","role":"RETAINED","balance":"9","height":101,"hash":"observed"}),
        );
        let report = summarize(&data).unwrap();
        assert_eq!(
            report["wallet_roles"]["RETAINED"]["saved_balance_atomic"],
            "9"
        );
        assert_eq!(
            report["wallet_roles"]["RETIRED"]["saved_balance_atomic"],
            "7"
        );
        assert_eq!(report["fresh_chain_read"], false);
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

#[cfg(test)]
mod characterization {
    use super::*;

    fn empty() -> Value {
        json!({"operations":[],"source_budget_entries":[],"wallets":[],"payment_attempts":[],"refunds":[]})
    }

    fn empty_summary() -> Value {
        json!({"scope":"treasury_wide_persisted","fresh_chain_read":false,
            "source_reserved_zatoshis":0,"recorded_consumed_zatoshis":0,
            "recorded_principal_zatoshis":0,"recorded_fees_zatoshis":0,
            "recorded_refund_outputs_zatoshis":0,"unresolved_payment_exposure_atomic":"0",
            "zec_observation":null,"wallet_roles":{}})
    }

    fn error(data: &Value, expected: &str) {
        assert_eq!(format!("{:#}", summarize(data).unwrap_err()), expected);
    }

    #[test]
    fn complete_mixed_summary_preserves_distinct_facts() {
        assert_eq!(
            summarize(&fixture()).unwrap(),
            json!({
            "scope":"treasury_wide_persisted","fresh_chain_read":false,
            "source_reserved_zatoshis":70,"recorded_consumed_zatoshis":108,
            "recorded_principal_zatoshis":100,"recorded_fees_zatoshis":8,
            "recorded_refund_outputs_zatoshis":10,"unresolved_payment_exposure_atomic":"5",
            "zec_observation":{"height":200,"checked_at":10,"confirmed_shielded_zatoshis":60,"spendable_shielded_zatoshis":50},
            "wallet_roles":{
                "ACTIVE":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":"0"},
                "ALLOCATED":{"wallets":1,"anchored_observations":0,"unobserved_wallets":1,"saved_balance_atomic":"0"},
                "RETIRED":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":"7"}}})
        );
    }

    #[test]
    fn empty_and_permissive_historical_evidence_keep_full_projection() {
        let mut data = empty();
        assert_eq!(summarize(&data).unwrap(), empty_summary());
        data["sync"] = Value::Null;
        assert_eq!(summarize(&data).unwrap(), empty_summary());
        data["operations"] = json!([{"operation_id":"opaque","facts":false}]);
        data["source_budget_entries"] = json!([{"id":"no-operation","reserved":7,"consumed":0}]);
        data["refunds"] = json!([{"txid":"opaque","output_index":0,"height":0,"operation_id":"opaque","amount":11}]);
        data["wallets"] = json!([
            {"id":"a","role":"ACTIVE","balance":"0000","height":0,"hash":"not-a-block-hash"},
            {"id":"b","role":"ALLOCATED","balance":"999"},
            {"id":"c","role":"READY","balance":"0005","height":0,"hash":" "},
            {"id":"d","role":"RETAINED","balance":"9","height":1,"hash":"opaque"},
            {"id":"e","role":"RETIRED","balance":"7","height":1,"hash":"opaque"},
            {"id":"f","role":"READY","balance":"80","height":null,"hash":null}]);
        data["sync"] = json!({"height":"opaque","checked_at":null,"confirmed_shielded_zatoshis":1,"spendable_shielded_zatoshis":9,"extra":{"keep":true}});
        let mut expected = empty_summary();
        expected["source_reserved_zatoshis"] = json!(7);
        expected["recorded_refund_outputs_zatoshis"] = json!(11);
        expected["zec_observation"] = json!({"height":"opaque","checked_at":null,"confirmed_shielded_zatoshis":1,"spendable_shielded_zatoshis":9,"extra":{"keep":true}});
        expected["wallet_roles"] = json!({
            "ACTIVE":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":"0"},
            "ALLOCATED":{"wallets":1,"anchored_observations":0,"unobserved_wallets":1,"saved_balance_atomic":"0"},
            "READY":{"wallets":2,"anchored_observations":1,"unobserved_wallets":1,"saved_balance_atomic":"5"},
            "RETAINED":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":"9"},
            "RETIRED":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":"7"}});
        assert_eq!(summarize(&data).unwrap(), expected);
    }

    #[test]
    fn malformed_fields_preserve_exact_diagnostics() {
        for (path, value, expected) in [
            (
                "/operations/0/operation_id",
                json!(""),
                "missing accounting identity",
            ),
            (
                "/operations/1/operation_id",
                json!("deposit"),
                "duplicate source operation",
            ),
            (
                "/source_budget_entries/1/id",
                json!("deposit"),
                "duplicate source budget entry",
            ),
            (
                "/source_budget_entries/0/reserved",
                json!(1),
                "consumed source cost remains reserved",
            ),
            (
                "/source_budget_entries/0/id",
                json!("missing"),
                "consumed source entry lacks operation",
            ),
            (
                "/operations/0/submission",
                json!("BROADCAST"),
                "consumed source operation is not recorded confirmed",
            ),
            (
                "/source_budget_entries/0/consumed",
                json!(104),
                "source cost disagrees with transaction facts",
            ),
            (
                "/source_budget_entries/0/consumed",
                json!(0),
                "confirmed operation lacks consumed budget evidence",
            ),
            (
                "/source_budget_entries/0/reserved",
                json!("0"),
                "invalid accounting amount",
            ),
            (
                "/operations/0/facts/fee_zatoshis",
                json!(-1),
                "invalid accounting amount",
            ),
            (
                "/operations/0/facts/amount_zatoshis",
                json!(1.5),
                "invalid accounting amount",
            ),
            (
                "/wallets/1/id",
                json!("private-active"),
                "duplicate wallet observation",
            ),
            ("/wallets/0/role", json!("OTHER"), "unknown wallet role"),
            ("/wallets/0/balance", json!(0), "missing atomic balance"),
            (
                "/wallets/1/hash",
                json!("unexpected"),
                "incomplete wallet balance anchor",
            ),
            (
                "/wallets/0/hash",
                json!(""),
                "incomplete wallet balance anchor",
            ),
            (
                "/wallets/0/height",
                json!("100"),
                "incomplete wallet balance anchor",
            ),
            (
                "/payment_attempts/1/id",
                json!("a"),
                "duplicate payment attempt",
            ),
            (
                "/payment_attempts/0/state",
                json!("UNKNOWN"),
                "unknown payment state",
            ),
            (
                "/payment_attempts/0/wallet",
                json!("unknown"),
                "payment references missing wallet",
            ),
            (
                "/payment_attempts/2/amount",
                json!("invalid"),
                "invalid atomic balance",
            ),
            (
                "/refunds/0/operation_id",
                json!("unknown"),
                "refund references missing operation",
            ),
            (
                "/refunds/0/height",
                Value::Null,
                "invalid accounting amount",
            ),
            (
                "/refunds/0/output_index",
                json!(-1),
                "invalid accounting amount",
            ),
            (
                "/sync/confirmed_shielded_zatoshis",
                json!(false),
                "invalid accounting amount",
            ),
            (
                "/sync/spendable_shielded_zatoshis",
                Value::Null,
                "invalid accounting amount",
            ),
        ] {
            let mut data = fixture();
            *data.pointer_mut(path).unwrap() = value;
            error(&data, expected);
        }
        for field in [
            "operations",
            "source_budget_entries",
            "wallets",
            "payment_attempts",
            "refunds",
        ] {
            for malformed in [Value::Null, json!({}), json!(0)] {
                let mut data = fixture();
                data[field] = malformed;
                error(&data, "missing accounting rows");
            }
        }
        for invalid in ["", "-1", "+1", " 1", "1 ", "0x1", "1.0", "١"] {
            let mut data = fixture();
            data["wallets"][1]["balance"] = json!(invalid);
            error(&data, "invalid atomic balance");
        }
        let mut data = fixture();
        let refund = data["refunds"][0].clone();
        data["refunds"].as_array_mut().unwrap().push(refund);
        error(&data, "duplicate refund outpoint");
    }

    #[test]
    fn checked_u64_boundaries_and_overflows() {
        let mut data = empty();
        data["source_budget_entries"] = json!([{"id":"a","consumed":0,"reserved":u64::MAX}]);
        let mut expected = empty_summary();
        expected["source_reserved_zatoshis"] = json!(u64::MAX);
        assert_eq!(summarize(&data).unwrap(), expected);
        data["source_budget_entries"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","consumed":0,"reserved":1}));
        error(&data, "accounting sum overflow");

        data = empty();
        data["operations"] = json!([{"operation_id":"a","submission":"CONFIRMED","facts":{"amount_zatoshis":u64::MAX-1,"fee_zatoshis":1}}]);
        data["source_budget_entries"] = json!([{"id":"a","consumed":u64::MAX,"reserved":0}]);
        expected = empty_summary();
        expected["recorded_consumed_zatoshis"] = json!(u64::MAX);
        expected["recorded_principal_zatoshis"] = json!(u64::MAX - 1);
        expected["recorded_fees_zatoshis"] = json!(1);
        assert_eq!(summarize(&data).unwrap(), expected);
        data["operations"][0]["facts"]["amount_zatoshis"] = json!(u64::MAX);
        error(&data, "source facts overflow");
        data["operations"][0]["facts"]["amount_zatoshis"] = json!(u64::MAX - 1);
        data["source_budget_entries"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","consumed":1,"reserved":0}));
        error(&data, "accounting sum overflow");

        data = empty();
        data["operations"] = json!([{"operation_id":"a"}]);
        data["refunds"] =
            json!([{"txid":"a","output_index":0,"height":0,"operation_id":"a","amount":u64::MAX}]);
        expected = empty_summary();
        expected["recorded_refund_outputs_zatoshis"] = json!(u64::MAX);
        assert_eq!(summarize(&data).unwrap(), expected);
        data["refunds"]
            .as_array_mut()
            .unwrap()
            .push(json!({"txid":"a","output_index":1,"height":0,"operation_id":"a","amount":1}));
        error(&data, "accounting sum overflow");
    }

    #[test]
    fn checked_atomic_boundaries_and_overflows() {
        const MAX: &str =
            "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        let mut data = empty();
        data["wallets"] =
            json!([{"id":"a","role":"ACTIVE","balance":MAX,"height":0,"hash":"opaque"}]);
        data["payment_attempts"] = json!([{"id":"a","wallet":"a","state":"ADMITTED","amount":MAX},
            {"id":"b","wallet":"a","state":"RESOLVED","amount":MAX}]);
        let mut expected = empty_summary();
        expected["wallet_roles"] = json!({"ACTIVE":{"wallets":1,"anchored_observations":1,"unobserved_wallets":0,"saved_balance_atomic":MAX}});
        expected["unresolved_payment_exposure_atomic"] = json!(MAX);
        assert_eq!(summarize(&data).unwrap(), expected);
        data["payment_attempts"][1]["state"] = json!("POSSIBLY_SUBMITTED");
        data["payment_attempts"][1]["amount"] = json!("1");
        error(&data, "payment exposure overflow");
        data["wallets"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"b","role":"ACTIVE","balance":"1","height":0,"hash":"opaque"}));
        error(&data, "USDC balance sum overflow");
        data["wallets"][0]["balance"] =
            json!("115792089237316195423570985008687907853269984665640564039457584007913129639936");
        // The pinned U256 parser supplies this diagnostic; it is part of the error chain.
        error(
            &data,
            "the value is too large to fit the target type: the value is too large to fit the target type",
        );
    }

    #[test]
    fn first_error_follows_stage_and_row_order() {
        let mut data = fixture();
        data["operations"][1]["operation_id"] = json!("deposit");
        data["source_budget_entries"][0]["reserved"] = json!(1);
        data["wallets"][0]["role"] = json!("OTHER");
        data["payment_attempts"][0]["state"] = json!("OTHER");
        data["refunds"][0]["operation_id"] = json!("missing");
        data["sync"]["confirmed_shielded_zatoshis"] = Value::Null;
        error(&data, "duplicate source operation");
        data["operations"][1]["operation_id"] = json!("shield");
        error(&data, "consumed source cost remains reserved");
        data["source_budget_entries"][0]["reserved"] = json!(0);
        data["source_budget_entries"][0]["consumed"] = json!(0);
        error(&data, "confirmed operation lacks consumed budget evidence");
        data["source_budget_entries"][0]["consumed"] = json!(105);
        error(&data, "unknown wallet role");
        data["wallets"][0]["role"] = json!("ACTIVE");
        error(&data, "unknown payment state");
        data["payment_attempts"][0]["state"] = json!("ADMITTED");
        error(&data, "refund references missing operation");
        data["refunds"][0]["operation_id"] = json!("deposit");
        error(&data, "invalid accounting amount");

        for (early_path, early_value, later_path, later_value, expected) in [
            (
                "/source_budget_entries/1/consumed",
                json!("bad"),
                "/source_budget_entries/1/id",
                json!("deposit"),
                "invalid accounting amount",
            ),
            (
                "/source_budget_entries/1/id",
                json!("deposit"),
                "/source_budget_entries/1/reserved",
                json!("bad"),
                "duplicate source budget entry",
            ),
            (
                "/wallets/1/id",
                json!("private-active"),
                "/wallets/1/role",
                json!("OTHER"),
                "duplicate wallet observation",
            ),
            (
                "/wallets/0/balance",
                json!("bad"),
                "/wallets/0/hash",
                json!(""),
                "invalid atomic balance",
            ),
            (
                "/payment_attempts/1/id",
                json!("a"),
                "/payment_attempts/1/state",
                json!("OTHER"),
                "duplicate payment attempt",
            ),
            (
                "/payment_attempts/0/state",
                json!("OTHER"),
                "/payment_attempts/0/wallet",
                json!("missing"),
                "unknown payment state",
            ),
            (
                "/payment_attempts/0/wallet",
                json!("missing"),
                "/payment_attempts/0/amount",
                json!("bad"),
                "payment references missing wallet",
            ),
            (
                "/refunds/0/output_index",
                json!("bad"),
                "/refunds/0/operation_id",
                json!("missing"),
                "invalid accounting amount",
            ),
            (
                "/wallets/0/role",
                json!("OTHER"),
                "/wallets/1/id",
                Value::Null,
                "unknown wallet role",
            ),
        ] {
            let mut data = fixture();
            *data.pointer_mut(early_path).unwrap() = early_value;
            *data.pointer_mut(later_path).unwrap() = later_value;
            error(&data, expected);
        }
    }
}
