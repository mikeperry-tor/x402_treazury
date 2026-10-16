//! Shared pool ownership and atomic signed admission after a valid challenge.
use super::error::AdmissionError;
use super::{
    base::{BaseRpc, now},
    store::{Authorization, StoreHandle},
};
use crate::payment::{SpendPolicy, USDC};
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use x402_chain_eip155::V2Eip155ExactClient;
use x402_types::scheme::client::{PaymentCandidate, PaymentSelector, X402SchemeClient};

pub(crate) const OMITTED_EXTENSIONS: &str = "Managed payment omitted advertised x402 extensions; the provider may require them. A submitted authorization may still settle and remains reserved until chain reconciliation. Do not automatically retry";

#[derive(Clone, Debug)]
pub struct PaymentCandidateHandle {
    pub wallet: String,
    pub address: String,
}
#[derive(Debug)]
pub(crate) struct PayerHandoff(pub PaymentCandidateHandle);
impl std::fmt::Display for PayerHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unsigned payer handoff required")
    }
}
impl std::error::Error for PayerHandoff {}
/// Direct Rust callers retain this value while consuming the response body.
/// Mark its cover lease complete only after successful output consumption.
pub struct ManagedResponse {
    pub response: reqwest::Response,
    pub extensions_omitted: bool,
    pub cover: Option<crate::cover::runtime::Call>,
}
pub(crate) struct AdmissionObservation<'a> {
    pub fixed_payer: bool,
    pub progress: &'a crate::payment::SubmissionProgress,
    pub recovery: &'a mut RecoveryAllowance,
    pub hint: Option<(
        crate::pricing::runtime::OperationKey,
        crate::pricing::runtime::Receipt,
    )>,
}
#[cfg(test)]
mod admission_tests;
mod maintenance;
mod selection;

#[cfg(test)]
#[derive(Clone)]
enum AdmissionTestStage {
    CandidateScan,
    CandidateEvaluated,
    CandidateReadCompleted { wallet: String },
    CandidateWaiting,
    Acquired,
    Journaled,
    Signed { nonce: String, valid_before: u64 },
}
#[cfg(test)]
type AdmissionTestHook =
    Arc<dyn Fn(AdmissionTestStage) -> futures_util::future::BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct ManagedPool {
    direct_work: crate::server::work::Work,
    store: StoreHandle,
    pool: String,
    base: BaseRpc,
    policy: SpendPolicy,
    gate: Arc<Mutex<()>>,
    reconciliation: Arc<Mutex<()>>,
    #[cfg(test)]
    test_admission_hook: Option<AdmissionTestHook>,
}
#[derive(Default)]
pub(crate) struct RecoveryAllowance {
    evidence_used: bool,
    signature_replacement_used: bool,
}
impl RecoveryAllowance {
    fn take_evidence(&mut self) -> bool {
        !std::mem::replace(&mut self.evidence_used, true)
    }
    fn take_signature_replacement(&mut self) -> bool {
        !std::mem::replace(&mut self.signature_replacement_used, true)
    }
}

