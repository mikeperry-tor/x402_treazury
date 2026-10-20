//! Shared pool ownership and atomic signed admission after a valid challenge.
use super::error::AdmissionError;
use super::{
    base::{BaseRpc, now},
    store::{Authorization, StoreHandle},
};
use crate::payment::{SpendPolicy, USDC, diagnostics};
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use x402_chain_eip155::V2Eip155ExactClient;
use x402_types::scheme::client::{PaymentCandidate, PaymentSelector, X402SchemeClient};

pub(crate) const OMITTED_EXTENSIONS: &str = "Managed payment omitted advertised x402 extensions; the provider may require them. A submitted authorization may still charge. Only the built-in recognized insufficient-funds traversal may repeat the request";

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
    pub through: i64,
    pub attempted: &'a [String],
    pub progress: &'a crate::payment::SubmissionProgress,
    pub hint: Option<(
        crate::pricing::runtime::OperationKey,
        crate::pricing::runtime::Receipt,
    )>,
    pub promotion: Option<(
        &'a Arc<crate::network::HttpTransport>,
        &'a Arc<crate::network::HttpTransport>,
    )>,
}
#[cfg(test)]
mod admission_tests;
mod maintenance;
mod selection;

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
    direct_work: crate::server::work::Work,
    store: StoreHandle,
    pool: String,
    base: BaseRpc,
    policy: SpendPolicy,
    gate: Arc<Mutex<()>>,
    reconciliation: Arc<Mutex<()>>,
    #[cfg(test)]
    reconciliation_test_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    test_admission_hook: Option<AdmissionTestHook>,
}
struct Offer {
    version: u8,
    raw: Value,
    amount: U256,
    payee: Address,
}
impl PaymentSelector for Offer {
    fn select<'a>(&self, candidates: &'a [PaymentCandidate]) -> Option<&'a PaymentCandidate> {
        candidates.iter().find(|c| {
            c.x402_version == self.version
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
            reconciliation_test_hook: None,
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

    pub(crate) async fn inventory_boundary(&self) -> Result<i64> {
        let pool = self.pool.clone();
        self.store.call(move |s| s.wallet_boundary(&pool)).await
    }
    pub(crate) async fn next_candidate(
        &self,
        selection_amount: U256,
        through: i64,
        attempted: &[String],
    ) -> Result<Option<PaymentCandidateHandle>> {
        let pool = self.pool.clone();
        let attempted = attempted.to_vec();
        self.store
            .call(move |s| s.advisory_candidate(&pool, selection_amount, None, through, &attempted))
            .await
    }
    pub(crate) async fn note_insufficient(&self, wallet: String) -> Result<()> {
        let pool = self.pool.clone();
        self.store
            .call(move |s| s.low_balance(&pool, &wallet))
            .await
    }
    pub async fn candidate(&self) -> Result<PaymentCandidateHandle> {
        let pool = self.pool.clone();
        self.store.call(move |s| s.payment_candidate(&pool)).await
    }
    pub(crate) async fn discovery_changes(&self) -> Result<Arc<tokio::sync::Notify>> {
        let pool = self.pool.clone();
        self.store
            .call(move |s| Ok(s.capacity_changes(&pool)))
            .await
    }
    pub(crate) async fn discovery_requires_creation(&self) -> Result<bool> {
        let pool = self.pool.clone();
        self.store
            .call(move |s| s.payment_identity_requires_creation(&pool))
            .await
    }
    pub(crate) async fn discovery_ready(&self) -> Result<bool> {
        let pool = self.pool.clone();
        self.store
            .call(move |s| s.payment_identity_available(&pool))
            .await
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
    /// Low-level dispatch for an already bound transport/challenge. Use
    /// `PaidClient::managed` for logical calls with identity-bound traversal.
    #[tracing::instrument(level = "warn", name = "direct_payment", skip_all, fields(call_id = %uuid::Uuid::new_v4()))]
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
        crate::payment::ownership::run(
            &self.direct_work,
            diagnostics::scope(async move {
                diagnostics::stage("challenge_preparation");
                let progress = crate::payment::SubmissionProgress::default();
                let result: Result<ManagedResponse> = async {
                    let through = pool.inventory_boundary().await?;
                    let response = if response.headers().contains_key("payment-required") {
                        response
                    } else {
                        crate::payment::bound_challenge(response, crate::limits::RESPONSE_BYTES)
                            .await?
                    };
                    let (response, extensions_omitted, _) = pool
                        .pay_with_cover_observed(
                            &http,
                            expected,
                            retry,
                            response,
                            cover.as_ref(),
                            AdmissionObservation {
                                progress: &progress,
                                hint: None,
                                promotion: None,
                                through,
                                attempted: &[],
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
                }
                .await;
                match &result {
                    Ok(owned) => diagnostics::finish_response(&owned.response, &progress),
                    Err(_) => diagnostics::finish(&result, &progress),
                }
                result
            }),
        )
        .await?
    }
    /// Close direct payment admission and retain its accepted signing/dispatch
    /// work through completion. PaidClient owns its encompassing output lifecycle.
    pub async fn drain_direct_payments(&self) {
        self.direct_work.drain().await;
    }
    pub(crate) async fn pay_with_cover_observed(
        &self,
        http: &reqwest::Client,
        expected: PaymentCandidateHandle,
        mut retry: reqwest::Request,
        response: reqwest::Response,
        cover: Option<&crate::cover::runtime::Call>,
        observation: AdmissionObservation<'_>,
    ) -> Result<(reqwest::Response, bool, U256)> {
        let AdmissionObservation {
            progress,
            hint,
            promotion,
            through,
            attempted,
        } = observation;
        // Bound foreground owners, including callers awaiting the group gate.
        // The permit lives through the one signed dispatch and cannot cancel it.
        let _foreground = crate::payment::ownership::foreground()?;
        diagnostics::stage("challenge_preparation");
        let (challenge, offer, omitted) = prepare_challenge(&response, &self.policy)?;
        if let Some((selected, transport)) = promotion {
            crate::network::global().promote_http(selected, transport);
        }
        // Learning follows complete envelope validation, before financial admission.
        // An insufficient balance does not erase a valid observed price.
        if let Some((key, receipt)) = hint {
            crate::pricing::runtime::process_hints().learn(key, offer.amount, receipt);
        }
        let requirements_hash = keccak256(serde_json::to_vec(&offer.raw)?).to_string();
        let qualification = crate::qualification::payment_context()?;
        diagnostics::stage("wallet_admission");
        {
            let selected = self
                .acquire_candidate(&expected, offer.amount, through, attempted)
                .await?
                .context(AdmissionError::WalletNotReady(
                    "no eligible payment identity",
                ))?;
            if selected.wallet != expected.wallet {
                return Err(PayerHandoff(selected).into());
            }
        }
        #[cfg(test)]
        self.admission_test_stage(AdmissionTestStage::Acquired)
            .await;
        let (headers, attempt_id) = {
            let pool = self.pool.clone();
            let hash = requirements_hash.clone();
            let amount = offer.amount;
            let bound = expected.clone();
            let prepared = self
                .store
                .call(move |s| s.prepare_advisory_payment(&pool, amount, &hash, &bound))
                .await?;
            diagnostics::stage("signing");
            let (signature, auth) = sign_authorization(&prepared, &offer, &challenge).await?;
            #[cfg(test)]
            self.admission_test_stage(AdmissionTestStage::Signed {
                nonce: auth.nonce.clone(),
                valid_before: auth.valid_before,
            })
            .await;
            let attempt_id = prepared.id.clone();
            let context = qualification.clone();
            // Lost journal acknowledgement must never imply no accepted intent.
            diagnostics::stage("authorization_journal");
            progress.mark_possible();
            self.store
                .call(move |s| s.journal_authorization(&prepared, auth, context.as_ref()))
                .await?;
            let mut headers = reqwest::header::HeaderMap::new();
            let header = if offer.version == 1 {
                "x-payment"
            } else {
                "payment-signature"
            };
            headers.insert(header, signature.parse()?);
            (headers, attempt_id)
        };
        #[cfg(test)]
        self.admission_test_stage(AdmissionTestStage::Journaled)
            .await;
        retry.headers_mut().extend(headers);
        let mut padding = cover.and_then(|c| c.pad(&mut retry, true));
        if let Some(p) = &mut padding {
            p.dispatched();
        }
        // This bound attempt owns its own signature. A seller response never erases it.
        let signed_request_started = qualification.as_ref().map(|c| c.elapsed_micros());
        diagnostics::stage("signed_request");
        let result = http.execute(retry).await.map_err(|_| {
            anyhow::anyhow!(AdmissionError::OutcomeUnknown(
                "signed request transport failed"
            ))
        });
        if let Ok(response) = &result {
            diagnostics::response(response);
        }
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
        Ok((response, omitted, offer.amount))
    }
}

fn prepare_challenge(
    response: &reqwest::Response,
    policy: &SpendPolicy,
) -> Result<(x402_types::proto::PaymentRequired, Offer, bool)> {
    let header =
        response
            .headers()
            .get("payment-required")
            .context(AdmissionError::UnsupportedPayment(
                "payment challenge required",
            ))?;
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
    let challenge = if offer.version == 1 {
        let envelope: x402_types::proto::v1::PaymentRequired<x402_types::proto::OriginalJson> =
            serde_json::from_value(challenge)
                .map_err(|_| AdmissionError::UnsupportedPayment("malformed v1 envelope"))?;
        x402_types::proto::PaymentRequired::V1(envelope)
    } else {
        let envelope: x402_types::proto::v2::PaymentRequired<x402_types::proto::OriginalJson> =
            serde_json::from_value(challenge)
                .map_err(|_| AdmissionError::UnsupportedPayment("malformed v2 envelope"))?;
        x402_types::proto::PaymentRequired::V2(envelope)
    };
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
    let signer = Arc::new(signer);
    let candidates = if offer.version == 1 {
        crate::payment::V1ExactClient::new(signer).accept(challenge)
    } else {
        V2Eip155ExactClient::new(signer).accept(challenge)
    };
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
            tracing::info!(extension = ?name, stage = "challenge_preparation", reason = "unsupported_extension", "managed payment omitting advertised x402 extension before offer validation and signing; provider may require it");
        }
    }
    challenge
        .as_object_mut()
        .expect("challenge object")
        .remove("extensions");
    Ok(omitted)
}
fn select_offer(challenge: &Value, policy: &SpendPolicy) -> Result<Offer> {
    let version = match challenge["x402Version"].as_u64() {
        Some(1) => 1u8,
        Some(2) => 2u8,
        _ => anyhow::bail!(AdmissionError::UnsupportedPayment(
            "only v1/v2 exact Base USDC is supported"
        )),
    };
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
            let amount_raw = if version == 1 {
                ensure!(
                    raw["network"] == "base"
                        && raw["resource"].is_string()
                        && raw["description"].is_string()
                        && raw
                            .get("mimeType")
                            .is_none_or(|v| v.is_null() || v.is_string())
                        && raw.get("amount").is_none(),
                    "unsupported offer"
                );
                &raw["maxAmountRequired"]
            } else {
                ensure!(
                    raw["network"] == "eip155:8453" && raw.get("maxAmountRequired").is_none(),
                    "unsupported offer"
                );
                &raw["amount"]
            };
            ensure!(
                raw["scheme"] == "exact"
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
                    "maxAmountRequired",
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
            let amount = atomic(amount_raw)?;
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
                version,
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
    if offer.version == 1 {
        ensure!(
            payload["x402Version"] == 1
                && payload["scheme"] == "exact"
                && payload["network"] == "base",
            "payment payload requirements mismatch"
        );
    } else {
        ensure!(
            payload["x402Version"] == 2 && payload["accepted"] == offer.raw,
            "payment payload requirements mismatch"
        );
    }
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
    #[tokio::test]
    async fn body_only_v1_uses_the_owning_response_limit_once() {
        let v1 = serde_json::json!({"x402Version":1,"accepts":[{
            "scheme":"exact","network":"base","asset":USDC,"maxAmountRequired":"1",
            "resource":"http://localhost/pay","description":"legacy fixture","mimeType":"application/json",
            "payTo":format!("0x{:040x}",1),"maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
        }]});
        // Whitespace makes the wire body exceed the default without inventing
        // a large offer or changing the amount authority.
        let mut body = vec![b' '; crate::limits::RESPONSE_BYTES + 1];
        body.extend_from_slice(v1.to_string().as_bytes());
        let response = |body: Vec<u8>| {
            reqwest::Response::from(
                axum::http::Response::builder()
                    .status(402)
                    .body(body)
                    .unwrap(),
            )
        };
        let error =
            crate::payment::bound_challenge(response(body.clone()), crate::limits::RESPONSE_BYTES)
                .await
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("payment challenge rejected before signing")
        );
        let normalized = crate::payment::bound_challenge(response(body.clone()), body.len())
            .await
            .unwrap();
        let (_, selected, _) = prepare_challenge(
            &normalized,
            &SpendPolicy {
                max_atomic: Some(U256::from(1)),
            },
        )
        .unwrap();
        assert_eq!(selected.version, 1);
        assert_eq!(selected.amount, U256::from(1));
        assert_eq!(selected.raw, v1["accepts"][0]);
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
    #[test]
    fn v1_base_exact_offer_selection_requires_v1_fields_and_forbids_conflicting_amounts() {
        let valid_v1 = serde_json::json!({
            "scheme": "exact",
            "network": "base",
            "asset": USDC,
            "maxAmountRequired": "50000",
            "resource": "https://api.relayshield.net/v1/payg/phone-reputation",
            "description": "Phone reputation check",
            "mimeType": "application/json",
            "payTo": format!("0x{:040x}", 1),
            "maxTimeoutSeconds": 300,
            "extra": {"name": "USD Coin", "version": "2"},
            "outputSchema": {"input": {"type": "http"}}
        });
        let mut bad_mime = valid_v1.clone();
        bad_mime["maxAmountRequired"] = serde_json::json!("1");
        bad_mime["mimeType"] = serde_json::json!(123);
        let mut bad_dual_amount = valid_v1.clone();
        bad_dual_amount["maxAmountRequired"] = serde_json::json!("1");
        bad_dual_amount["amount"] = serde_json::json!("1");
        let mut bad_extra = valid_v1.clone();
        bad_extra["maxAmountRequired"] = serde_json::json!("1");
        bad_extra["extra"]["maxAmountRequired"] = serde_json::json!("1");
        let challenge = serde_json::json!({
            "x402Version": 1,
            "accepts": [bad_mime, bad_dual_amount, bad_extra, valid_v1]
        });
        let selected = select_offer(
            &challenge,
            &SpendPolicy {
                max_atomic: Some(U256::from(50000)),
            },
        )
        .unwrap();
        assert_eq!(selected.version, 1);
        assert_eq!(selected.amount, U256::from(50000));
        assert_eq!(selected.raw, challenge["accepts"][3]);
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
                BaseRpc::new("http://127.0.0.1:1", 12).unwrap(),
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
