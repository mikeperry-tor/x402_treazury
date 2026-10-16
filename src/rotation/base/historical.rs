//! Reverify immutable destination support after endpoint trust invalidation.
//! Historical balances may already have been spent; this creates no fresh
//! admission observation and never renews the original observation lifetime.
use super::*;

#[derive(Clone)]
pub(crate) struct HistoricalBalanceRequest {
    pub capture: EvidenceCapture,
    pub observation: evidence::BalanceObservation,
    pub checkpoint: Option<Anchor>,
}

pub(crate) struct HistoricalBalanceCertificate {
    request: HistoricalBalanceRequest,
    endpoint_generation: u64,
}
impl HistoricalBalanceCertificate {
    pub(crate) fn request(&self) -> &HistoricalBalanceRequest {
        &self.request
    }
    pub(crate) fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
}

#[derive(Debug)]
pub(crate) struct HistoricalAmountConflict {
    pub wallet: String,
    pub anchor: Anchor,
}
impl std::fmt::Display for HistoricalAmountConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("historical_balance_amount_contradiction")
    }
}
impl std::error::Error for HistoricalAmountConflict {}

impl BaseRpc {
    pub(crate) async fn verify_historical_balance(
        &self,
        request: HistoricalBalanceRequest,
    ) -> Result<HistoricalBalanceCertificate> {
        let _owner = resources::query()?;
        request.observation.validate()?;
        ensure!(
            request.capture.wallet == request.observation.wallet,
            evidence::EvidenceFailure::ScopeMismatch
        );
        let observation = &request.observation;
        let endpoint = self
            .original_endpoint(&observation.endpoint)?
            .for_address(&observation.address)?;
        let mut capture = request.capture.clone();
        capture.previous = None;
        capture.durable_balance_anchors.clear();
        let mut rechecked = endpoint
            .verify_recheck(&capture, &observation.endpoint)
            .await?;
        let mut anchors = vec![
            observation.confirmed.anchor.clone(),
            observation.latest.anchor.clone(),
        ];
        if let Some(checkpoint) = &request.checkpoint {
            anchors.push(checkpoint.clone());
        }
        anchors.sort_by(|a, b| (a.height, &a.hash).cmp(&(b.height, &b.hash)));
        anchors.dedup();
        let address: Address = observation.address.parse()?;
        let data = format!("0x70a08231{:0>64}", format!("{address:x}"));
        for pass in 0..2 {
            for anchor in &anchors {
                let canonical = endpoint
                    .recheck_anchor(
                        anchor.height,
                        &mut rechecked,
                        &ViewProgress::new(),
                        "historical_anchor_recheck",
                    )
                    .await?;
                ensure!(
                    canonical == *anchor,
                    EvidenceConflict {
                        wallet: capture.wallet.clone(),
                        endpoint: observation.endpoint.clone(),
                        anchor: anchor.clone(),
                        latest_only: *anchor == observation.latest.anchor
                            && *anchor != observation.confirmed.anchor,
                        financial_checkpoint: capture.checkpoint.as_ref() == Some(anchor),
                    }
                );
            }
            if pass == 0 {
                for point in [&observation.confirmed, &observation.latest] {
                    let amount = endpoint.call(data.clone(), &point.anchor).await?;
                    ensure!(
                        amount == point.amount,
                        HistoricalAmountConflict {
                            wallet: capture.wallet.clone(),
                            anchor: point.anchor.clone()
                        }
                    );
                }
                // State reads separate canonical verification phases: a header
                // checked before them cannot certify their post-read provenance.
                rechecked.clear();
            }
        }
        let endpoint_generation = request
            .capture
            .endpoint_generations
            .get(&observation.endpoint)
            .copied()
            .unwrap_or(0);
        Ok(HistoricalBalanceCertificate {
            request,
            endpoint_generation,
        })
    }
}

/// Rechecking an original nonce observation grants no new release or spending authority.
#[derive(Clone)]
pub(crate) struct HistoricalResolutionRequest {
    pub capture: EvidenceCapture,
    pub authorization: PendingAuthorization,
    pub endpoint: String,
    pub anchor: Anchor,
    pub outcome: AuthorizationOutcome,
    pub block_time: u64,
    pub checkpoint: Option<Anchor>,
}
impl BaseRpc {
    pub(crate) async fn verify_historical_resolution(
        &self,
        request: HistoricalResolutionRequest,
    ) -> Result<()> {
        let _owner = resources::query()?;
        ensure!(
            request.capture.wallet == request.authorization.wallet,
            evidence::EvidenceFailure::ScopeMismatch
        );
        let endpoint = self
            .original_endpoint(&request.endpoint)?
            .for_address(&request.authorization.payer)?;
        let mut capture = request.capture;
        capture.previous = None;
        capture.durable_balance_anchors = request.checkpoint.into_iter().collect();
        endpoint.verify_recheck(&capture, &request.endpoint).await?;
        let tag = format!("0x{:x}", request.anchor.height);
        let expected = (request.anchor.clone(), request.block_time);
        ensure!(
            endpoint.block(&tag).await? == expected,
            evidence::EvidenceFailure::ResolutionConflict
        );
        let payer: Address = request.authorization.payer.parse()?;
        let nonce: B256 = request.authorization.nonce.parse()?;
        let selector =
            alloy_primitives::hex::encode(&keccak256("authorizationState(address,bytes32)")[..4]);
        let data = format!(
            "0x{selector}{:0>64}{}",
            format!("{payer:x}"),
            alloy_primitives::hex::encode(nonce)
        );
        let used = endpoint.call(data, &request.anchor).await?;
        let matches = match request.outcome {
            AuthorizationOutcome::Used => used == U256::from(1),
            AuthorizationOutcome::ExpiredUnused => {
                used == U256::ZERO && request.block_time > request.authorization.valid_before
            }
        };
        ensure!(matches, evidence::EvidenceFailure::ResolutionConflict);
        ensure!(
            endpoint.block(&tag).await? == expected,
            evidence::EvidenceFailure::ResolutionConflict
        );
        endpoint.verify_recheck(&capture, &request.endpoint).await?;
        Ok(())
    }
}
