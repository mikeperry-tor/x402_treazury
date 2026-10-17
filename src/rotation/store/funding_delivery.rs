//! Bounded provider hints and immutable canonical destination evidence. Neither
//! is source accounting, refund closure, or current payment capacity.
use super::*;
#[cfg(feature = "zcash")]
use crate::rotation::{
    base::{EvidenceCapture, FundingDeliveryExpectation, FundingDeliveryProof},
    near::{Quote, StatusObservation},
};
#[cfg(feature = "zcash")]
use alloy_primitives::B256;
#[cfg(feature = "zcash")]
use sha2::{Digest, Sha256};

#[cfg(feature = "zcash")]
#[derive(Serialize, Deserialize)]
struct Hint {
    source: B256,
    destination: B256,
    amount_in: Option<U256>,
    amount_out: Option<U256>,
    deposited: Option<U256>,
    refunded: Option<U256>,
}
#[cfg(feature = "zcash")]
type DeliveryHintRow = (Option<Vec<u8>>, Option<Vec<u8>>, Option<String>, bool);

/// The ticket owns observation only. It cannot release any financial liability.
#[cfg(feature = "zcash")]
pub(crate) struct DeliveryRead {
    namespace: Uuid,
    job: String,
    operation: String,
    ticket: String,
    binding: [u8; 32],
    event_candidate: bool,
    pub(crate) expected: FundingDeliveryExpectation,
    pub(crate) capture: EvidenceCapture,
}
#[cfg(feature = "zcash")]
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
#[cfg(feature = "zcash")]
impl Store {
    fn delivery_quote(&self, job: &str, operation: &str) -> Result<Quote> {
        Ok(serde_json::from_slice(
            &self.funding_quote_for_operation(job, operation)?,
        )?)
    }
    fn delivery_aad(&self, operation: &str) -> String {
        format!(
            "v1:{}:{}:delivery:{operation}",
            self.id,
            self.network.name()
        )
    }
    fn delivery_binding(
        &self,
        job: &str,
        operation: &str,
    ) -> Result<([u8; 32], String, Quote, B256)> {
        let quote = self.delivery_quote(job, operation)?;
        let (wallet,address,accepted,recipient,facts,raw):(String,String,Option<String>,Option<String>,String,Vec<u8>)=
            self.db.query_row("SELECT f.wallet_id,w.address,j.accepted_output,j.accepted_recipient,o.facts,x.raw FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id JOIN wallets w ON w.id=f.wallet_id JOIN treasury_operations o ON o.id=j.operation_id JOIN outgoing x ON x.id=o.id WHERE f.id=?1 AND j.operation_id=?2",params![job,operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        let target = quote.request["amount"]
            .as_str()
            .context("delivery accepted amount missing")?;
        let payer: alloy_primitives::Address = address.parse()?;
        ensure!(
            quote.request["recipient"]
                .as_str()
                .and_then(|v| v.parse::<alloy_primitives::Address>().ok())
                == Some(payer)
                && accepted.as_deref() == Some(target)
                && recipient
                    .as_deref()
                    .and_then(|v| v.parse::<alloy_primitives::Address>().ok())
                    == Some(payer),
            "delivery destination binding mismatch"
        );
        let parsed: TransactionFacts = serde_json::from_str(&facts)?;
        #[cfg(feature = "zcash")]
        {
            let transaction =
                crate::treasury::submission::decode(&self.prepared_bytes(operation)?)?;
            ensure!(
                transaction.txid().to_string() == parsed.txid
                    && u32::from(transaction.expiry_height()) == parsed.expiry_height,
                "durable transaction identity mismatch"
            );
        }
        let source: B256 = parsed
            .txid
            .parse()
            .map_err(|_| anyhow::anyhow!("delivery source identity malformed"))?;
        let binding = digest(&serde_json::to_vec(&(
            job, operation, &wallet, &address, &accepted, &recipient, &quote, &facts, &raw,
        ))?);
        Ok((binding, wallet, quote, source))
    }
    /// Fixed-size normalized fields only; arbitrary provider prose/URLs and
    /// changing unknown fields never enter durable hint storage.
    pub(crate) fn record_delivery_observation(
        &mut self,
        job: &str,
        operation: &str,
        observation: &StatusObservation,
        observed_at: u64,
    ) -> Result<()> {
        use crate::rotation::near::SwapStatus;
        let quote = self.delivery_quote(job, operation)?;
        let quote_digest = digest(&serde_json::to_vec(&quote)?);
        ensure!(
            observation.quote_digest() == &quote_digest,
            "delivery quote correlation mismatch"
        );
        let hint = observation
            .settlement()
            .filter(|s| s.origin_transactions.len() == 1 && s.destination_transactions.len() == 1)
            .map(|s| Hint {
                source: s.origin_transactions[0],
                destination: s.destination_transactions[0],
                amount_in: s.amount_in,
                amount_out: s.amount_out,
                deposited: s.deposited_amount,
                refunded: s.refunded_amount,
            });
        let return_binding = if observation.reported_refund().is_some() {
            let (_, _, bound_quote, source) = self.delivery_binding(job, operation)?;
            if FundingDeliveryExpectation::from_observation(
                operation.parse()?,
                &bound_quote,
                observation,
                source,
            )
            .is_ok()
            {
                let destination = observation
                    .settlement()
                    .expect("validated correlation")
                    .destination_transactions[0];
                Some(return_correlation(source, destination, &quote_digest))
            } else {
                None
            }
        } else {
            None
        };
        let encrypted = hint
            .map(|hint| -> Result<Vec<u8>> {
                seal(
                    &self.key,
                    &self.delivery_aad(operation),
                    &serde_json::to_vec(&hint)?,
                )
            })
            .transpose()?;
        let issues = if observation.issues().is_empty() && encrypted.is_some() {
            0i64
        } else {
            1i64
        };
        let declared_return = observation.reported_refund().map(|value| value.to_string());
        let return_conflict = observation
            .issues()
            .contains(&crate::rotation::near::SettlementIssue::InvalidRefundedAmount);
        let status = observation.status();
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE funding_progress SET provider_success=MAX(provider_success,?3),provider_failed=MAX(provider_failed,?4),provider_refunded=MAX(provider_refunded,?5),provider_incomplete=MAX(provider_incomplete,?6),provider_observed_at=MAX(COALESCE(provider_observed_at,0),?7) WHERE job_id=?1 AND operation_id=?2 AND quote IS NOT NULL",params![job,operation,matches!(status,SwapStatus::Success),matches!(status,SwapStatus::Failed),matches!(status,SwapStatus::Refunded),matches!(status,SwapStatus::IncompleteDeposit),i64::try_from(observed_at)?])?;
        ensure!(changed == 1, "delivery operation changed");
        let retained=tx.execute("INSERT INTO funding_delivery_hints(operation_id,job_id,quote_digest,queued,issue_bits,declared_return,return_conflict,return_binding) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(operation_id) DO UPDATE SET queued=COALESCE(excluded.queued,queued),issue_bits=issue_bits|excluded.issue_bits,declared_return=COALESCE(declared_return,excluded.declared_return),return_binding=CASE WHEN declared_return IS NULL OR declared_return=excluded.declared_return THEN COALESCE(excluded.return_binding,return_binding) ELSE return_binding END,return_conflict=MAX(return_conflict,excluded.return_conflict,CASE WHEN declared_return IS NOT NULL AND excluded.declared_return IS NOT NULL AND declared_return!=excluded.declared_return THEN 1 ELSE 0 END) WHERE job_id=excluded.job_id AND quote_digest=excluded.quote_digest",params![operation,job,quote_digest.as_slice(),encrypted,issues,declared_return,return_conflict,return_binding.as_ref().map(|value|value.as_slice())])?;
        ensure!(retained == 1, "delivery hint binding changed");
        funding::derive_operation_phase(&tx, job, operation)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn capture_funding_delivery(
        &mut self,
        job: &str,
        operation: &str,
    ) -> Result<Option<DeliveryRead>> {
        let row:Option<DeliveryHintRow>=self.db.query_row("SELECT active,queued,ticket,prefer_event FROM funding_delivery_hints WHERE operation_id=?1 AND job_id=?2",params![operation,job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (active, queued, ticket, prefer_event) = row.unwrap_or((None, None, None, true));
        let accepted:Option<(String,String)>=self.db.query_row("SELECT e.transaction_hash,COALESCE(d.reason,'') FROM funding_delivery_events e JOIN funding_delivery_current c ON c.event_id=e.id AND c.operation_id=e.operation_id JOIN funding_delivery_proofs p ON p.id=c.proof_id AND p.event_id=e.id LEFT JOIN funding_delivery_disputes d ON d.proof_id=p.id WHERE e.job_id=?1 AND e.operation_id=?2 ORDER BY e.id DESC LIMIT 1",params![job,operation],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if active.is_none() && queued.is_none() && accepted.is_none() {
            return Ok(None);
        }
        let (binding, wallet, quote, source) = self.delivery_binding(job, operation)?;
        let was_active = active.is_some();
        // Endpoint distrust requires revalidation of the accepted event itself.
        // A chain-disputed event alternates with the queued correction; only a
        // completed read advances this finite cache cursor.
        let authoritative = active.is_none()
            && accepted.as_ref().is_some_and(|(_, reason)| {
                reason != "chain_repair_required" || queued.is_none() || prefer_event
            });
        let bytes = if let Some(active) = active {
            active
        } else if authoritative {
            let destination = &accepted.as_ref().expect("accepted event selected").0;
            let hint = Hint {
                source,
                destination: destination.parse()?,
                amount_in: None,
                amount_out: None,
                deposited: None,
                refunded: None,
            };
            seal(
                &self.key,
                &self.delivery_aad(operation),
                &serde_json::to_vec(&hint)?,
            )?
        } else {
            queued.context("delivery hint disappeared")?
        };
        let hint: Hint =
            serde_json::from_slice(&unseal(&self.key, &self.delivery_aad(operation), &bytes)?)?;
        let event_candidate = accepted
            .as_ref()
            .is_some_and(|(hash, _)| hash.parse::<B256>().ok() == Some(hint.destination));
        let value = serde_json::json!({"status":"UNKNOWN","quoteResponse":quote.response,"swapDetails":{
            "originChainTxHashes":[{"hash":format!("{:x}",hint.source)}],
            "destinationChainTxHashes":[{"hash":hint.destination.to_string()}],
            "amountIn":hint.amount_in.map(|v|v.to_string()),"amountOut":hint.amount_out.map(|v|v.to_string()),
            "depositedAmount":hint.deposited.map(|v|v.to_string()),"refundedAmount":hint.refunded.map(|v|v.to_string())}});
        let observation = crate::rotation::near::restore_status_observation(&quote, &value)?;
        let expected = match FundingDeliveryExpectation::from_observation(
            operation.parse()?,
            &quote,
            &observation,
            source,
        ) {
            Ok(expected) => expected,
            Err(error) => {
                // This is rejected unsigned correlation, not unresolved signed
                // work. Preserve a corrected queued candidate and finite issue.
                if was_active {
                    self.db.execute("UPDATE funding_delivery_hints SET active=NULL,ticket=NULL,issue_bits=issue_bits|2 WHERE operation_id=?1",[operation])?;
                } else {
                    self.db.execute("UPDATE funding_delivery_hints SET queued=NULL,issue_bits=issue_bits|2 WHERE operation_id=?1",[operation])?;
                }
                return Err(error);
            }
        };
        // Delivery does not consume payment balances, checkpoints, readiness,
        // role, or claims. Only this wallet's and endpoint's revocation epochs.
        let endpoints = self
            .db
            .prepare("SELECT scope,generation FROM evidence_generations WHERE kind='endpoint'")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    u64::try_from(r.get::<_, i64>(1)?)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, -1))?,
                ))
            })?
            .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?;
        let mut capture = EvidenceCapture {
            namespace: self.evidence_namespace,
            wallet: wallet.clone(),
            generation: evidence::generation(&self.db, "wallet", &wallet)?,
            endpoint_generations: endpoints,
            capacity_snapshot: None,
            checkpoint: None,
            previous: None,
            previous_confirmed_covered: false,
            durable_balance_anchors: Vec::new(),
        };
        let anchors=self.db.prepare("SELECT p.block_height,p.block_hash FROM funding_delivery_events e JOIN funding_delivery_current c ON c.event_id=e.id AND c.operation_id=e.operation_id JOIN funding_delivery_proofs p ON p.id=c.proof_id WHERE e.operation_id=?1")?.query_map([operation],|r|Ok(crate::rotation::base::Anchor{height:r.get::<_,i64>(0)?.try_into().map_err(|_|rusqlite::Error::IntegralValueOutOfRange(0,0))?,hash:r.get(1)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        capture.durable_balance_anchors.extend(anchors);
        let ticket = match ticket {
            Some(ticket) => ticket,
            None => {
                let ticket = Uuid::new_v4().to_string();
                self.db.execute("INSERT INTO funding_delivery_hints(operation_id,job_id,quote_digest,active,ticket) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(operation_id) DO UPDATE SET active=excluded.active,ticket=excluded.ticket,queued=CASE WHEN ?6 THEN queued ELSE NULL END WHERE active IS NULL AND job_id=excluded.job_id AND quote_digest=excluded.quote_digest",params![operation,job,digest(&serde_json::to_vec(&quote)?).as_slice(),bytes,ticket,authoritative])?;
                ticket
            }
        };
        Ok(Some(DeliveryRead {
            namespace: self.evidence_namespace,
            job: job.into(),
            operation: operation.into(),
            ticket,
            binding,
            event_candidate,
            expected,
            capture,
        }))
    }
    pub(crate) fn finish_delivery_read(&mut self, read: &DeliveryRead) -> Result<()> {
        ensure!(
            read.namespace == self.evidence_namespace,
            "delivery observation namespace changed"
        );
        self.db.execute("UPDATE funding_delivery_hints SET queued=COALESCE(queued,active),active=NULL,ticket=NULL,prefer_event=?4 WHERE operation_id=?1 AND job_id=?2 AND ticket=?3",params![read.operation,read.job,read.ticket,!read.event_candidate])?;
        Ok(())
    }
    pub(crate) fn commit_funding_delivery(
        &mut self,
        read: &DeliveryRead,
        proof: &FundingDeliveryProof,
    ) -> Result<()> {
        let (binding, wallet, quote, source) = self.delivery_binding(&read.job, &read.operation)?;
        ensure!(
            read.namespace == self.evidence_namespace
                && binding == read.binding
                && proof.capture() == &read.capture
                && proof.matches_expectation(&read.expected)
                && proof.operation().to_string() == read.operation
                && proof.source_transaction() == source
                && proof.quote_digest() == &digest(&serde_json::to_vec(&quote)?)
                && proof.is_fresh(crate::rotation::base::now()?)?,
            "delivery publication binding changed"
        );
        ensure!(
            proof.capture().wallet == wallet
                && proof.capture().generation == evidence::generation(&self.db, "wallet", &wallet)?
                && proof.endpoint_generation()
                    == evidence::generation(&self.db, "endpoint", proof.endpoint())?,
            "delivery evidence generation changed"
        );
        let tx = self.db.transaction()?;
        let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_delivery_hints WHERE operation_id=?1 AND job_id=?2 AND ticket=?3)",params![read.operation,read.job,read.ticket],|r|r.get(0))?;
        ensure!(owned, "delivery observation ownership changed");
        tx.execute("INSERT INTO funding_delivery_events(operation_id,job_id,wallet_id,quote_digest,source_transaction,transaction_hash,log_index,recipient,amount) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(transaction_hash,log_index) DO NOTHING",params![read.operation,read.job,wallet,proof.quote_digest().as_slice(),source.to_string(),proof.transaction().to_string(),i64::try_from(proof.log_index())?,proof.recipient().to_string(),proof.amount().to_string()])?;
        let (event,owner):(i64,String)=tx.query_row("SELECT id,operation_id FROM funding_delivery_events WHERE transaction_hash=?1 AND log_index=?2",params![proof.transaction().to_string(),i64::try_from(proof.log_index())?],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(owner == read.operation, "delivery event already owned");
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM funding_delivery_events WHERE id=?1 AND job_id=?2 AND wallet_id=?3 AND quote_digest=?4 AND source_transaction=?5 AND recipient=?6 AND amount=?7)",params![event,read.job,wallet,proof.quote_digest().as_slice(),source.to_string(),proof.recipient().to_string(),proof.amount().to_string()],|r|r.get(0))?;
        ensure!(exact, "delivery event binding conflict");
        tx.execute("INSERT INTO funding_delivery_proofs(event_id,block_height,block_hash,endpoint,endpoint_generation) VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",params![event,i64::try_from(proof.block().height)?,proof.block().hash,proof.endpoint(),i64::try_from(proof.endpoint_generation())?])?;
        let id:i64=tx.query_row("SELECT id FROM funding_delivery_proofs WHERE event_id=?1 AND block_height=?2 AND block_hash=?3 AND endpoint=?4 AND endpoint_generation=?5",params![event,i64::try_from(proof.block().height)?,proof.block().hash,proof.endpoint(),i64::try_from(proof.endpoint_generation())?],|r|r.get(0))?;
        tx.execute(
            "DELETE FROM funding_delivery_disputes WHERE proof_id=?1",
            [id],
        )?;
        tx.execute("INSERT INTO funding_delivery_current(operation_id,event_id,proof_id) VALUES(?1,?2,?3) ON CONFLICT(operation_id) DO UPDATE SET event_id=excluded.event_id,proof_id=excluded.proof_id",params![read.operation,event,id])?;
        // Keep the verified candidate for restart re-observation; a queued newer
        // hint remains separate and cannot rewrite this immutable event.
        tx.execute("UPDATE funding_delivery_hints SET queued=COALESCE(queued,active),active=NULL,ticket=NULL,prefer_event=1 WHERE operation_id=?1 AND ticket=?2",params![read.operation,read.ticket])?;
        ensure!(
            proof.is_fresh(crate::rotation::base::now()?)?,
            "delivery proof expired before publication"
        );
        tx.commit()?;
        Ok(())
    }
}
/// Only delivery is discharged. Source/refund/readiness predicates remain separate.
pub(super) fn delivered(db: &Connection, job: &str, operation: &str) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM funding_delivery_events e JOIN funding_delivery_current c ON c.event_id=e.id AND c.operation_id=e.operation_id JOIN funding_delivery_proofs p ON p.id=c.proof_id AND p.event_id=e.id JOIN funding_jobs f ON f.id=e.job_id AND f.wallet_id=e.wallet_id JOIN funding_progress j ON j.job_id=f.id AND j.operation_id=e.operation_id JOIN wallets w ON w.id=e.wallet_id WHERE e.job_id=?1 AND e.operation_id=?2 AND j.accepted_output=e.amount AND lower(j.accepted_recipient)=lower(e.recipient) AND lower(w.address)=lower(e.recipient) AND p.endpoint_generation=COALESCE((SELECT generation FROM evidence_generations WHERE kind='endpoint' AND scope=p.endpoint),0) AND NOT EXISTS(SELECT 1 FROM funding_delivery_disputes d WHERE d.proof_id=p.id))",params![job,operation],|r|r.get(0))?)
}
pub(super) fn dispute_endpoint(db: &Connection, endpoint: &str) -> Result<()> {
    db.execute("INSERT INTO funding_delivery_disputes SELECT p.id,'endpoint_reverification_required' FROM funding_delivery_proofs p JOIN funding_delivery_current c ON c.proof_id=p.id WHERE p.endpoint=?1 ON CONFLICT(proof_id) DO NOTHING",[endpoint])?;
    Ok(())
}
pub(super) fn dispute_anchor(db: &Connection, wallet: &str, hash: &str) -> Result<()> {
    db.execute("INSERT INTO funding_delivery_disputes SELECT p.id,'chain_repair_required' FROM funding_delivery_events e JOIN funding_delivery_current c ON c.event_id=e.id AND c.operation_id=e.operation_id JOIN funding_delivery_proofs p ON p.id=c.proof_id WHERE e.wallet_id=?1 AND p.block_hash=?2 ON CONFLICT(proof_id) DO UPDATE SET reason='chain_repair_required' ",params![wallet,hash])?;
    Ok(())
}

