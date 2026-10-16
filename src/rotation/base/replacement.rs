//! Current canonical nonce evidence for an atomic wallet repair. This API does
//! not publish releases; the store must adopt a complete, revision-checked pass.
use super::*;

/// A pass stays on one configured endpoint. A failed page never switches the
/// provenance of already staged proofs; a new pass may select a fallback.
pub(crate) struct ReplacementPass {
    id: uuid::Uuid,
    capture: EvidenceCapture,
    payer: String,
    anchor: Anchor,
    block_time: u64,
    debit_floor: Option<u64>,
    endpoint: BaseRpc,
}

pub(crate) struct ReplacementPage {
    pass: uuid::Uuid,
    resolutions: Vec<ResolutionCertificate>,
    unresolved: Vec<PendingAuthorization>,
}

pub(crate) struct ReplacementBalance {
    pass: uuid::Uuid,
    view: ChainView,
}

impl ReplacementPass {
    pub(crate) fn id(&self) -> uuid::Uuid {
        self.id
    }
    pub(crate) fn capture(&self) -> &EvidenceCapture {
        &self.capture
    }
    pub(crate) fn anchor(&self) -> &Anchor {
        &self.anchor
    }
    pub(crate) fn block_time(&self) -> u64 {
        self.block_time
    }
    pub(crate) fn endpoint(&self) -> String {
        self.endpoint.endpoint_id()
    }
    fn current_capture(&self) -> EvidenceCapture {
        let mut capture = self.capture.clone();
        capture.capacity_snapshot = None;
        capture.checkpoint = Some(self.anchor.clone());
        capture.previous = None;
        capture.previous_confirmed_covered = false;
        capture.durable_balance_anchors.clear();
        capture
    }
}

impl ReplacementPage {
    pub(crate) fn pass(&self) -> uuid::Uuid {
        self.pass
    }
    pub(crate) fn resolutions(&self) -> &[ResolutionCertificate] {
        &self.resolutions
    }
    pub(crate) fn unresolved(&self) -> &[PendingAuthorization] {
        &self.unresolved
    }
}

impl ReplacementBalance {
    pub(crate) fn pass(&self) -> uuid::Uuid {
        self.pass
    }
    pub(crate) fn view(&self) -> &ChainView {
        &self.view
    }
}

impl BaseRpc {
    pub(crate) async fn begin_replacement(
        &self,
        capture: EvidenceCapture,
        payer: String,
        debit_floor: Option<u64>,
    ) -> Result<ReplacementPass> {
        let historical = self.classified(resources::Class::Historical);
        let _owner = resources::query(resources::Class::Historical)?;
        ensure!(
            !capture.namespace.is_nil() && !capture.wallet.is_empty(),
            evidence::EvidenceFailure::ScopeMismatch
        );
        let payer = payer.parse::<Address>()?.to_string();
        // Starting headers are current and confirmed, but need not stay fresh
        // while an arbitrarily long paginated nonce scan makes progress.
        let query = ChainQuery {
            wallets: vec![],
            pending: vec![],
            anchor: None,
            captures: BTreeMap::new(),
        };
        for (index, endpoint) in std::iter::once(&historical)
            .chain(&historical.fallbacks)
            .enumerate()
        {
            let endpoint = endpoint.for_address(&payer)?;
            let result = async {
                let headers = endpoint
                    .initial_view_headers(&query, &ViewProgress::new())
                    .await?;
                let pass = ReplacementPass {
                    id: uuid::Uuid::new_v4(),
                    capture: capture.clone(),
                    payer: payer.clone(),
                    anchor: headers.confirmed,
                    block_time: headers.confirmed_time,
                    debit_floor,
                    endpoint,
                };
                pass.endpoint.check_replacement_anchor(&pass).await?;
                Ok::<_, anyhow::Error>(pass)
            }
            .await;
            match result {
                Ok(pass) => return Ok(pass),
                Err(error)
                    if index < self.fallbacks.len()
                        && error
                            .downcast_ref::<RpcFailure>()
                            .is_some_and(RpcFailure::can_failover) => {}
                Err(error) => return Err(error),
            }
        }
        unreachable!("primary Base endpoint exists")
    }

