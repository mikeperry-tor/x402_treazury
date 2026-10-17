//! Canonical delivery evidence. Provider correlation remains an explicit input;
//! this proof establishes the transfer, not bridge attribution or source closure.
use super::*;
#[cfg(feature = "zcash")]
use crate::rotation::near::{Quote, StatusObservation};
#[cfg(feature = "zcash")]
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq)]
pub struct FundingDeliveryExpectation {
    operation: uuid::Uuid,
    quote_digest: [u8; 32],
    source_transaction: B256,
    transaction: B256,
    recipient: Address,
    amount: U256,
}
#[cfg(feature = "zcash")]
impl FundingDeliveryExpectation {
    pub fn from_observation(
        operation: uuid::Uuid,
        quote: &Quote,
        observation: &StatusObservation,
        source_transaction: B256,
    ) -> Result<Self> {
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(quote)?).into();
        ensure!(
            !operation.is_nil() && observation.quote_digest() == &digest,
            "funding delivery quote binding mismatch"
        );
        let candidate = observation
            .settlement()
            .context("funding delivery correlation incomplete")?;
        ensure!(
            candidate.origin_transactions.as_slice() == [source_transaction]
                && candidate.destination_transactions.len() == 1,
            "funding delivery correlation ambiguous"
        );
        let recipient = quote.request["recipient"]
            .as_str()
            .context("funding recipient missing")?
            .parse::<Address>()
            .map_err(|_| anyhow::anyhow!("funding recipient malformed"))?;
        let amount = quote.request["amount"]
            .as_str()
            .context("funding amount missing")?;
        ensure!(
            !amount.is_empty() && amount.bytes().all(|b| b.is_ascii_digit()),
            "funding amount malformed"
        );
        let amount = U256::from_str_radix(amount, 10)
            .map_err(|_| anyhow::anyhow!("funding amount malformed"))?;
        ensure!(
            amount > U256::ZERO
                && quote.response["quote"]["amountOut"]
                    .as_str()
                    .and_then(|v| U256::from_str_radix(v, 10).ok())
                    == Some(amount),
            "funding amount mismatch"
        );
        ensure!(
            candidate.amount_out.is_none_or(|actual| actual == amount),
            "funding delivered amount contradicts quote"
        );
        Ok(Self {
            operation,
            quote_digest: digest,
            source_transaction,
            transaction: candidate.destination_transactions[0],
            recipient,
            amount,
        })
    }
}

