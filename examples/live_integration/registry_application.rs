//! Validate application acceptance/completion separately from driver observations.
use super::*;
use std::collections::BTreeMap;
impl Registry {
    pub fn application_failure(&self, run: &str, case: &str) -> Result<Option<String>> {
        let (count, category):(i64,Option<String>) = self.db.query_row("SELECT COUNT(*),MIN(json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.failure_category')) FROM events WHERE run=?1 AND kind='application_finished' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2", params![run,case], |r| Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(
            count == 1,
            "expected exactly one application completion for case {case}"
        );
        Ok(category)
    }
    pub fn application_evidence(&self, run: &str) -> Result<Value> {
        let m = self.manifest(run)?;
        let cases: BTreeMap<_, _> = m.cases.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut query=self.db.prepare("SELECT kind,detail FROM events WHERE run=?1 AND kind IN ('application_claim','application_finished') ORDER BY seq LIMIT 20001")?;
        let rows = query
            .query_map([run], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= 20000,
            "application evidence exceeds 20000-event run limit"
        );
        let mut claims = BTreeMap::new();
        let mut finished = BTreeMap::new();
        for (kind, raw) in rows {
            let event: Value = serde_json::from_str(&raw)?;
            let id = event["case"]
                .as_str()
                .context("application event missing case ID")?
                .to_owned();
            let case = cases
                .get(id.as_str())
                .context("application event names an unreviewed case")?;
            ensure!(
                event["session"].as_str().is_some_and(identifier),
                "application event session missing/invalid"
            );
            if kind == "application_claim" {
                ensure!(
                    event["mode"] == (if case.unsigned { "unsigned" } else { "managed" })
                        && event["server"] == case.server
                        && event["source"] == case.source
                        && event["tool"] == case.tool,
                    "application claim differs from reviewed binding"
                );
                ensure!(
                    !matches!(
                        self.execution_state(run, &id)?.as_str(),
                        "UNATTEMPTED" | "SKIPPED_TARGET_REACHED"
                    ),
                    "application accepted an unreserved case"
                );
                ensure!(
                    claims.insert(id, event).is_none(),
                    "application accepted a case more than once"
                );
            } else {
                let claim = claims
                    .get(&id)
                    .context("application completed without a claim")?;
                ensure!(
                    claim["session"] == event["session"] && event["is_error"].is_boolean(),
                    "application completion does not match claim"
                );
                ensure!(
                    event["finished_micros"]
                        .as_u64()
                        .zip(claim["started_micros"].as_u64())
                        .is_some_and(|(end, start)| end >= start),
                    "application interval is missing or invalid"
                );
                ensure!(
                    finished.insert(id, event).is_none(),
                    "duplicate application completion"
                );
            }
        }
        for id in cases.keys() {
            let (execution, semantic): (String, String) = self.db.query_row(
                "SELECT execution,semantic FROM cases WHERE run=?1 AND id=?2",
                params![run, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if execution == "COMPLETED" {
                let event = finished
                    .get(*id)
                    .context("completed MCP case lacks application completion evidence")?;
                ensure!(
                    (semantic == "PASSED") == (event["is_error"] == false),
                    "MCP/application result disagreement"
                );
            }
        }
        let pending: Vec<_> = claims
            .keys()
            .filter(|id| !finished.contains_key(*id))
            .collect();
        let payment_attempts = self.payment_evidence(run, &claims)?;
        let receipts = self.receipt_evidence(run, payment_attempts)?;
        let debits = self.verified_debits(run)?;
        Ok(
            json!({"verified_debits":debits,"seller_receipts":receipts,"claims":claims.len(),"finished":finished.len(),"without_completion":pending,"payment_attempts_correlated":payment_attempts,"scope":"application acceptance/completion, pre-signing admission correlation and canonical receipt debit proofs; settlement status and Tor isolation are reported separately"}),
        )
    }
    fn receipt_evidence(&self, run: &str, admissions: usize) -> Result<Value> {
        let mut query = self.db.prepare("SELECT detail FROM events WHERE run=?1 AND kind='application_receipt' ORDER BY seq LIMIT 10001")?;
        let rows = query
            .query_map([run], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= 10000,
            "receipt evidence exceeds 10000-event limit"
        );
        let mut seen = std::collections::BTreeSet::new();
        let mut counts = BTreeMap::<String, usize>::new();
        for raw in &rows {
            let event: Value = serde_json::from_str(raw)?;
            let case = event["case"].as_str().context("receipt case missing")?;
            ensure!(
                seen.insert(case.to_owned()),
                "duplicate receipt observation"
            );
            let payment: String = self.db.query_row(
                "SELECT detail FROM events WHERE run=?1 AND kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2",
                params![run,case], |r| r.get(0)).context("receipt lacks payment admission")?;
            let payment: Value = serde_json::from_str(&payment)?;
            ensure!(
                event["session"] == payment["session"]
                    && event["attempt_id"] == payment["attempt_id"],
                "receipt differs from admitted payment"
            );
            let receipt = &event["receipt"];
            let classification = receipt["classification"]
                .as_str()
                .context("receipt classification missing")?;
            ensure!(
                matches!(
                    classification,
                    "missing"
                        | "duplicate"
                        | "oversized"
                        | "malformed"
                        | "seller_success"
                        | "seller_failure"
                        | "transport_unknown"
                ),
                "unknown receipt classification"
            );
            if classification == "seller_success" {
                ensure!(
                    receipt["network"] == "eip155:8453"
                        && receipt["transaction"]
                            .as_str()
                            .and_then(|s| s.parse::<alloy_primitives::B256>().ok())
                            .is_some_and(|v| !v.is_zero()),
                    "invalid successful seller receipt"
                );
                if let Some(payer) = receipt.get("payer") {
                    let payer: alloy_primitives::Address =
                        payer.as_str().context("receipt payer malformed")?.parse()?;
                    let admitted: alloy_primitives::Address = payment["address"]
                        .as_str()
                        .context("admitted payer missing")?
                        .parse()?;
                    ensure!(
                        payer == admitted,
                        "seller receipt payer differs from admitted wallet"
                    );
                }
            }
            *counts.entry(classification.into()).or_default() += 1;
        }
        Ok(
            json!({"recorded":rows.len(),"without_observation":admissions.checked_sub(rows.len()).context("receipts exceed admissions")?,
            "classifications":counts,"meaning":"seller claims only; transaction receipt and USDC debit still require chain verification"}),
        )
    }
    fn payment_evidence(&self, run: &str, claims: &BTreeMap<String, Value>) -> Result<usize> {
        let m = self.manifest(run)?;
        let mut query = self.db.prepare("SELECT detail FROM events WHERE run=?1 AND kind='application_payment' ORDER BY seq LIMIT 10001")?;
        let rows = query
            .query_map([run], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= 10000,
            "payment evidence exceeds 10000-attempt run limit"
        );
        let mut cases = std::collections::BTreeSet::new();
        let mut attempts = std::collections::BTreeSet::new();
        for raw in &rows {
            let event: Value = serde_json::from_str(raw)?;
            let case = event["case"]
                .as_str()
                .context("payment evidence lacks case")?;
            let reviewed = m
                .cases
                .iter()
                .find(|c| c.id == case)
                .context("payment has unreviewed case")?;
            let claim = claims
                .get(case)
                .context("payment lacks application claim")?;
            ensure!(
                !reviewed.unsigned
                    && claim["session"] == event["session"]
                    && event["stage"] == "admitted_before_signing",
                "payment differs from accepted case"
            );
            let amount: u64 = event["amount"]
                .as_str()
                .context("payment amount missing")?
                .parse()?;
            ensure!(
                amount > 0 && amount <= super::atomic_usdc(&reviewed.reserve_usdc)?,
                "payment exceeds case reservation"
            );
            ensure!(
                cases.insert(case.to_owned())
                    && attempts.insert(
                        event["attempt_id"]
                            .as_str()
                            .context("payment attempt missing")?
                            .to_owned()
                    ),
                "duplicate payment case/attempt evidence"
            );
        }
        Ok(rows.len())
    }
}