struct Offer {
    raw: Value,
    amount: U256,
    payee: Address,
}
impl PaymentSelector for Offer {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.iter().find(|c| {
            c.x402_version == 2
                && c.scheme == "exact"
                && c.chain_id.to_string() == "eip155:8453"
                && c.asset.eq_ignore_ascii_case(USDC)
                && c.amount == self.amount
                && c.pay_to.parse::<Address>().ok() == Some(self.payee)
        })
    }
}
impl ManagedPool {
    pub fn new(
        store: StoreHandle,
        pool: String,
        base: BaseRpc,
        policy: SpendPolicy,
    ) -> Result<Self> {
        Ok(Self {
            direct_work: Default::default(),
            gate: store.group_gate(&pool)?,
            reconciliation: store.reconciliation_gate(&pool)?,
            #[cfg(test)]
            test_admission_hook: None,
            store,
            pool,
            base,
            policy,
        })
    }
    #[cfg(test)]
    async fn admission_test_stage(&self, stage: AdmissionTestStage) {
        if let Some(hook) = &self.test_admission_hook {
            hook(stage).await;
        }
    }
    /// Runtime-only group scope for estimates, never payment authority.
    pub(crate) fn hint_group_scope(&self) -> (uuid::Uuid, &str) {
        (self.store.runtime_namespace(), &self.pool)
    }
    async fn acquire_view(
        &self,
        query: super::base::ChainQuery,
        foreground: bool,
    ) -> Result<super::base::ChainView> {
        // Foreground reads belong to this call; background reads can share an owner.
        let result = if foreground {
            self.base.view(query).await
        } else {
            self.base
                .shared_balance_view(query, self.store.clone())
                .await
        };
        match result {
            Ok(view) => Ok(view),
            Err(error) => {
                let guard = Arc::new(self.gate.clone().lock_owned().await);
                self.store
                    .call_with_guard(guard, move |store| {
                        store.record_base_failure(&error)?;
                        Err(error)
                    })
                    .await
            }
        }
    }
    pub(crate) async fn documentation_address(&self) -> Result<String> {
        let pool = self.pool.clone();
        self.store
            .call(move |s| s.documentation_address(&pool))
            .await
    }