/// Opaque, process-local proof. Publication must CAS the original capture and
/// operation binding and enforce unique (transaction, log_index) ownership.
#[derive(Clone)]
pub struct FundingDeliveryProof {
    expected: FundingDeliveryExpectation,
    capture: EvidenceCapture,
    endpoint: String,
    endpoint_generation: u64,
    block: Anchor,
    log_index: u64,
    valid_until: u64,
    freshness: crate::freshness::FreshnessLease,
}
impl FundingDeliveryProof {
    pub(crate) fn matches_expectation(&self, expected: &FundingDeliveryExpectation) -> bool {
        &self.expected == expected
    }
    pub fn operation(&self) -> uuid::Uuid {
        self.expected.operation
    }
    pub fn quote_digest(&self) -> &[u8; 32] {
        &self.expected.quote_digest
    }
    pub fn source_transaction(&self) -> B256 {
        self.expected.source_transaction
    }
    pub fn transaction(&self) -> B256 {
        self.expected.transaction
    }
    pub fn recipient(&self) -> Address {
        self.expected.recipient
    }
    pub fn amount(&self) -> U256 {
        self.expected.amount
    }
    pub fn capture(&self) -> &EvidenceCapture {
        &self.capture
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
    pub fn block(&self) -> &Anchor {
        &self.block
    }
    pub fn log_index(&self) -> u64 {
        self.log_index
    }
    pub fn is_fresh(&self, now: u64) -> Result<bool> {
        self.freshness.is_fresh(now, self.valid_until)
    }
}
impl BaseRpc {
    pub async fn verify_funding_delivery(
        &self,
        expected: &FundingDeliveryExpectation,
        capture: EvidenceCapture,
    ) -> Result<Option<FundingDeliveryProof>> {
        let historical = self.classified(resources::Class::Historical);
        let _owner = resources::query(resources::Class::Historical)?;
        ensure!(
            !capture.namespace.is_nil() && !capture.wallet.is_empty(),
            evidence::EvidenceFailure::ScopeMismatch
        );
        for (index, endpoint) in std::iter::once(&historical)
            .chain(&historical.fallbacks)
            .enumerate()
        {
            let endpoint = endpoint.for_address(&expected.recipient.to_string())?;
            match endpoint.delivery_inner(expected, capture.clone()).await {
                Ok(proof) => return Ok(proof),
                Err(error) => {
                    let failover = index < self.fallbacks.len()
                        && error
                            .downcast_ref::<RpcFailure>()
                            .is_some_and(RpcFailure::can_failover);
                    if !failover {
                        return Err(error.context(VerificationStage("funding delivery")));
                    }
                }
            }
        }
        unreachable!("primary RPC always exists")
    }
    async fn recheck_saved_delivery(&self, capture: &EvidenceCapture) -> Result<()> {
        for anchor in &capture.durable_balance_anchors {
            let (current, _) = self.block(&format!("0x{:x}", anchor.height)).await?;
            ensure!(
                current.height == anchor.height,
                "receipt block height mismatch"
            );
            ensure!(
                current.hash == anchor.hash,
                EvidenceConflict {
                    wallet: capture.wallet.clone(),
                    endpoint: self.endpoint_id(),
                    anchor: anchor.clone(),
                    latest_only: false,
                    financial_checkpoint: false,
                }
            );
        }
        Ok(())
    }
    async fn delivery_inner(
        &self,
        expected: &FundingDeliveryExpectation,
        capture: EvidenceCapture,
    ) -> Result<Option<FundingDeliveryProof>> {
        let origin = crate::freshness::ObservationClock::capture()?;
        ensure!(
            quantity(&self.rpc("eth_chainId", json!([])).await?)? == 8453,
            EndpointMismatch {
                endpoint: self.endpoint_id()
            }
        );
        let (latest, stamp) = self.block("latest").await?;
        validate_timestamp(stamp, now()?, self.max_age)?;
        let valid_until = stamp
            .checked_add(self.max_age)
            .context("funding freshness overflow")?;
        let freshness = crate::freshness::FreshnessLease::from_observation(origin, valid_until);
        let receipt = self
            .rpc(
                "eth_getTransactionReceipt",
                json!([expected.transaction.to_string()]),
            )
            .await?;
        if receipt.is_null() {
            self.recheck_saved_delivery(&capture).await?;
            return Ok(None);
        }
        let height = quantity(&receipt["blockNumber"])?;
        if latest
            .height
            .checked_sub(self.confirmations)
            .is_none_or(|confirmed| height > confirmed)
        {
            self.recheck_saved_delivery(&capture).await?;
            return Ok(None);
        }
        let (block, _) = self.block(&format!("0x{height:x}")).await?;
        ensure!(block.height == height, "receipt block height mismatch");
        let log_index = match validate_delivery(&receipt, expected, &block) {
            Ok(index) => index,
            Err(error) => {
                // A now-reverted or changed receipt must not hide a canonical
                // contradiction of previously accepted support.
                self.recheck_saved_delivery(&capture).await?;
                return Err(error);
            }
        };
        for pinned in [&block, &latest] {
            let (current, _) = self.block(&format!("0x{:x}", pinned.height)).await?;
            ensure!(
                current.height == pinned.height,
                "receipt block height mismatch"
            );
            if current.hash != pinned.hash {
                let financial_checkpoint = capture.checkpoint.as_ref() == Some(pinned);
                if financial_checkpoint || capture.durable_balance_anchors.contains(pinned) {
                    return Err(EvidenceConflict {
                        wallet: capture.wallet.clone(),
                        endpoint: self.endpoint_id(),
                        anchor: pinned.clone(),
                        latest_only: false,
                        financial_checkpoint,
                    }
                    .into());
                }
                // A candidate changing mid-request is not a contradiction of
                // previously published wallet evidence without that dependency.
                anyhow::bail!("Base view changed during reconciliation");
            }
        }
        ensure!(freshness.is_fresh(now()?, valid_until)?, "stale Base block");
        let endpoint = self.endpoint_id();
        let endpoint_generation = capture
            .endpoint_generations
            .get(&endpoint)
            .copied()
            .unwrap_or(0);
        Ok(Some(FundingDeliveryProof {
            expected: expected.clone(),
            capture,
            endpoint,
            endpoint_generation,
            block,
            log_index,
            valid_until,
            freshness,
        }))
    }
}
fn hash(value: &Value) -> Result<B256> {
    value
        .as_str()
        .context("delivery hash missing")?
        .parse()
        .map_err(|_| anyhow::anyhow!("delivery hash malformed"))
}
fn validate_delivery(
    receipt: &Value,
    expected: &FundingDeliveryExpectation,
    block: &Anchor,
) -> Result<u64> {
    ensure!(
        quantity(&receipt["status"])? == 1,
        "receipt transaction reverted"
    );
    let block_hash: B256 = block.hash.parse()?;
    ensure!(
        hash(&receipt["transactionHash"])? == expected.transaction
            && hash(&receipt["blockHash"])? == block_hash
            && quantity(&receipt["blockNumber"])? == block.height,
        "receipt transaction/block mismatch"
    );
    let logs = receipt["logs"].as_array().context("receipt logs missing")?;
    ensure!(logs.len() <= 4096, "receipt log limit exceeded");
    let token: Address = USDC.parse()?;
    let transfer = keccak256("Transfer(address,address,uint256)");
    let recipient = B256::left_padding_from(expected.recipient.as_slice());
    let mut found = None;
    for log in logs {
        let address: Address = log["address"]
            .as_str()
            .context("delivery token missing")?
            .parse()
            .map_err(|_| anyhow::anyhow!("delivery token malformed"))?;
        if address != token {
            continue;
        }
        ensure!(
            log["removed"] == false
                && hash(&log["transactionHash"])? == expected.transaction
                && hash(&log["blockHash"])? == block_hash
                && quantity(&log["blockNumber"])? == block.height,
            "receipt log provenance mismatch"
        );
        let topics = log["topics"]
            .as_array()
            .context("receipt log topics missing")?;
        if topics.first().map(hash).transpose()? != Some(transfer) {
            continue;
        }
        ensure!(topics.len() == 3, "receipt event topics malformed");
        ensure!(
            hash(&topics[1])?.as_slice()[..12] == [0; 12],
            "delivery sender malformed"
        );
        if hash(&topics[2])? != recipient {
            continue;
        }
        // More than one recipient transfer is ambiguous, even if only one has the desired amount.
        ensure!(
            found.is_none() && U256::from_be_bytes(hash(&log["data"])?.0) == expected.amount,
            "funding delivery lacks unique exact transfer"
        );
        found = Some(quantity(&log["logIndex"])?);
    }
    found.context("funding delivery lacks unique exact transfer")
}
#[cfg(test)]
mod tests;