/// Correlation is finite provider evidence, not proof that future returns cannot
/// occur. Every newly discovered canonical output invalidates this accounting.
fn return_correlation(
    source: alloy_primitives::B256,
    destination: alloy_primitives::B256,
    quote: &[u8],
) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"funding-return-v1");
    hash.update(source.as_slice());
    hash.update(destination.as_slice());
    hash.update(quote);
    hash.finalize().into()
}
fn return_complete(db: &Connection, operation: &str) -> Result<bool> {
    let claim:Option<(Option<String>,Option<Vec<u8>>,bool)>=db.query_row("SELECT declared_return,return_binding,return_conflict FROM funding_delivery_hints WHERE operation_id=?1",[operation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((Some(amount), Some(binding), false)) = claim else {
        return Ok(false);
    };
    let amount = U256::from_str_radix(&amount, 10).context("invalid retained return amount")?;
    let event:Option<(String,String,Vec<u8>)>=db.query_row("SELECT e.source_transaction,e.transaction_hash,e.quote_digest FROM funding_delivery_current c JOIN funding_delivery_events e ON e.id=c.event_id AND e.operation_id=c.operation_id WHERE c.operation_id=?1",[operation],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((source, destination, quote)) = event else {
        return Ok(false);
    };
    let exact = return_correlation(source.parse()?, destination.parse()?, &quote);
    if binding.as_slice() != exact.as_slice() {
        return Ok(false);
    }
    let Some(returned) = super::source_support::returned(db, operation)? else {
        return Ok(false);
    };
    // Explicit zero closes only the currently known finite obligation. A newly
    // observed positive output contradicts zero and reopens the operation.
    Ok(if amount == U256::ZERO {
        returned == U256::ZERO
    } else {
        returned >= amount
    })
}
pub(super) fn return_pending(db: &Connection, operation: &str) -> Result<bool> {
    let (claimed,external):(bool,bool)=db.query_row("SELECT EXISTS(SELECT 1 FROM funding_delivery_hints WHERE operation_id=?1 AND (declared_return IS NOT NULL OR return_conflict=1)),EXISTS(SELECT 1 FROM funding_jobs f JOIN funding_progress j ON j.job_id=f.id WHERE j.operation_id=?1 AND f.state='COMPLETE' AND f.completion_outcome IN ('EXTERNAL','RECOVERED'))",[operation],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if external && funding::source_attempt_closed(db, operation)? {
        return Ok(false);
    }
    let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 25 {
        return Ok(claimed);
    }
    let required:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM funding_progress WHERE operation_id=?1 AND (provider_success=1 OR provider_refunded=1)) OR EXISTS(SELECT 1 FROM refund_outputs WHERE operation_id=?1) OR EXISTS(SELECT 1 FROM funding_delivery_current WHERE operation_id=?1)",[operation],|row|row.get(0))?;
    Ok((claimed || required) && !return_complete(db, operation)?)
}
#[cfg(all(test, feature = "zcash"))]
mod tests;
