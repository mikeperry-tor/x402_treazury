//! Shared pool ownership and atomic signed admission after a valid challenge.
use super::error::AdmissionError;
use super::{
    base::{BaseRpc, now},
    config::positive_usdc,
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
    pub generation: i64,
}
#[cfg(test)]
mod admission_tests;
mod maintenance;

#[cfg(test)]
#[derive(Clone)]
enum AdmissionTestStage {
    Acquired,
    Journaled,
    Signed { nonce: String, valid_before: u64 },
}
#[cfg(test)]
type AdmissionTestHook =
    Arc<dyn Fn(AdmissionTestStage) -> futures_util::future::BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct ManagedPool {
    store: StoreHandle,
    pool: String,
    base: BaseRpc,
    payment_limit: U256,
    policy: SpendPolicy,
    gate: Arc<Mutex<()>>,
    reconciliation: Arc<Mutex<()>>,
    #[cfg(test)]
    test_admission_hook: Option<AdmissionTestHook>,
}
#[derive(Default)]
struct RecoveryAllowance {
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
        deposit: &str,
        policy: SpendPolicy,
    ) -> Result<Self> {
        Ok(Self {
            gate: store.group_gate(&pool)?,
            reconciliation: store.reconciliation_gate(&pool)?,
            #[cfg(test)]
            test_admission_hook: None,
            store,
            pool,
            base,
            payment_limit: positive_usdc(deposit)?.min(policy.max_atomic.unwrap_or(U256::MAX)),
            policy,
        })
    }
    #[cfg(test)]
    async fn admission_test_stage(&self, stage: AdmissionTestStage) {
        if let Some(hook) = &self.test_admission_hook {
            hook(stage).await;
        }
    }
    /// Explicit operator opt-out of the historical funding-target payment ceiling.
    pub fn with_funding_target_limit(mut self, enabled: bool) -> Self {
        if !enabled {
            self.payment_limit = self.policy.max_atomic.unwrap_or(U256::MAX);
        }
        self
    }
    async fn acquire_view(
        &self,
        query: super::base::ChainQuery,
        payment: bool,
        guard: Option<Arc<tokio::sync::OwnedMutexGuard<()>>>,
    ) -> Result<super::base::ChainView> {
        let result = if payment {
            self.base.payment_view(query).await
        } else {
            self.base
                .shared_balance_view(query, self.store.clone())
                .await
        };
        match result {
            Ok(view) => Ok(view),
            Err(error) => {
                let guard = match guard {
                    Some(guard) => guard,
                    None => Arc::new(self.gate.clone().lock_owned().await),
                };
                self.store
                    .call_with_guard(guard, move |store| {
                        store.record_base_failure(&error)?;
                        Err(error)
                    })
                    .await
            }
        }
    }
    pub async fn candidate(&self) -> Result<PaymentCandidateHandle> {
        let pool = self.pool.clone();
        self.store.call(move |s| s.payment_candidate(&pool)).await
    }
    pub async fn pay(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        retry: reqwest::Request,
        response: reqwest::Response,
    ) -> Result<(reqwest::Response, bool)> {
        self.pay_with_cover(http, expected, retry, response, None)
            .await
    }
    pub async fn pay_with_cover(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        retry: reqwest::Request,
        response: reqwest::Response,
        cover: Option<&crate::cover::runtime::Call>,
    ) -> Result<(reqwest::Response, bool)> {
        self.pay_with_cover_observed(
            http,
            expected,
            retry,
            response,
            cover,
            &crate::payment::SubmissionProgress::default(),
        )
        .await
    }
    pub(crate) async fn pay_with_cover_observed(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        mut retry: reqwest::Request,
        response: reqwest::Response,
        cover: Option<&crate::cover::runtime::Call>,
        progress: &crate::payment::SubmissionProgress,
    ) -> Result<(reqwest::Response, bool)> {
        // Bound foreground owners, including callers awaiting the group gate.
        // The permit lives through the one signed dispatch and cannot cancel it.
        static FOREGROUND: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(128);
        let _foreground = FOREGROUND
            .try_acquire()
            .context("managed payment queue capacity unavailable")?;
        let header = response
            .headers()
            .get("payment-required")
            .context(AdmissionError::UnsupportedPayment("v2 challenge required"))?;
        let mut challenge: Value = serde_json::from_slice(&STANDARD.decode(header.as_bytes())?)?;
        let omitted = strip_extensions(&mut challenge)?;
        let offer = select_offer(&challenge, &self.policy, self.payment_limit)?;
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
        let requirements_hash = keccak256(serde_json::to_vec(&offer.raw)?).to_string();
        let qualification = crate::qualification::payment_context()?;
        let (headers, attempt_id) = {
            let mut guard = Arc::new(self.gate.clone().lock_owned().await);
            let pool = self.pool.clone();
            let query = self
                .store
                .call_with_guard(guard.clone(), move |s| {
                    let boundary = s.wallet_boundary(&pool)?;
                    let wallets = s
                        .priority_wallets(&pool, boundary)?
                        .into_iter()
                        .map(|(_, id, _)| id)
                        .collect();
                    s.scoped_chain_query(&pool, wallets, vec![])
                })
                .await?;
            drop(guard);
            let mut view = self.acquire_view(query, true, None).await?;
            #[cfg(test)]
            self.admission_test_stage(AdmissionTestStage::Acquired)
                .await;
            guard = Arc::new(self.gate.clone().lock_owned().await);
            let mut recovery = RecoveryAllowance::default();
            loop {
                let pool = self.pool.clone();
                let hash = requirements_hash.clone();
                let amount = offer.amount;
                let payment_limit = self.payment_limit;
                let bound_payer = expected.clone();
                let evidence = view.clone();
                let prepared = self
                    .store
                    .call_with_guard(guard.clone(), move |s| {
                        s.prepare_payment(
                            &pool,
                            amount,
                            &hash,
                            evidence,
                            Some(&bound_payer),
                            payment_limit,
                        )
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
                        for (wallet, certificate) in &mut view.balance_certificates {
                            if wallet != &expected.wallet {
                                continue;
                            }
                            match self
                                .base
                                .recheck_balance(certificate.clone(), captures[wallet].clone())
                                .await
                            {
                                Ok(verified) => *certificate = verified,
                                Err(error) => {
                                    let failure_guard =
                                        Arc::new(self.gate.clone().lock_owned().await);
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
                        guard = Arc::new(self.gate.clone().lock_owned().await);
                        continue;
                    }
                    Err(error)
                        if matches!(
                            error.downcast_ref::<super::base::evidence::EvidenceFailure>(),
                            Some(
                                super::base::evidence::EvidenceFailure::Expired
                                    | super::base::evidence::EvidenceFailure::BeforeDebit
                                    | super::base::evidence::EvidenceFailure::Invalidated
                            )
                        ) && recovery.take_evidence() =>
                    {
                        let pool = self.pool.clone();
                        let query = self
                            .store
                            .call_with_guard(guard.clone(), move |store| {
                                let boundary = store.wallet_boundary(&pool)?;
                                let wallets = store
                                    .priority_wallets(&pool, boundary)?
                                    .into_iter()
                                    .map(|(_, id, _)| id)
                                    .collect();
                                store.scoped_chain_query(&pool, wallets, vec![])
                            })
                            .await?;
                        drop(guard);
                        view = self.acquire_view(query, true, None).await?;
                        guard = Arc::new(self.gate.clone().lock_owned().await);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let signer = PrivateKeySigner::from_slice(&prepared.key)
                    .map_err(|_| anyhow::anyhow!("invalid stored signer"))?;
                ensure!(
                    signer.address().to_string() == prepared.address,
                    "stored signer identity mismatch"
                );
                let candidates = V2Eip155ExactClient::new(Arc::new(signer)).accept(&challenge);
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
                let auth = validate_payload(&payload, &offer, &prepared.address)?;
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
                            let query = self
                                .store
                                .call_with_guard(guard.clone(), move |s| {
                                    let boundary = s.wallet_boundary(&pool)?;
                                    let wallets = s
                                        .priority_wallets(&pool, boundary)?
                                        .into_iter()
                                        .map(|(_, id, _)| id)
                                        .collect();
                                    s.scoped_chain_query(&pool, wallets, vec![])
                                })
                                .await?;
                            drop(guard);
                            view = self.acquire_view(query, true, None).await?;
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
        tracing::warn!(extension = ?name, "managed payment stripping advertised x402 extension; attempting payment without extensions; provider may reject; submitted authorization remains reserved until chain reconciliation");
    }
    challenge
        .as_object_mut()
        .expect("challenge object")
        .remove("extensions");
    Ok(omitted)
}
fn select_offer(challenge: &Value, policy: &SpendPolicy, target: U256) -> Result<Offer> {
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
            if offer.amount > target || policy.max_atomic.is_some_and(|cap| offer.amount > cap) {
                price_rejected = true;
                continue;
            }
            return Ok(offer);
        }
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
        let pool = state.ensure_pool("shared", "5").unwrap();
        let (store, worker) = StoreHandle::spawn(state);
        let make = || {
            ManagedPool::new(
                store.clone(),
                pool.clone(),
                BaseRpc::new("http://127.0.0.1:1", 12, 120).unwrap(),
                "5",
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
