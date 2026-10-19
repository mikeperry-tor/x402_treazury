//! Bounded seller claims, never canonical chain evidence. No payload/signature is retained.
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const HEADER_LIMIT: usize = 16 * 1024;

pub(super) fn evidence(result: &anyhow::Result<reqwest::Response>) -> Value {
    let Ok(response) = result else {
        return json!({"classification":"transport_unknown"});
    };
    let values: Vec<_> = ["payment-response", "x-payment-response"]
        .into_iter()
        .flat_map(|name| {
            response
                .headers()
                .get_all(name)
                .iter()
                .map(move |header| (name, header))
        })
        .collect();
    let mut receipt = match values.as_slice() {
        [] => json!({"classification":"missing"}),
        [(name, header)] => parse_network(
            header.as_bytes(),
            if *name == "x-payment-response" {
                "base"
            } else {
                "eip155:8453"
            },
        ),
        _ => json!({"classification":"duplicate"}),
    };
    receipt["http_status"] = json!(response.status().as_u16());
    receipt
}
fn parse_network(raw: &[u8], network: &str) -> Value {
    if raw.len() > HEADER_LIMIT {
        tracing::warn!(
            code = "qualification_receipt_limit",
            limit_bytes = HEADER_LIMIT,
            actual_bytes = raw.len(),
            "seller receipt exceeds evidence limit; classified oversized, not truncated"
        );
        return json!({"classification":"oversized","limit_bytes":HEADER_LIMIT,"actual_bytes":raw.len()});
    }
    let digest = format!("{:x}", Sha256::digest(raw));
    let mut result = json!({"classification":"malformed","header_sha256":digest});
    let Some(value) = BASE64_STANDARD
        .decode(raw)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return result;
    };
    let Some(success) = value["success"].as_bool() else {
        return result;
    };
    result["seller_success"] = json!(success);
    if !success {
        result["classification"] = json!("seller_failure");
        // Vendor error prose can echo secrets. Record its presence, never its text.
        result["error_reason_present"] = json!(value.get("errorReason").is_some());
        return result;
    }
    if value["network"] != network {
        return result;
    }
    let Some(transaction) = value["transaction"]
        .as_str()
        .and_then(|s| s.parse::<alloy_primitives::B256>().ok())
        .filter(|h| !h.is_zero())
    else {
        return result;
    };
    if let Some(payer) = value.get("payer") {
        let Some(payer) = payer
            .as_str()
            .and_then(|s| s.parse::<alloy_primitives::Address>().ok())
        else {
            return result;
        };
        result["payer"] = json!(payer.to_string());
    }
    result["classification"] = json!("seller_success");
    result["network"] = json!("eip155:8453");
    result["transaction"] = json!(transaction.to_string());
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(raw: &[u8]) -> Value {
        parse_network(raw, "eip155:8453")
    }
    #[test]
    fn receipts_are_bounded_projected_and_cannot_claim_chain_verification() {
        let encode = |v: Value| BASE64_STANDARD.encode(v.to_string());
        let valid = json!({"success":true,"network":"eip155:8453","transaction":format!("0x{:064x}",1),
            "payer":format!("0x{:040x}",2),"secret_extra":"do not retain"});
        let parsed = parse(encode(valid.clone()).as_bytes());
        assert_eq!(parsed["classification"], "seller_success");
        assert!(parsed.get("secret_extra").is_none());
        assert!(parsed.get("verified").is_none());
        for patch in [
            json!({"network":"eip155:1"}),
            json!({"transaction":""}),
            json!({"transaction":format!("0x{:064x}",0)}),
            json!({"payer":"bad"}),
            json!({"success":"true"}),
        ] {
            let mut v = valid.clone();
            v.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert_eq!(parse(encode(v).as_bytes())["classification"], "malformed");
        }
        assert_eq!(parse(b"not base64")["classification"], "malformed");
        let failure =
            parse(encode(json!({"success":false,"errorReason":"sensitive reason"})).as_bytes());
        assert_eq!(failure["classification"], "seller_failure");
        assert!(!failure.to_string().contains("sensitive reason"));
        let oversized = parse(&vec![b'a'; HEADER_LIMIT + 1]);
        assert_eq!(oversized["classification"], "oversized");
        assert_eq!(oversized["actual_bytes"], HEADER_LIMIT + 1);
    }
    #[test]
    fn absent_duplicate_and_transport_uncertainty_remain_distinct() {
        let response = |headers: &[&str]| {
            let mut r = axum::http::Response::builder().status(200);
            for value in headers {
                r = r.header("payment-response", *value);
            }
            Ok(reqwest::Response::from(r.body("").unwrap()))
        };
        assert_eq!(evidence(&response(&[]))["classification"], "missing");
        assert_eq!(
            evidence(&response(&["a", "b"]))["classification"],
            "duplicate"
        );
        assert_eq!(
            evidence(&Err(anyhow::anyhow!("sensitive transport error")))["classification"],
            "transport_unknown"
        );
    }

    #[test]
    fn legacy_receipts_normalize_for_recovery_and_both_names_share_ambiguity() {
        let claim = json!({"success":true,"network":"base","transaction":format!("0x{:064x}",1),"payer":format!("0x{:040x}",2)});
        let encoded = BASE64_STANDARD.encode(claim.to_string());
        let response = |headers: &[(&str, &str)]| {
            let mut r = axum::http::Response::builder().status(200);
            for (name, value) in headers {
                r = r.header(*name, *value);
            }
            Ok(reqwest::Response::from(r.body("").unwrap()))
        };
        let legacy = evidence(&response(&[("x-payment-response", &encoded)]));
        assert_eq!(legacy["classification"], "seller_success");
        assert_eq!(legacy["network"], "eip155:8453");
        assert_eq!(legacy["transaction"], claim["transaction"]);
        assert_eq!(legacy["payer"], claim["payer"]);
        assert_eq!(
            evidence(&response(&[("payment-response", &encoded)]))["classification"],
            "malformed"
        );
        assert_eq!(
            evidence(&response(&[("x-payment-response", "bad")]))["classification"],
            "malformed"
        );
        for names in [
            ["x-payment-response", "x-payment-response"],
            ["x-payment-response", "payment-response"],
        ] {
            assert_eq!(
                evidence(&response(&[(names[0], &encoded), (names[1], "bad")]))["classification"],
                "duplicate"
            );
        }
        let mut wrong = claim.clone();
        wrong["network"] = json!("eip155:8453");
        assert_eq!(
            evidence(&response(&[(
                "x-payment-response",
                &BASE64_STANDARD.encode(wrong.to_string())
            )]))["classification"],
            "malformed"
        );
    }
}
