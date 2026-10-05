//! Bounded seller claims, never canonical chain evidence. No payload/signature is retained.
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const HEADER_LIMIT: usize = 16 * 1024;

pub(super) fn evidence(result: &anyhow::Result<reqwest::Response>) -> Value {
    let Ok(response) = result else {
        return json!({"classification":"transport_unknown"});
    };
    let values: Vec<_> = response
        .headers()
        .get_all("payment-response")
        .iter()
        .collect();
    let mut receipt = match values.as_slice() {
        [] => json!({"classification":"missing"}),
        [header] => parse(header.as_bytes()),
        _ => json!({"classification":"duplicate"}),
    };
    receipt["http_status"] = json!(response.status().as_u16());
    receipt
}
fn parse(raw: &[u8]) -> Value {
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
    if value["network"] != "eip155:8453" {
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
}