    async fn check_replacement_anchor(&self, pass: &ReplacementPass) -> Result<()> {
        ensure!(
            quantity(&self.rpc("eth_chainId", json!([])).await?)
                .map_err(|_| RpcFailure::malformed("eth_chainId"))?
                == 8453,
            EndpointMismatch {
                endpoint: self.endpoint_id()
            }
        );
        let (anchor, time) = self.block(&format!("0x{:x}", pass.anchor.height)).await?;
        ensure!(
            anchor == pass.anchor && time == pass.block_time,
            evidence::EvidenceFailure::ResolutionConflict
        );
        Ok(())
    }

    pub(crate) async fn replacement_page(
        &self,
        pass: &ReplacementPass,
        authorizations: Vec<PendingAuthorization>,
    ) -> Result<ReplacementPage> {
        let _owner = resources::query(resources::Class::Historical)?;
        ensure!(
            !authorizations.is_empty()
                && authorizations.len() <= crate::rotation::store::history::PAGE_SIZE,
            "replacement page exceeds resource bound"
        );
        // Validate the whole page before any request. Caller-supplied fields
        // cannot choose another wallet or smuggle a repeated authorization.
        let mut ids = std::collections::BTreeSet::new();
        let mut nonces = std::collections::BTreeSet::new();
        for auth in &authorizations {
            ensure!(
                auth.wallet == pass.capture.wallet
                    && auth.payer.parse::<Address>()? == pass.payer.parse::<Address>()?
                    && !auth.id.is_empty()
                    && ids.insert(&auth.id)
                    && nonces.insert(auth.nonce.parse::<B256>()?),
                evidence::EvidenceFailure::ScopeMismatch
            );
        }
        // Only the pass's chosen endpoint supplies evidence, even if `self`
        // has healthy fallbacks. Failed passes remain unadopted at the store.
        let endpoint = &pass.endpoint;
        endpoint.check_replacement_anchor(pass).await?;
        let capture = pass.current_capture();
        let selector =
            alloy_primitives::hex::encode(&keccak256("authorizationState(address,bytes32)")[..4]);
        let mut resolutions = Vec::with_capacity(authorizations.len());
        let mut unresolved = Vec::new();
        for auth in authorizations {
            let nonce: B256 = auth.nonce.parse()?;
            let payer: Address = auth.payer.parse()?;
            let data = format!(
                "{selector}{:0>64}{}",
                format!("{payer:x}"),
                alloy_primitives::hex::encode(nonce)
            );
            let used = endpoint.call(format!("0x{data}"), &pass.anchor).await?;
            ensure!(used <= U256::from(1), RpcFailure::malformed("eth_call"));
            let outcome = if used == U256::from(1) {
                Some(AuthorizationOutcome::Used)
            } else if pass.block_time > auth.valid_before {
                Some(AuthorizationOutcome::ExpiredUnused)
            } else {
                None
            };
            if let Some(outcome) = outcome {
                resolutions.push(ResolutionCertificate::verified(
                    capture.clone(),
                    endpoint.endpoint_id(),
                    auth,
                    pass.anchor.clone(),
                    outcome,
                    pass.block_time,
                ));
            } else {
                unresolved.push(auth);
            }
        }
        endpoint.check_replacement_anchor(pass).await?;
        Ok(ReplacementPage {
            pass: pass.id,
            resolutions,
            unresolved,
        })
    }

    /// Acquire balances last. Freshness comes from these block observations,
    /// never the pass start, page completion, or a renewed nonce-proof lifetime.
    pub(crate) async fn finish_replacement(
        &self,
        pass: &ReplacementPass,
    ) -> Result<ReplacementBalance> {
        let _owner = resources::query(resources::Class::Historical)?;
        let endpoint = &pass.endpoint;
        endpoint.check_replacement_anchor(pass).await?;
        let view = endpoint
            .view_inner(
                ChainQuery {
                    wallets: vec![(pass.capture.wallet.clone(), pass.payer.clone())],
                    pending: vec![],
                    anchor: Some(pass.anchor.clone()),
                    captures: BTreeMap::from([(
                        pass.capture.wallet.clone(),
                        pass.current_capture(),
                    )]),
                },
                &ViewProgress::new(),
            )
            .await?;
        let observation = &view.observations[&pass.capture.wallet];
        ensure!(
            pass.debit_floor
                .is_none_or(|floor| observation.confirmed.anchor.height >= floor),
            evidence::EvidenceFailure::BeforeDebit
        );
        endpoint.check_replacement_anchor(pass).await?;
        Ok(ReplacementBalance {
            pass: pass.id,
            view,
        })
    }
}

#[cfg(test)]
mod tests;
