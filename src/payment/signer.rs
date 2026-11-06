//! Native EVM signing with best-effort erasure of each owned key.
use alloy_primitives::{Address, B256, Signature};
use anyhow::Result;
use secp256k1::{Message, SECP256K1, SecretKey};
use x402_chain_eip155::v1_eip155_exact::client::SignerLike;
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct EvmSigner {
    key: SecretKey,
    address: Address,
}

impl Drop for EvmSigner {
    fn drop(&mut self) {
        // Wipe this owner's storage in place. Rust may still leave copies made
        // by moves or upstream native operations, as documented by secp256k1.
        self.key.non_secure_erase();
    }
}

impl EvmSigner {
    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self> {
        let key =
            SecretKey::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid EVM private key"))?;
        Ok(Self::from_key(key))
    }
    fn from_key(key: SecretKey) -> Self {
        let public = key.public_key(SECP256K1).serialize_uncompressed();
        let hash = alloy_primitives::keccak256(&public[1..]);
        Self {
            key,
            address: Address::from_slice(&hash[12..]),
        }
    }
    pub(crate) fn random() -> Self {
        Self::from_key(SecretKey::new(&mut secp256k1::rand::thread_rng()))
    }
    pub(crate) fn address(&self) -> Address {
        self.address
    }
    pub(crate) fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.key.secret_bytes())
    }
}

impl std::str::FromStr for EvmSigner {
    type Err = anyhow::Error;
    fn from_str(key: &str) -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        alloy_primitives::hex::decode_to_slice(key, &mut bytes[..])
            .map_err(|_| anyhow::anyhow!("invalid EVM private key"))?;
        Self::from_slice(&bytes[..])
    }
}

#[async_trait::async_trait]
impl SignerLike for EvmSigner {
    fn address(&self) -> Address {
        self.address()
    }
    async fn sign_hash(&self, hash: &B256) -> Result<Signature, alloy_signer::Error> {
        let signature = SECP256K1.sign_ecdsa_recoverable(&Message::from_digest(hash.0), &self.key);
        let (recovery, bytes) = signature.serialize_compact();
        Ok(Signature::from_scalars_and_parity(
            B256::from_slice(&bytes[..32]),
            B256::from_slice(&bytes[32..]),
            i32::from(recovery) & 1 != 0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn production_signer_matches_independent_python_signature_and_saved_key() {
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/crypto_vectors.json")).unwrap();
        let vector = &vector["eip3009"];
        // Public test key used by scripts/generate_crypto_vectors.py.
        let key = format!("{:064x}", 1);
        let signer: EvmSigner = key.parse().unwrap();
        let hash: B256 = vector["hash"].as_str().unwrap().parse().unwrap();
        let expected: Signature = vector["signature"].as_str().unwrap().parse().unwrap();
        assert_eq!(signer.sign_hash(&hash).await.unwrap(), expected);
        assert_eq!(
            signer.address(),
            vector["typed_data"]["message"]["from"]
                .as_str()
                .unwrap()
                .parse::<Address>()
                .unwrap()
        );
        let cloned = signer.clone();
        let restored = EvmSigner::from_slice(signer.to_bytes().as_slice()).unwrap();
        drop(signer);
        let signer = cloned;
        assert_eq!(restored.address(), signer.address());
        assert_eq!(restored.sign_hash(&hash).await.unwrap(), expected);
        let random = EvmSigner::random();
        assert_eq!(
            EvmSigner::from_slice(random.to_bytes().as_slice())
                .unwrap()
                .address(),
            random.address()
        );
    }

    #[test]
    fn rejects_invalid_secp256k1_scalars_without_disclosing_input() {
        for key in [
            "00".repeat(32),
            "ff".repeat(32),
            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141".into(),
            "11".repeat(31),
            "not-a-key".into(),
        ] {
            let error = key.parse::<EvmSigner>().err().unwrap();
            assert_eq!(error.to_string(), "invalid EVM private key");
        }
        assert!(EvmSigner::from_slice(&[0; 32]).is_err());
    }
}
