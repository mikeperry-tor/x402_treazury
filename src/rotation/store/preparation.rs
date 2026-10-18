//! Accepted SDK calculations and their exact input owner, never a capital ledger.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputPool {
    Transparent,
    Sapling,
    Orchard,
    Ironwood,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct OwnedInput {
    pub pool: InputPool,
    pub txid: String,
    pub output_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CalculationInputs {
    pub destination: String,
    pub inputs: Vec<OwnedInput>,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct PreparationIntent {
    pub pool: Option<String>,
    pub cost_zatoshis: u64,
    pub calculation: Option<CalculationInputs>,
}

impl Store {
    pub(super) fn preparation_intent(&self, id: &str) -> Result<PreparationIntent> {
        let bytes: Vec<u8> = self.db.query_row(
            "SELECT preparation FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL UNION ALL SELECT preparation FROM outgoing WHERE id=?1",
            [id], |r| r.get(0),
        ).optional()?.context("accepted preparation missing")?;
        let plain = unseal(
            &self.key,
            &format!("v1:{}:{}:preparation:{id}", self.id, self.network.name()),
            &bytes,
        )?;
        serde_json::from_slice(&plain).context("invalid accepted preparation")
    }

    /// The actual proposal cost and owned input identities are captured before
    /// SDK calculation, atomically with its funding preparation phase. An existing
    /// intent cannot authorize another calculation or reset its later phase.
    pub fn accept_preparation(
        &mut self,
        id: &str,
        pool: Option<&str>,
        cost: i64,
        calculation: Option<CalculationInputs>,
    ) -> Result<()> {
        ensure!(cost > 0, "invalid preparation cost");
        let intent = PreparationIntent {
            pool: pool.map(str::to_owned),
            cost_zatoshis: u64::try_from(cost)?,
            calculation,
        };
        let accepted: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM operation_network WHERE operation_id=?1 AND preparation IS NOT NULL) OR EXISTS(SELECT 1 FROM outgoing WHERE id=?1)", [id], |r| r.get(0))?;
        if accepted {
            ensure!(self.preparation_intent(id)? == intent, "operation_conflict");
            return Ok(());
        }
        self.funding_restriction
            .require_new_funding("source_preparation_start")?;
        let job:Option<(String,String)>=self.db.query_row("SELECT j.job_id,f.pool_id FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id WHERE j.operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((job, owner_pool)) = job {
            let archived: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM funding_recovery WHERE operation_id=?1)",
                [id],
                |r| r.get(0),
            )?;
            ensure!(!archived, "funding attempt closed");
            ensure!(
                intent.pool.as_deref() == Some(owner_pool.as_str()),
                "funding source pool mismatch"
            );
            ensure!(
                capacity::funding_claim_present(&self.db, &job)?,
                "initial funding claim consumed or disabled"
            );
        }

        ensure!(!self.preparation_pending()?, "treasury_send_pending");
        if let Some(pool) = pool {
            let enabled: bool =
                self.db
                    .query_row("SELECT enabled FROM pools WHERE id=?1", [pool], |r| {
                        r.get(0)
                    })?;
            ensure!(enabled, "pool disabled");
        }
        self.check_operation_permit(id, intent.cost_zatoshis)?;
        let bytes = seal(
            &self.key,
            &format!("v1:{}:{}:preparation:{id}", self.id, self.network.name()),
            &serde_json::to_vec(&intent)?,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO operation_network(operation_id,preparation) VALUES(?1,?2) ON CONFLICT(operation_id) DO UPDATE SET preparation=excluded.preparation",params![id,bytes])?;
        tx.execute(
            "UPDATE funding_progress SET phase='\"PREPARING\"',last_error=NULL WHERE operation_id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn preparation_pending(&self) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED') OR EXISTS(SELECT 1 FROM operation_network WHERE preparation IS NOT NULL)", [], |r| r.get(0))?)
    }

    /// Read-only authority/readiness check. Qualification permits currently
    /// authorize funding jobs only, so they cannot authorize jobless shielding.
    pub(crate) fn automatic_shielding_available(&self) -> Result<bool> {
        if self.funding_restriction != super::super::restriction::FundingRestriction::Ordinary
            || self.funding_permits.is_some()
        {
            return Ok(false);
        }
        Ok(!self.preparation_pending()?)
    }

    #[cfg(feature = "zcash")]
    pub(crate) fn automatic_shielding_submission(&self, id: &str, max_fee: u64) -> Result<bool> {
        if self.funding_restriction != super::super::restriction::FundingRestriction::Ordinary
            || self.funding_permits.is_some()
        {
            return Ok(false);
        }
        let operation = self.operation(id)?;
        Ok(operation.attempts == 0
            && operation.facts.fee_zatoshis <= max_fee
            && self
                .shielding_identity(id, Some(&operation.facts))?
                .is_some())
    }

    #[cfg(feature = "zcash")]
    pub(crate) fn shielding_identity(
        &self,
        id: &str,
        facts: Option<&TransactionFacts>,
    ) -> Result<Option<crate::network::IsolationId>> {
        let Some(facts) = facts else {
            return Ok(None);
        };
        if facts.amount_zatoshis != 0 || facts.deadline.is_some() {
            return Ok(None);
        }
        let intent = self.preparation_intent(id)?;
        let Some(calculation) = intent.calculation else {
            return Ok(None);
        };
        if intent.pool.is_some()
            || intent.cost_zatoshis != facts.fee_zatoshis
            || calculation.inputs.is_empty()
            || calculation
                .inputs
                .iter()
                .any(|i| i.pool != InputPool::Transparent)
        {
            return Ok(None);
        }
        // Invalid completed facts must not prevent retaining exact local bytes.
        Ok(crate::network::IsolationId::transparent_treasury(
            self.network,
            &self.id,
            &calculation.destination,
        )
        .ok())
    }

    pub(crate) fn prepared_inputs(&self, id: &str) -> Result<Vec<OwnedInput>> {
        self.preparation_intent(id)?
            .calculation
            .context("original selected input inventory missing")
            .map(|v| v.inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_owner_survives_policy_change_reopen_and_byte_publication() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"initial")?;
        let owner = store.id().to_owned();
        let manifest = CalculationInputs {
            destination: "original-deposit".into(),
            inputs: vec![OwnedInput {
                pool: InputPool::Orchard,
                txid: format!("{:064x}", 1),
                output_index: 2,
            }],
        };
        store.accept_preparation("operation", None, 60, Some(manifest.clone()))?;
        let before = store.qualification_state()?;
        assert_eq!(
            before["pending_source_operations"],
            serde_json::json!(["operation"])
        );
        assert!(store.abandon_unprepared("operation").is_err());
        assert!(store.accept_preparation("other", None, 1, None).is_err());
        drop(store);
        let mut store = Store::open(&state, &key, &owner)?;
        store.deny_new_funding();
        let facts = TransactionFacts {
            txid: "original".into(),
            expiry_height: 0,
            amount_zatoshis: 50,
            fee_zatoshis: 10,
            deadline: None,
        };
        assert_eq!(
            store.prepare_with_facts(
                "operation",
                1,
                b"calculated",
                b"exact",
                Some(facts.clone())
            )?,
            2
        );
        assert_eq!(
            store.prepare_with_facts("operation", 1, b"calculated", b"exact", Some(facts))?,
            2
        );
        assert_eq!(store.prepared_inputs("operation")?, manifest.inputs);
        let after = store.qualification_state()?;
        assert_eq!(
            before["source_preparations"][0]["preparation_sha256"],
            after["source_preparations"][0]["preparation_sha256"]
        );
        assert!(before["source_preparations"][0]["prepared_record_sha256"].is_null());
        assert!(after["source_preparations"][0]["prepared_record_sha256"].is_string());
        assert_eq!(
            after["pending_source_operations"],
            serde_json::json!(["operation"])
        );
        assert!(store.request_broadcast("operation", 1, 1, false).is_err());
        assert!(store.status()?.legacy_refunds.is_none());
        Ok(())
    }
}

