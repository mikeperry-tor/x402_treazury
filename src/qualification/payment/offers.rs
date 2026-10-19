//! A bounded, redacted projection of a seller's untrusted HTTP 402 header.
//! This observation neither selects an offer nor grants signing authority.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

const HEADER_BYTES: usize = 65536;
const OFFERS: usize = 128;

fn summary(headers: &reqwest::header::HeaderMap) -> Value {
    let mut headers = headers.get_all("payment-required").iter();
    let Some(header) = headers.next() else {
        return json!({"status":"header_absent","offers":[]});
    };
    if headers.next().is_some() {
        return json!({"status":"duplicate_headers","offers":[]});
    }
    if header.as_bytes().len() > HEADER_BYTES {
        tracing::warn!(
            limit_bytes = HEADER_BYTES,
            "qualification challenge header observation exceeded limit; no partial offer list recorded"
        );
        return json!({"status":"header_limit_exceeded","limit_bytes":HEADER_BYTES,"offers":[]});
    }
    let digest = format!("{:x}", Sha256::digest(header.as_bytes()));
    let parsed = STANDARD
        .decode(header.as_bytes())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let Some(challenge) = parsed else {
        return json!({"status":"malformed_header","header_sha256":digest,"offers":[]});
    };
    let Some(offers) = challenge["accepts"].as_array() else {
        return json!({"status":"malformed_offers","header_sha256":digest,"offers":[]});
    };
    if offers.len() > OFFERS {
        tracing::warn!(
            limit_offers = OFFERS,
            observed_offers = offers.len(),
            "qualification offer observation exceeded limit; no partial offer list recorded"
        );
        return json!({"status":"offer_limit_exceeded","limit_offers":OFFERS,"observed_offers":offers.len(),"header_sha256":digest,"offers":[]});
    }
    let v1 = challenge["x402Version"] == 1;
    let rows: Vec<_> = offers
        .iter()
        .enumerate()
        .map(|(index, offer)| {
            let base_network = if v1 {
                offer["network"] == "base"
            } else {
                offer["network"] == "eip155:8453"
            };
            let base_usdc = base_network
                && offer["asset"]
                    .as_str()
                    .is_some_and(|asset| asset.eq_ignore_ascii_case(crate::payment::USDC));
            let amount_field = if v1 {
                &offer["maxAmountRequired"]
            } else {
                &offer["amount"]
            };
            let amount = amount_field
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 78 && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| U256::from_str_radix(s, 10).ok());
            json!({"index":index,"base_usdc":base_usdc,
                "scheme":match offer["scheme"].as_str() {Some("exact")=>"exact",Some("upto")=>"upto",_=>"other_or_missing"},
                "amount_atomic":amount.map(|n|n.to_string()),
                "above_one_cent":if base_usdc {amount.map(|n|n>U256::from(10000))} else {None}})
        })
        .collect();
    json!({"status":"observed","version":challenge["x402Version"].as_u64(),"header_sha256":digest,"offers":rows})
}

pub async fn record(headers: &reqwest::header::HeaderMap) -> Result<()> {
    let Ok(context) = CASE.try_with(Clone::clone) else {
        return Ok(());
    };
    let observation = summary(headers);
    tokio::task::spawn_blocking(move || {
        let mut db = connection(&context.guard.binding)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claim: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_claim' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?3)",
            params![context.guard.binding.run,context.case,context.guard.binding.session], |r| r.get(0))?;
        ensure!(claim,"challenge observation lacks application claim");
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM events WHERE run=?1 AND kind='application_challenge' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2",
            params![context.guard.binding.run,context.case], |r|r.get(0))?;
        let event = json!({"case":context.case,"session":context.guard.binding.session,"ordinal":count,
            "observed_micros":context.elapsed_micros(),"observation":observation});
        tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_challenge',?2,?3)",
            params![context.guard.binding.run,event.to_string(),now()?])?;
        tx.commit()?;
        Ok(())
    }).await.context("challenge evidence worker stopped")?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(value: Value) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "payment-required",
            STANDARD.encode(value.to_string()).parse().unwrap(),
        );
        h
    }
    #[tokio::test]
    async fn normalized_body_only_v1_offers_are_projected_without_private_fields() {
        let response = reqwest::Response::from(axum::http::Response::builder().status(402).body(
            json!({"x402Version":1,"accepts":[{"network":"base","asset":crate::payment::USDC,"scheme":"exact","maxAmountRequired":"50000","resource":"private","description":"private","payTo":"private"}]}).to_string()
        ).unwrap());
        let response = crate::payment::bound_challenge(response, crate::limits::RESPONSE_BYTES)
            .await
            .unwrap();
        let observed = summary(response.headers());
        assert_eq!(observed["status"], "observed");
        assert_eq!(observed["version"], 1);
        assert_eq!(observed["offers"][0]["base_usdc"], true);
        assert_eq!(observed["offers"][0]["amount_atomic"], "50000");
        assert_eq!(observed["offers"][0]["above_one_cent"], true);
        assert!(!observed.to_string().contains("private"));
    }
    #[test]
    fn unselected_prices_are_observed_without_private_metadata_or_authority() {
        let h = headers(
            json!({"x402Version":2,"resource":{"description":"private"},"extensions":{"private":true},"accepts":[
            {"network":"eip155:8453","asset":crate::payment::USDC,"scheme":"exact","amount":"10000","payTo":"private"},
            {"network":"eip155:8453","asset":crate::payment::USDC,"scheme":"upto","amount":"10001"},
            {"network":"other","asset":"other","amount":"10001"},
            {"network":"eip155:8453","asset":crate::payment::USDC,"amount":"not-a-number"}]}),
        );
        let v = summary(&h);
        assert_eq!(v["offers"][0]["above_one_cent"], false);
        assert_eq!(v["offers"][1]["above_one_cent"], true);
        assert!(v["offers"][2]["above_one_cent"].is_null());
        assert!(v["offers"][3]["amount_atomic"].is_null());
        assert!(!v.to_string().contains("private"));
    }
    #[test]
    fn malformed_duplicate_and_bounded_headers_never_become_empty_successes() {
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen().unwrap())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let mut h = reqwest::header::HeaderMap::new();
        assert_eq!(summary(&h)["status"], "header_absent");
        h.insert("payment-required", "invalid".parse().unwrap());
        assert_eq!(summary(&h)["status"], "malformed_header");
        h.append("payment-required", "invalid".parse().unwrap());
        assert_eq!(summary(&h)["status"], "duplicate_headers");
        h = headers(json!({"accepts":vec![json!({});OFFERS+1]}));
        assert_eq!(summary(&h)["status"], "offer_limit_exceeded");
        assert_eq!(summary(&h)["offers"], json!([]));
        h.insert(
            "payment-required",
            "x".repeat(HEADER_BYTES + 1).parse().unwrap(),
        );
        assert_eq!(summary(&h)["status"], "header_limit_exceeded");
        let logs = std::fs::read_to_string(log.path()).unwrap();
        assert!(logs.contains("limit_offers=128"));
        assert!(logs.contains("limit_bytes=65536"));
        assert!(logs.contains("no partial offer list recorded"));
    }
}
