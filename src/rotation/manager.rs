//! One shared gate per pool; signer leases are acquired only after a valid challenge.
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
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;
use x402_chain_eip155::V2Eip155ExactClient;
use x402_reqwest::X402Client;
use x402_types::scheme::client::{PaymentCandidate, PaymentSelector};

pub(crate) const OMITTED_EXTENSIONS: &str = "Managed payment omitted advertised x402 extensions; the provider may require them. A submitted authorization may still settle and remains reserved until chain reconciliation. Do not automatically retry";

#[derive(Clone, Debug)]
pub struct PaymentCandidateHandle {
    pub wallet: String,
    pub address: String,
    pub generation: i64,
}
pub struct ManagedPool {
    store: StoreHandle,
    pool: String,
    base: BaseRpc,
    target: U256,
    policy: SpendPolicy,
    wait: Duration,
    gate: Mutex<()>,
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
        wait_seconds: u64,
    ) -> Result<Self> {
        ensure!(
            wait_seconds > 0 && wait_seconds <= 3600,
            "invalid admission deadline"
        );
        Ok(Self {
            store,
            pool,
            base,
            target: positive_usdc(deposit)?,
            policy,
            wait: crate::network::global().request_timeout(Duration::from_secs(wait_seconds)),
            gate: Mutex::new(()),
        })
    }
    pub async fn reconcile(&self) -> Result<()> {
        let _gate = self.gate.lock().await;
        let pool = self.pool.clone();
        let query = self.store.call(move |s| s.chain_query(&pool)).await?;
        let observation = crate::qualification::pool_observation(&self.pool).await;
        let view = self.base.view(query).await?;
        let pool = self.pool.clone();
        self.store
            .call(move |s| {
                // Keep the RPC anchor and refreshed wallet set distinct from persisted
                // balances of retired wallets not included in this chain query.
                let evidence = observation.as_ref().map(|_| serde_json::json!({
                    "block_height":view.anchor.height,"block_hash":view.anchor.hash,
                    "confirmed":view.balances.iter().map(|(id,b)| (id.clone(),b.to_string())).collect::<std::collections::BTreeMap<_,_>>()
                }));
                s.reconcile_pool(&pool, view)?;
                if let Some(observation) = observation {
                    let result = (|| {
                        let mut evidence = evidence.context("missing pool balance evidence")?;
                        evidence["unresolved"] = s.qualification_exposure(&pool)?;
                        observation.record(s.qualification_lifecycle()?, evidence)
                    })();
                    if let Err(error) = result {
                        tracing::warn!(category="qualification_pool_observation_failed", %error,
                            "pool observation unavailable after successful reconciliation; normal reconciliation continues, no paid request is retried");
                    }
                }
                Ok(())
            })
            .await?;
        drop(_gate);
        crate::qualification::reconcile_debit(&self.base, &self.store, &self.pool).await
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
        mut retry: reqwest::Request,
        mut response: reqwest::Response,
        cover: Option<&crate::cover::runtime::Call>,
    ) -> Result<(reqwest::Response, bool)> {
        let header = response
            .headers()
            .get("payment-required")
            .context(AdmissionError::UnsupportedPayment("v2 challenge required"))?;
        let mut challenge: Value = serde_json::from_slice(&STANDARD.decode(header.as_bytes())?)?;
        let omitted = strip_extensions(&mut challenge)?;
        let offer = select_offer(&challenge, &self.policy, self.target)?;
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
        response.headers_mut().insert(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge)?).parse()?,
        );
        // Validate the SDK envelope before admission; malformed resource metadata
        // must not cause a promotion followed by an SDK parse failure.
        let _: x402_types::proto::v2::PaymentRequired<x402_types::proto::OriginalJson> =
            serde_json::from_value(challenge.clone())
                .map_err(|_| AdmissionError::UnsupportedPayment("malformed v2 envelope"))?;
        let requirements_hash = keccak256(serde_json::to_vec(&offer.raw)?).to_string();
        let qualification = crate::qualification::payment_context()?;
        let admission_context = qualification.clone();
        let (headers, attempt_id) = tokio::time::timeout(self.wait, async {
            let _guard = self.gate.lock().await;
            let pool = self.pool.clone();
            let query = self.store.call(move |s| s.chain_query(&pool)).await?;
            let view = self.base.view(query).await?;
            let pool = self.pool.clone();
            let hash = requirements_hash.clone();
            let amount = offer.amount;
            let lease = self
                .store
                .call(move |s| {
                    let lease = s.admit_for(&pool, amount, &hash, view, Some(&expected))?;
                    if let Some(context) = admission_context {
                        context.record(&s.qualification_payment(&lease.id)?)?;
                    }
                    Ok(lease)
                })
                .await?;
            let signer = PrivateKeySigner::from_slice(&lease.key)
                .map_err(|_| anyhow::anyhow!("invalid stored signer"))?;
            ensure!(
                signer.address().to_string() == lease.address,
                "stored signer identity mismatch"
            );
            let client = X402Client::new()
                .register(V2Eip155ExactClient::new(Arc::new(signer)))
                .with_selector(Offer {
                    raw: offer.raw.clone(),
                    amount: offer.amount,
                    payee: offer.payee,
                });
            let headers = client
                .make_payment_headers(response)
                .await
                .context("managed signing failed")?;
            let payload: Value = serde_json::from_slice(
                &STANDARD.decode(
                    headers
                        .get("payment-signature")
                        .context("missing payment signature")?
                        .as_bytes(),
                )?,
            )?;
            ensure!(
                payload
                    .get("extensions")
                    .is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty())),
                "managed signer unexpectedly emitted extensions"
            );
            let auth = validate_payload(&payload, &offer, &lease.address)?;
            let attempt_id = lease.id;
            let id = attempt_id.clone();
            let wallet = lease.wallet;
            let generation = lease.generation;
            let hash = requirements_hash;
            // Cancellation here retains the row, even if the accepted store operation
            // commits after its caller disappears. Signed bytes never leave before this.
            self.store
                .call(move |s| s.journal_authorization(&id, &wallet, generation, &hash, auth))
                .await?;
            // The journal now reserves exposure across cancellation, concurrent
            // admission, reconciliation and rotation. Do not hold the pool gate
            // while the seller establishes a connection or returns its response.
            Ok::<_, anyhow::Error>((headers, attempt_id))
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(AdmissionError::WalletNotReady(
                "admission deadline exceeded"
            ))
        })??;
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
            ensure!(
                raw.as_object().is_some_and(|m| m.keys().all(|k| matches!(
                    k.as_str(),
                    "scheme"
                        | "network"
                        | "asset"
                        | "amount"
                        | "payTo"
                        | "maxTimeoutSeconds"
                        | "outputSchema"
                        | "extra"
                        | "assetTransferMethod"
                        | "flow"
                        | "extensions"
                ))),
                "unknown offer metadata"
            );
            // Descriptive JSON Schema only: preserve it, but never resolve references,
            // evaluate it, or use its contents as payment terms.
            ensure!(
                raw.get("outputSchema")
                    .is_none_or(|v| v.is_object() || v.is_boolean()),
                "malformed output schema metadata"
            );
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
            let extra = raw["extra"].as_object().context("missing USDC domain")?;
            ensure!(
                extra.keys().all(|k| matches!(
                    k.as_str(),
                    "name"
                        | "version"
                        | "assetTransferMethod"
                        | "flow"
                        | "facilitatorAddress"
                        | "breakdown"
                        | "totalUsd"
                        | "acceptId"
                        | "merchant"
                        | "tier"
                )),
                "unknown payment metadata"
            );
            // Reviewed vendor annotations are echoed unchanged, never used to select a
            // signing mechanism or compute the reservation. Atomic `amount` is authoritative.
            ensure!(
                ["acceptId", "merchant", "tier"]
                    .iter()
                    .all(|key| extra.get(*key).is_none_or(Value::is_string))
                    && extra
                        .get("totalUsd")
                        .is_none_or(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
                    && extra
                        .get("breakdown")
                        .is_none_or(|v| v.as_object().is_some_and(|m| m
                            .values()
                            .all(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0)))),
                "malformed informational payment metadata"
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
        AdmissionError::PriceLimit("no offer fits cap and deposit_size")
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
        "payment payload lease mismatch"
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