#[cfg(all(test, feature = "zcash"))]
mod shielding_identity_tests {
    use super::*;
    use crate::rotation::transaction::PreparedTransaction;
    #[test]
    fn shielding_identity_reloads_unchanged_and_explicit_recipient_takes_precedence() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let mut store = Store::create(&state, &key, 1, b"fixture")?;
        let owner = store.id().to_owned();
        let address = "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F";
        store.accept_preparation(
            "shield",
            None,
            100,
            Some(CalculationInputs {
                destination: address.into(),
                inputs: vec![OwnedInput {
                    pool: InputPool::Transparent,
                    txid: "owned input".into(),
                    output_index: 0,
                }],
            }),
        )?;
        store.prepare_with_facts(
            "shield",
            1,
            b"next",
            b"exact",
            Some(TransactionFacts {
                txid: "shield".into(),
                expiry_height: 100,
                amount_zatoshis: 0,
                fee_zatoshis: 100,
                deadline: None,
            }),
        )?;
        let original = PreparedTransaction::load(&store, "shield")?.network_identity()?;
        drop(store);
        let mut store = Store::open(&state, &key, &owner)?;
        assert_eq!(
            PreparedTransaction::load(&store, "shield")?.network_identity()?,
            original
        );
        let recipient = format!("0x{:040x}", 1);
        store.bind_operation_recipient("shield", &recipient)?;
        let explicit = PreparedTransaction::load(&store, "shield")?;
        assert_eq!(
            explicit.network_identity()?,
            crate::network::IsolationId::evm(&recipient)?
        );
        assert_ne!(explicit.network_identity()?, original);
        assert_eq!(explicit.bytes(), b"exact");
        Ok(())
    }
}
