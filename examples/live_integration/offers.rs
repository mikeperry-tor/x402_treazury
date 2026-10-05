//! Observed challenge offers, separately from admission and canonical debits.
use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn report(report: &Value) -> Result<BTreeMap<String, Vec<Value>>> {
    let mut claims = BTreeMap::new();
    let mut closed = BTreeSet::new();
    let mut observations = BTreeMap::<String, Vec<Value>>::new();
    for event in report["runtime_events"]
        .as_array()
        .context("offer events missing")?
    {
        let detail = &event["detail"];
        let kind = event["kind"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "application_claim"
                | "application_challenge"
                | "application_payment"
                | "application_finished"
        ) {
            continue;
        }
        let id = detail["case"]
            .as_str()
            .context("offer correlation case missing")?;
        match kind {
            "application_claim" => {
                ensure!(
                    claims.insert(id, detail).is_none(),
                    "duplicate offer case claim"
                );
            }
            "application_payment" | "application_finished" => {
                closed.insert(id);
            }
            _ => {
                let claim = claims.get(id).context("offer observation precedes claim")?;
                ensure!(
                    !closed.contains(id) && detail["session"] == claim["session"],
                    "offer observation is outside its claimed session/interval"
                );
                let start = claim["started_micros"]
                    .as_u64()
                    .context("offer claim time missing")?;
                let at = detail["observed_micros"]
                    .as_u64()
                    .context("offer observation time missing")?;
                ensure!(at >= start, "offer observation precedes application start");
                let rows = observations.entry(id.to_owned()).or_default();
                ensure!(
                    rows.len() < 2 && detail["ordinal"].as_u64() == Some(rows.len() as u64),
                    "duplicate or excessive challenge observations (limit two per case)"
                );
                if let Some(previous) = rows.last() {
                    ensure!(
                        previous["observed_micros"]
                            .as_u64()
                            .is_some_and(|t| t <= at),
                        "offer observation time regressed"
                    );
                }
                let mut row = projection(&detail["observation"])?;
                row["observed_micros"] = json!(at);
                rows.push(row);
            }
        }
    }
    Ok(observations)
}

fn projection(observation: &Value) -> Result<Value> {
    let status = observation["status"]
        .as_str()
        .context("offer observation status missing")?;
    ensure!(
        matches!(
            status,
            "observed"
                | "header_absent"
                | "duplicate_headers"
                | "header_limit_exceeded"
                | "malformed_header"
                | "malformed_offers"
                | "offer_limit_exceeded"
        ),
        "unknown offer observation status"
    );
    let offers = observation["offers"]
        .as_array()
        .context("offer list missing")?;
    ensure!(
        offers.len() <= 128 && (status == "observed" || offers.is_empty()),
        "invalid or excessive observed offer list (limit 128)"
    );
    let mut rows = Vec::new();
    for (index, offer) in offers.iter().enumerate() {
        ensure!(
            offer["index"].as_u64() == Some(index as u64),
            "offer index mismatch"
        );
        let base = offer["base_usdc"]
            .as_bool()
            .context("offer asset classification missing")?;
        let scheme = offer["scheme"].as_str().context("offer scheme missing")?;
        ensure!(
            matches!(scheme, "exact" | "upto" | "other_or_missing"),
            "invalid offer scheme classification"
        );
        let amount = if offer["amount_atomic"].is_null() {
            None
        } else {
            let raw = offer["amount_atomic"]
                .as_str()
                .context("offer amount must be decimal string")?;
            ensure!(
                !raw.is_empty() && raw.len() <= 78 && raw.bytes().all(|b| b.is_ascii_digit()),
                "invalid offer amount"
            );
            Some(U256::from_str_radix(raw, 10)?)
        };
        let above = if base {
            amount.map(|n| n > U256::from(10000))
        } else {
            None
        };
        ensure!(
            offer["above_one_cent"] == json!(above),
            "offer price threshold mismatch"
        );
        rows.push(json!({"index":index,"base_usdc":base,"scheme":scheme,"amount_atomic":amount.map(|n|n.to_string()),"above_one_cent":above}));
    }
    Ok(json!({"status":status,"offers":rows}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"runtime_events":[{"kind":"application_claim","detail":{"case":"a","session":"s","started_micros":1}},
            {"kind":"application_challenge","detail":{"case":"a","session":"s","ordinal":0,"observed_micros":2,
                "observation":{"status":"observed","offers":[{"index":0,"base_usdc":true,"scheme":"exact","amount_atomic":"10001","above_one_cent":true}]}}}]})
    }
    #[test]
    fn rejected_offer_is_visible_without_an_admission() {
        let result = report(&input()).unwrap();
        assert_eq!(result["a"][0]["offers"][0]["amount_atomic"], "10001");
    }
    #[test]
    fn cross_session_replay_order_and_threshold_tampering_are_refused() {
        for (field, value) in [
            ("session", json!("other")),
            ("ordinal", json!(1)),
            ("observed_micros", json!(0)),
        ] {
            let mut r = input();
            r["runtime_events"][1]["detail"][field] = value;
            assert!(report(&r).is_err());
        }
        let mut r = input();
        r["runtime_events"][1]["detail"]["observation"]["offers"][0]["above_one_cent"] =
            json!(false);
        assert!(report(&r).is_err());
        let mut r = input();
        let e = r["runtime_events"][1].clone();
        r["runtime_events"].as_array_mut().unwrap().push(e);
        assert!(report(&r).is_err());
        let mut r = input();
        r["runtime_events"].as_array_mut().unwrap().insert(
            1,
            json!({"kind":"application_payment","detail":{"case":"a"}}),
        );
        assert!(report(&r).is_err());
    }
}