    pub async fn candidate(&self) -> Result<PaymentCandidateHandle> {
        self.candidate_for(None).await
    }
    pub(crate) async fn candidate_for(&self, hint: Option<U256>) -> Result<PaymentCandidateHandle> {
        let pool = self.pool.clone();
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        self.store
            .call_with_guard(guard, move |s| {
                if let Some(cost) = hint {
                    match s.cached_payment_candidate(&pool, cost, None) {
                        Ok(Some(candidate)) => return Ok(candidate),
                        Ok(None) => {}
                        // This is only an unsigned identity hint. A wallet-local
                        // cache rejection must not prevent challenge-time traversal
                        // from acquiring an independent payer's uncached evidence.
                        Err(error) if error.is::<super::base::evidence::EvidenceFailure>() => {}
                        Err(error) => return Err(error),
                    }
                }
                s.payment_candidate(&pool)
            })
            .await
    }
    async fn acquire_fixed_payer(
        &self,
        expected: &PaymentCandidateHandle,
    ) -> Result<(
        super::base::ChainView,
        Arc<tokio::sync::OwnedMutexGuard<()>>,
    )> {
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let (pool, wallet) = (self.pool.clone(), expected.wallet.clone());
        if let Some(view) = self
            .store
            .call_with_guard(guard.clone(), move |s| {
                s.cached_payment_view(&pool, &wallet)
            })
            .await?
        {
            return Ok((view, guard));
        }
        let (pool, wallet) = (self.pool.clone(), expected.wallet.clone());
        let query = self
            .store
            .call_with_guard(guard.clone(), move |s| {
                s.scoped_chain_query(&pool, vec![wallet], vec![])
            })
            .await?;
        drop(guard);
        let view = self.acquire_view(query, true).await?;
        Ok((view, Arc::new(self.gate.clone().lock_owned().await)))
    }
    pub async fn pay(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        retry: reqwest::Request,
        response: reqwest::Response,
    ) -> Result<(reqwest::Response, bool)> {
        let owned = self
            .pay_with_cover(http, expected, retry, response, None)
            .await?;
        Ok((owned.response, owned.extensions_omitted))
    }
    pub async fn pay_with_cover(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        retry: reqwest::Request,
        response: reqwest::Response,
        mut cover: Option<crate::cover::runtime::Call>,
    ) -> Result<ManagedResponse> {
        let pool = self.clone();
        let http = http.clone();
        crate::payment::ownership::run(&self.direct_work, async move {
            let mut recovery = RecoveryAllowance::default();
            let (response, extensions_omitted) = pool
                .pay_with_cover_observed(
                    &http,
                    expected,
                    retry,
                    response,
                    cover.as_ref(),
                    AdmissionObservation {
                        progress: &crate::payment::SubmissionProgress::default(),
                        recovery: &mut recovery,
                        hint: None,
                        fixed_payer: false,
                    },
                )
                .await?;
            if let Some(call) = &mut cover {
                call.protocol(response.version());
                call.rejected_padding(response.status());
                call.response_headers();
            }
            Ok(ManagedResponse {
                response,
                extensions_omitted,
                cover,
            })
        })
        .await?
    }
    /// Close direct payment admission and retain its accepted signing/dispatch
    /// work through completion. PaidClient owns its encompassing output lifecycle.
    pub async fn drain_direct_payments(&self) {
        self.direct_work.drain().await;
    }
    async fn recheck_payment_balances(
        &self,
        view: &mut super::base::ChainView,
        expected_wallet: &str,
        captures: &std::collections::BTreeMap<String, super::base::EvidenceCapture>,
    ) -> Result<()> {
        for (wallet, certificate) in &mut view.balance_certificates {
            if wallet != expected_wallet {
                continue;
            }
            match self
                .base
                .recheck_balance(certificate.clone(), captures[wallet].clone())
                .await
            {
                Ok(verified) => *certificate = verified,
                Err(error) => {
                    let failure_guard = Arc::new(self.gate.clone().lock_owned().await);
                    return self
                        .store
                        .call_with_guard(failure_guard, move |store| {
                            store.record_base_failure(&error)?;
                            Err(error)
                        })
                        .await;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn pay_with_cover_observed(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        mut retry: reqwest::Request,
        response: reqwest::Response,
        cover: Option<&crate::cover::runtime::Call>,
        observation: AdmissionObservation<'_>,
    ) -> Result<(reqwest::Response, bool)> {
        let AdmissionObservation {
            progress,
            recovery,
            hint,
            fixed_payer,
        } = observation;
        // Bound foreground owners, including callers awaiting the group gate.
        // The permit lives through the one signed dispatch and cannot cancel it.
        let _foreground = crate::payment::ownership::foreground()?;
        let (challenge, offer, omitted) = prepare_challenge(&response, &self.policy)?;
        let guard = Arc::new(self.gate.clone().lock_owned().await);
        let pool = self.pool.clone();
        let amount = offer.amount;
        self.store
            .call_with_guard(guard, move |s| s.validate_payment_price(&pool, amount))
            .await?;
        // Learning follows complete envelope validation, before financial admission.
        // An insufficient balance does not erase a valid observed price.
        if let Some((key, receipt)) = hint {
            crate::pricing::runtime::process_hints().learn(key, offer.amount, receipt);
        }
        let requirements_hash = keccak256(serde_json::to_vec(&offer.raw)?).to_string();
        let qualification = crate::qualification::payment_context()?;
        let (headers, attempt_id) = {
            let (mut view, mut guard) = if fixed_payer {
                self.acquire_fixed_payer(&expected).await?
            } else {
                match self.acquire_candidate(&expected, offer.amount).await? {
                    selection::CandidatePassResult::Selected(selected) => {
                        if selected.candidate.wallet != expected.wallet {
                            return Err(PayerHandoff(selected.candidate).into());
                        }
                        (selected.view, selected.guard)
                    }
                    selection::CandidatePassResult::CompleteNegative => {
                        anyhow::bail!(AdmissionError::FundingUnavailable(
                            "no individually sufficient wallet in completed captured inventory; no payment signed"
                        ))
                    }
                    selection::CandidatePassResult::Incomplete => {
                        anyhow::bail!(AdmissionError::WalletNotReady(
                            "candidate evidence acquisition incomplete; no payment signed"
                        ))
                    }
                }
            };
            #[cfg(test)]
            self.admission_test_stage(AdmissionTestStage::Acquired)
                .await;
            loop {
                let pool = self.pool.clone();
                let hash = requirements_hash.clone();
                let amount = offer.amount;
                let bound_payer = expected.clone();
                let evidence = view.clone();
                let prepared = self
                    .store
                    .call_with_guard(guard.clone(), move |s| {
                        s.prepare_payment(&pool, amount, &hash, evidence, Some(&bound_payer))
                    })
                    .await;
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error)
                        if error.downcast_ref::<super::base::evidence::EvidenceFailure>()
                            == Some(
                                &super::base::evidence::EvidenceFailure::CompatibilityRequired,
                            )
                            && recovery.take_evidence() =>
                    {
                        let wallets = vec![expected.wallet.clone()];
                        let captures = self
                            .store
                            .call_with_guard(guard.clone(), move |store| {
                                wallets
                                    .into_iter()
                                    .map(|wallet| {
                                        Ok((wallet.clone(), store.capture_evidence(&wallet)?))
                                    })
                                    .collect::<Result<std::collections::BTreeMap<_, _>>>()
                            })
                            .await?;
                        drop(guard);
                        self.recheck_payment_balances(&mut view, &expected.wallet, &captures)
                            .await?;
                        guard = Arc::new(self.gate.clone().lock_owned().await);
                        continue;
                    }
                    Err(error)
                        if matches!(
                            error.downcast_ref::<super::base::evidence::EvidenceFailure>(),
                            Some(
                                super::base::evidence::EvidenceFailure::Expired
                                    | super::base::evidence::EvidenceFailure::BeforeDebit
                            )
                        ) && recovery.take_evidence() =>
                    {
                        let pool = self.pool.clone();
                        let wallet = expected.wallet.clone();
                        let query = self
                            .store
                            .call_with_guard(guard.clone(), move |store| {
                                let wallets = vec![wallet];
                                store.scoped_chain_query(&pool, wallets, vec![])
                            })
                            .await?;
                        drop(guard);
                        view = self.acquire_view(query, true).await?;
                        guard = Arc::new(self.gate.clone().lock_owned().await);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let (signature, auth) = sign_authorization(&prepared, &offer, &challenge).await?;
                #[cfg(test)]
                self.admission_test_stage(AdmissionTestStage::Signed {
                    nonce: auth.nonce.clone(),
                    valid_before: auth.valid_before,
                })
                .await;
                let attempt_id = prepared.id.clone();
                let balance_valid_until = prepared.valid_until;
                let context = qualification.clone();
                // Mark before enqueueing: cancelled/lost acknowledgements never
                // permit relay retry, dispatch or replacement signing.
                progress.mark_possible();
                let committed = self
                    .store
                    .call_with_guard(guard.clone(), move |s| {
                        s.journal_authorization(&prepared, auth, context.as_ref())
                    })
                    .await;
                match committed {
                    Ok(()) => {
                        let mut headers = reqwest::header::HeaderMap::new();
                        headers.insert("payment-signature", signature.parse()?);
                        break (headers, attempt_id);
                    }
                    Err(error)
                        if error.is::<super::store::AdmissionExpired>()
                            && recovery.take_signature_replacement() =>
                    {
                        // This typed rejection proves rollback and the bytes are
                        // still private. Discard them and sign once more.
                        if now()? > balance_valid_until {
                            ensure!(
                                recovery.take_evidence(),
                                AdmissionError::WalletNotReady(
                                    "fresh Base balances required; evidence refresh allowance exhausted; no payment signed"
                                )
                            );
                            let pool = self.pool.clone();
                            let wallet = expected.wallet.clone();
                            let query = self
                                .store
                                .call_with_guard(guard.clone(), move |s| {
                                    let wallets = vec![wallet];
                                    s.scoped_chain_query(&pool, wallets, vec![])
                                })
                                .await?;
                            drop(guard);
                            view = self.acquire_view(query, true).await?;
                            guard = Arc::new(self.gate.clone().lock_owned().await);
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        #[cfg(test)]
        self.admission_test_stage(AdmissionTestStage::Journaled)
            .await;
        retry.headers_mut().extend(headers);
        let mut padding = cover.and_then(|c| c.pad(&mut retry, true));
        if let Some(p) = &mut padding {
            p.dispatched();
        }
        // One attempt only. Receipt/final 402/transport errors do not release exposure.
        let signed_request_started = qualification.as_ref().map(|c| c.elapsed_micros());
        let result = http.execute(retry).await.map_err(|_| {
            anyhow::anyhow!(AdmissionError::OutcomeUnknown(
                "signed request transport failed"
            ))
        });
        if let Some(context) = qualification {
            let response_observed = context.elapsed_micros();
            let observer = context.clone();
            let lifecycle = self.store.call(move |s| {
                let state = s.qualification_lifecycle()?;
                Ok(serde_json::json!({"observed_micros":observer.elapsed_micros(),"state":state}))
            }).await;
            context
                .record_response(
                    attempt_id,
                    signed_request_started.context("qualification request timing missing")?,
                    response_observed,
                    lifecycle,
                    &result,
                )
                .await?;
        }
        let response = if omitted {
            result.context(OMITTED_EXTENSIONS)?
        } else {
            result?
        };
        Ok((response, omitted))
    }
}

fn prepare_challenge(
    response: &reqwest::Response,
    policy: &SpendPolicy,
) -> Result<(x402_types::proto::PaymentRequired, Offer, bool)> {
    let header = response
        .headers()
        .get("payment-required")
        .context(AdmissionError::UnsupportedPayment("v2 challenge required"))?;
    let mut challenge: Value = serde_json::from_slice(&STANDARD.decode(header.as_bytes())?)?;
    let omitted = strip_extensions(&mut challenge)?;
    let offer = select_offer(&challenge, policy)?;
    challenge["accepts"] = serde_json::json!([offer.raw.clone()]);
    if let Some(desc) = challenge.pointer_mut("/resource/description")
        && let Some(text) = desc.as_str()
    {
        if text.chars().count() > 500 {
            tracing::warn!(
                limit_chars = 500,
                "x402 challenge description truncated for facilitator protocol compatibility"
            );
        }
        *desc = Value::String(text.chars().take(500).collect());
    }
    // Parse once; the SDK signs this exact selected envelope on each
    // permitted unpublished attempt, without reconstructing HTTP responses.
    let envelope: x402_types::proto::v2::PaymentRequired<x402_types::proto::OriginalJson> =
        serde_json::from_value(challenge)
            .map_err(|_| AdmissionError::UnsupportedPayment("malformed v2 envelope"))?;
    let challenge = x402_types::proto::PaymentRequired::V2(envelope);
    Ok((challenge, offer, omitted))
}

async fn sign_authorization(
    prepared: &super::store::PreparedPayment,
    offer: &Offer,
    challenge: &x402_types::proto::PaymentRequired,
) -> Result<(String, Authorization)> {
    let signer = PrivateKeySigner::from_slice(&prepared.key)
        .map_err(|_| anyhow::anyhow!("invalid stored signer"))?;
    ensure!(
        signer.address().to_string() == prepared.address,
        "stored signer identity mismatch"
    );
    let candidates = V2Eip155ExactClient::new(Arc::new(signer)).accept(challenge);
    let candidate = offer
        .select(&candidates)
        .context("managed SDK rejected selected requirements")?;
    let signature = candidate.sign().await.context("managed signing failed")?;
    let payload: Value = serde_json::from_slice(&STANDARD.decode(&signature)?)?;
    ensure!(
        payload
            .get("extensions")
            .is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty())),
        "managed signer unexpectedly emitted extensions"
    );
    let auth = validate_payload(&payload, offer, &prepared.address)?;
    Ok((signature, auth))
}

fn strip_extensions(challenge: &mut Value) -> Result<bool> {
    let Some(extensions) = challenge.get("extensions") else {
        return Ok(false);
    };
    let extensions = extensions
        .as_object()
        .context(AdmissionError::UnsupportedPayment(
            "malformed extensions: expected an object",
        ))?;
    let omitted = !extensions.is_empty();
    for name in extensions.keys() {
        if name == "bazaar" {
            tracing::debug!(extension = ?name, "managed payment omitting Bazaar discovery metadata before signing");
        } else {
            tracing::warn!(extension = ?name, "managed payment stripping advertised x402 extension; attempting payment without extensions; provider may reject; submitted authorization remains reserved until chain reconciliation");
        }
    }
    challenge
        .as_object_mut()
        .expect("challenge object")
        .remove("extensions");
    Ok(omitted)
}
fn select_offer(challenge: &Value, policy: &SpendPolicy) -> Result<Offer> {
    ensure!(
        challenge["x402Version"] == 2,
        AdmissionError::UnsupportedPayment("only v2 exact Base USDC is supported")
    );
    ensure!(
        challenge
            .get("extensions")
            .is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty())),
        AdmissionError::UnsupportedPayment("extensions require explicit support")
    );
    let mut price_rejected = false;
    let mut selected: Option<Offer> = None;
    for raw in challenge["accepts"]
        .as_array()
        .context(AdmissionError::UnsupportedPayment("missing offers"))?
    {
        let parsed = (|| -> Result<Offer> {
            ensure!(raw.is_object(), "malformed offer");
            ensure!(
                raw.get("outputSchema")
                    .is_none_or(|v| v.is_object() || v.is_boolean()),
                "malformed output schema metadata"
            );
            // Bounded opaque annotations are preserved in the accepted payload.
            // Only authority-bearing fields below determine compatibility.
            ensure!(
                raw["scheme"] == "exact"
                    && raw["network"] == "eip155:8453"
                    && raw["asset"]
                        .as_str()
                        .is_some_and(|v| v.eq_ignore_ascii_case(USDC)),
                "unsupported offer"
            );
            for location in [raw, raw.get("extra").unwrap_or(&Value::Null)] {
                ensure!(
                    location
                        .get("assetTransferMethod")
                        .is_none_or(|v| v == "eip3009"),
                    "unsupported transfer method"
                );
                ensure!(
                    location.get("flow").is_none_or(|v| v == "authorization"),
                    "unsupported flow"
                );
                ensure!(
                    location
                        .get("extensions")
                        .is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty())),
                    "unsupported extensions"
                );
            }
            ensure!(
                ["permit2", "permit2Authorization", "spender", "approval"]
                    .iter()
                    .all(|key| raw.get(*key).is_none()),
                "unsupported transfer mechanism"
            );
            let extra = raw["extra"].as_object().context("missing USDC domain")?;
            ensure!(
                [
                    "scheme",
                    "network",
                    "asset",
                    "amount",
                    "payTo",
                    "maxTimeoutSeconds",
                    "permit2",
                    "permit2Authorization",
                    "spender",
                    "approval"
                ]
                .iter()
                .all(|key| !extra.contains_key(*key)),
                "conflicting payment authority metadata"
            );
            ensure!(
                extra.get("name").is_some_and(|v| v == "USD Coin")
                    && extra.get("version").is_some_and(|v| v == "2"),
                "wrong USDC domain"
            );
            let amount = atomic(&raw["amount"])?;
            ensure!(amount > U256::ZERO, "zero amount");
            let payee: Address = raw["payTo"].as_str().context("missing payee")?.parse()?;
            ensure!(payee != Address::ZERO, "zero payee");
            let timeout = raw["maxTimeoutSeconds"]
                .as_u64()
                .context("invalid payment timeout")?;
            ensure!(
                timeout > 0 && timeout <= i64::MAX as u64 - now()?,
                "invalid payment timeout"
            );
            Ok(Offer {
                raw: raw.clone(),
                amount,
                payee,
            })
        })();
        if let Ok(offer) = parsed {
            if policy.max_atomic.is_some_and(|cap| offer.amount > cap) {
                price_rejected = true;
                continue;
            }
            // Keep the original exact offer, and retain challenge order on ties.
            if selected
                .as_ref()
                .is_none_or(|old| offer.amount < old.amount)
            {
                selected = Some(offer);
            }
        }
    }
    if let Some(offer) = selected {
        return Ok(offer);
    }
    anyhow::bail!(if price_rejected {
        AdmissionError::PriceLimit("no offer fits configured API payment ceiling")
    } else {
        AdmissionError::UnsupportedPayment("no compatible EIP-3009 offer")
    })
}
fn atomic(value: &Value) -> Result<U256> {
    let s = value.as_str().context("invalid atomic amount")?;
    ensure!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        "invalid atomic amount"
    );
    Ok(U256::from_str_radix(s, 10)?)
}
fn validate_payload(payload: &Value, offer: &Offer, payer: &str) -> Result<Authorization> {
    ensure!(
        payload["x402Version"] == 2 && payload["accepted"] == offer.raw,
        "payment payload requirements mismatch"
    );
    let auth = &payload["payload"]["authorization"];
    let address = |key: &str| -> Result<Address> {
        Ok(auth[key]
            .as_str()
            .context("missing authorization address")?
            .parse()?)
    };
    ensure!(
        address("from")? == payer.parse::<Address>()?
            && address("to")? == offer.payee
            && atomic(&auth["value"])? == offer.amount,
        "payment payload payer or amount mismatch"
    );
    let nonce: B256 = auth["nonce"].as_str().context("missing nonce")?.parse()?;
    let after = u64::try_from(atomic(&auth["validAfter"])?)?;
    let before = u64::try_from(atomic(&auth["validBefore"])?)?;
    let clock = now()?;
    ensure!(
        after <= clock
            && before > clock
            && before
                <= clock
                    .checked_add(offer.raw["maxTimeoutSeconds"].as_u64().unwrap())
                    .context("timeout overflow")?
            && after < before,
        "invalid signed validity interval"
    );
    ensure!(
        payload["payload"].get("permit2Authorization").is_none(),
        "unexpected Permit2 payload"
    );
    Ok(Authorization {
        payer: payer.into(),
        payee: offer.payee.to_string(),
        nonce: nonce.to_string(),
        valid_after: after,
        valid_before: before,
    })
}

#[cfg(test)]
mod offer_tests {
    use super::*;
    fn offer(amount: &str, payee: u8) -> Value {
        serde_json::json!({"scheme":"exact","network":"eip155:8453","asset":USDC,
            "amount":amount,"payTo":format!("0x{payee:040x}"),"maxTimeoutSeconds":60,
            "extra":{"name":"USD Coin","version":"2"},"annotation":{"opaque":true}})
    }
    #[test]
    fn sole_offer_ceiling_is_inclusive_and_unlimited_keeps_asset_validation() {
        let challenge = serde_json::json!({"x402Version":2,"accepts":[offer("100",1)]});
        assert_eq!(
            select_offer(
                &challenge,
                &SpendPolicy {
                    max_atomic: Some(U256::from(100))
                }
            )
            .unwrap()
            .amount,
            U256::from(100)
        );
        assert!(matches!(
            select_offer(
                &challenge,
                &SpendPolicy {
                    max_atomic: Some(U256::from(99))
                }
            )
            .err()
            .unwrap()
            .downcast_ref::<AdmissionError>(),
            Some(AdmissionError::PriceLimit(_))
        ));
        let mut large =
            serde_json::json!({"x402Version":2,"accepts":[offer(&U256::MAX.to_string(),1)]});
        assert_eq!(
            select_offer(&large, &SpendPolicy { max_atomic: None })
                .unwrap()
                .amount,
            U256::MAX
        );
        large["accepts"][0]["asset"] =
            serde_json::json!("0x0000000000000000000000000000000000000001");
        assert!(matches!(
            select_offer(&large, &SpendPolicy { max_atomic: None })
                .err()
                .unwrap()
                .downcast_ref::<AdmissionError>(),
            Some(AdmissionError::UnsupportedPayment(_))
        ));
    }
    #[test]
    fn lowest_supported_offer_wins_and_equal_prices_keep_exact_challenge_order() {
        let first = offer("100", 1);
        let second = offer("20", 2);
        let equal = offer("20", 3);
        let challenge = serde_json::json!({"x402Version":2,"accepts":[first,second,equal]});
        let selected = select_offer(
            &challenge,
            &SpendPolicy {
                max_atomic: Some(U256::from(100)),
            },
        )
        .unwrap();
        assert_eq!(selected.amount, U256::from(20));
        assert_eq!(selected.raw, challenge["accepts"][1]);
        assert_eq!(
            selected.payee,
            format!("0x{:040x}", 2).parse::<Address>().unwrap()
        );
    }
    #[test]
    fn unsupported_and_over_cap_offers_cannot_teach_an_affordable_price() {
        let mut unsupported = offer("1", 1);
        unsupported["scheme"] = serde_json::json!("upto");
        let challenge = serde_json::json!({"x402Version":2,"accepts":[unsupported,offer("101",2)]});
        let error = select_offer(
            &challenge,
            &SpendPolicy {
                max_atomic: Some(U256::from(100)),
            },
        );
        assert!(matches!(
            error.err().unwrap().downcast_ref::<AdmissionError>(),
            Some(AdmissionError::PriceLimit(_))
        ));
        let mut challenge = challenge;
        challenge["accepts"]
            .as_array_mut()
            .unwrap()
            .push(offer("50", 3));
        assert_eq!(
            select_offer(
                &challenge,
                &SpendPolicy {
                    max_atomic: Some(U256::from(100))
                },
            )
            .unwrap()
            .raw,
            challenge["accepts"][2]
        );
    }
    #[test]
    fn malformed_cheaper_or_equal_offer_does_not_hide_valid_exact_terms() {
        let mut cheap = offer("1", 1);
        cheap["extra"]["name"] = serde_json::json!("wrong domain");
        let mut equal = offer("20", 2);
        equal["maxTimeoutSeconds"] = serde_json::json!(0);
        let expected = offer("20", 3);
        let challenge = serde_json::json!({"x402Version":2,"accepts":[cheap,equal,expected]});
        let selected = select_offer(&challenge, &SpendPolicy { max_atomic: None }).unwrap();
        assert_eq!(selected.raw, challenge["accepts"][2]);
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[tokio::test]
    async fn independently_constructed_pool_handles_share_the_store_group_gate() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = super::super::store::Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let pool = crate::rotation::test_capacity::declare_and_allocate!(
            state,
            "shared",
            crate::rotation::test_capacity::two_reserves("5"),
            Some(crate::rotation::config::positive_usdc("5").unwrap()),
            0
        )
        .unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let make = || {
            ManagedPool::new(
                store.clone(),
                pool.clone(),
                BaseRpc::new("http://127.0.0.1:1", 12, 120).unwrap(),
                SpendPolicy::dollars("5").unwrap(),
            )
            .unwrap()
        };
        let first = make();
        let second = make();
        assert!(Arc::ptr_eq(&first.gate, &second.gate));
        assert!(Arc::ptr_eq(&first.reconciliation, &second.reconciliation));
        assert!(!Arc::ptr_eq(&first.gate, &first.reconciliation));
        let guard = first.gate.lock().await;
        assert!(second.gate.try_lock().is_err());
        drop(guard);
        assert!(second.gate.try_lock().is_ok());
        drop(first);
        drop(second);
        drop(store);
        worker.await.unwrap();
    }
}

#[cfg(test)]
mod recovery_allowance_tests {
    use super::RecoveryAllowance;

    #[test]
    fn compatibility_refresh_and_definite_rollback_allow_one_local_signature_replacement() {
        let mut recovery = RecoveryAllowance::default();
        assert!(recovery.take_evidence());
        assert!(
            recovery.take_signature_replacement(),
            "earlier header recheck cannot consume private-signature rollback recovery"
        );
        assert!(
            !recovery.take_evidence(),
            "expired balances after that rollback cannot acquire a second refresh"
        );
        assert!(
            !recovery.take_signature_replacement(),
            "a second expired private signature ends the invocation"
        );
    }

    #[test]
    fn definite_rollback_may_use_the_still_available_evidence_refresh() {
        let mut recovery = RecoveryAllowance::default();
        assert!(recovery.take_signature_replacement());
        assert!(recovery.take_evidence());
        assert!(!recovery.take_signature_replacement());
        assert!(!recovery.take_evidence());
    }
}
