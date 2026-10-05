//! Validate case-correlated canonical debit proofs independently of seller claims.
use super::*;
use std::collections::BTreeMap;
impl Registry {
    pub fn record_receipt_observations(&self, run: &str, events: &[Value], now: i64) -> Result<()> {
        self.application_evidence(run)?;
        let tx = self.db.unchecked_transaction()?;
        for event in events {
            let attempt = event["attempt_id"]
                .as_str()
                .context("observation attempt missing")?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_debit_verified' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?2)", params![run,attempt], |r| r.get(0))?;
            if exists {
                continue;
            }
            self.event(run, "application_debit_check", event, now)?;
            if event["status"] == "verified" {
                self.event(run, "application_debit_verified", event, now)?;
            }
        }
        self.application_evidence(run)?;
        tx.commit()?;
        Ok(())
    }
    pub fn verified_debits(&self, run: &str) -> Result<BTreeMap<String, Value>> {
        let mut query = self.db.prepare("SELECT detail FROM events WHERE run=?1 AND kind='application_debit_verified' ORDER BY seq LIMIT 10001")?;
        let rows = query
            .query_map([run], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= 10000,
            "debit proof report exceeds 10000-event limit"
        );
        let mut proofs = BTreeMap::new();
        let mut authorizations = std::collections::BTreeSet::new();
        for raw in rows {
            let event: Value = serde_json::from_str(&raw)?;
            let case = event["case"].as_str().context("debit proof case missing")?;
            let load = |kind: &str| -> Result<Value> {
                let raw:String = self.db.query_row("SELECT detail FROM events WHERE run=?1 AND kind=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?3",
                    params![run,kind,case], |r|r.get(0))?;
                Ok(serde_json::from_str(&raw)?)
            };
            let payment = load("application_payment")?;
            let receipt = load("application_receipt")?;
            for original in [&payment, &receipt] {
                ensure!(
                    event["attempt_id"] == original["attempt_id"]
                        && event["session"] == original["session"],
                    "canonical debit proof correlation differs"
                );
            }
            let proof = &event["proof"];
            let payer: alloy_primitives::Address = proof["payer"]
                .as_str()
                .context("debit payer missing")?
                .parse()?;
            let admitted: alloy_primitives::Address = payment["address"]
                .as_str()
                .context("admitted payer missing")?
                .parse()?;
            let _: alloy_primitives::Address = proof["payee"]
                .as_str()
                .context("debit payee missing")?
                .parse()?;
            let _: alloy_primitives::B256 = proof["nonce"]
                .as_str()
                .context("debit nonce missing")?
                .parse()?;
            let _: alloy_primitives::B256 = proof["block_hash"]
                .as_str()
                .context("debit block hash missing")?
                .parse()?;
            ensure!(
                payer == admitted
                    && proof["amount_atomic"] == payment["amount"]
                    && proof["transaction"] == receipt["receipt"]["transaction"]
                    && proof["block_height"].as_u64().is_some()
                    && receipt["receipt"]["classification"] == "seller_success",
                "canonical debit proof differs from admitted payment or seller receipt"
            );
            ensure!(
                authorizations.insert((
                    proof["transaction"]
                        .as_str()
                        .context("debit transaction missing")?
                        .to_owned(),
                    payer,
                    proof["nonce"]
                        .as_str()
                        .context("debit nonce missing")?
                        .parse::<alloy_primitives::B256>()?,
                )),
                "canonical payment attributed to more than one case"
            );
            ensure!(
                proofs.insert(case.to_owned(), proof.clone()).is_none(),
                "duplicate canonical debit proof"
            );
        }
        Ok(proofs)
    }
}
