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
