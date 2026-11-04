//! Bound newly signed authority without changing the seller's accepted offer.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use x402_types::{
    proto::{OriginalJson, PaymentRequired},
    scheme::{
        X402SchemeId,
        client::{PaymentCandidate, PaymentCandidateSigner, X402Error, X402SchemeClient},
    },
};

pub(crate) const MAX_AUTHORITY_SECONDS: u64 = 300;

pub(crate) struct BoundedClient<C>(pub(crate) C);

impl<C: X402SchemeId> X402SchemeId for BoundedClient<C> {
    fn x402_version(&self) -> u8 {
        self.0.x402_version()
    }
    fn namespace(&self) -> &str {
        self.0.namespace()
    }
    fn scheme(&self) -> &str {
        self.0.scheme()
    }
}

impl<C: X402SchemeClient> X402SchemeClient for BoundedClient<C> {
    fn accept(&self, required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let offers = match required {
            PaymentRequired::V1(value) => &value.accepts,
            PaymentRequired::V2(value) => &value.accepts,
        };
        let mut candidates = Vec::new();
        let mut challenge = match required {
            PaymentRequired::V1(value) => PaymentRequired::V1(value.clone()),
            PaymentRequired::V2(value) => PaymentRequired::V2(value.clone()),
        };
        // Process offers independently: identical payment fields may carry distinct
        // opaque metadata, and each resulting signer must retain its own original.
        for original in offers {
            let Ok(mut offer) =
                serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(original.0.get())
            else {
                continue;
            };
            let Some(timeout) = offer
                .get("maxTimeoutSeconds")
                .and_then(|value| serde_json::from_str::<u64>(value.get()).ok())
                .filter(|value| *value > 0)
            else {
                continue;
            };
            offer.insert(
                "maxTimeoutSeconds".into(),
                serde_json::value::to_raw_value(&timeout.min(MAX_AUTHORITY_SECONDS))
                    .expect("integer serialization"),
            );
            let bounded = OriginalJson(
                serde_json::value::to_raw_value(&offer).expect("JSON value serialization"),
            );
            match &mut challenge {
                PaymentRequired::V1(value) => {
                    value.accepts = vec![bounded];
                }
                PaymentRequired::V2(value) => {
                    value.accepts = vec![bounded];
                }
            };
            for mut candidate in self.0.accept(&challenge) {
                candidate.signer = Box::new(OriginalOfferSigner {
                    inner: candidate.signer,
                    original: match required {
                        PaymentRequired::V1(_) => None,
                        PaymentRequired::V2(_) => Some(original.clone()),
                    },
                });
                candidates.push(candidate);
            }
        }
        candidates
    }
}

struct OriginalOfferSigner {
    inner: Box<dyn PaymentCandidateSigner + Send + Sync>,
    original: Option<OriginalJson>,
}

#[async_trait::async_trait]
impl PaymentCandidateSigner for OriginalOfferSigner {
    async fn sign_payment(&self) -> Result<String, X402Error> {
        let signed = self.inner.sign_payment().await?;
        let Some(original) = &self.original else {
            return Ok(signed);
        };
        // accepted is envelope metadata, outside the signed authorization. Restore
        // its raw JSON, including opaque numbers; the SDK owns all signed fields.
        let bytes = STANDARD
            .decode(signed)
            .map_err(|_| X402Error::SigningError("invalid SDK payment encoding".into()))?;
        let mut payload: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(&bytes)?;
        payload.insert("accepted".into(), original.0.clone());
        Ok(STANDARD.encode(serde_json::to_vec(&payload)?))
    }
}

#[cfg(test)]
#[path = "lifetime_tests.rs"]
mod tests;
