//! Cache identity retains the exact authority used by a shared document fetch.
use super::*;
use std::cmp::Ordering;

#[derive(Clone)]
pub(crate) enum Authority {
    Unsigned,
    Static(Arc<Payer>),
    Managed(Arc<crate::rotation::manager::ManagedPool>),
}
impl Authority {
    fn key(&self) -> (u8, usize) {
        match self {
            Self::Unsigned => (0, 0),
            Self::Static(payer) => (1, Arc::as_ptr(payer) as usize),
            Self::Managed(pool) => (2, Arc::as_ptr(pool) as usize),
        }
    }
}
impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsigned => "Unsigned",
            Self::Static(_) => "Static",
            Self::Managed(..) => "Managed",
        })
    }
}
impl PartialEq for Authority {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for Authority {}
impl PartialOrd for Authority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Authority {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

impl PaidClient {
    pub(crate) fn with_metadata_payer(mut self, payer: Arc<Self>) -> Self {
        self.metadata_payer = Some(payer);
        self
    }

    pub(crate) fn has_metadata_payer(&self) -> bool {
        self.metadata_payer.is_some()
    }

    pub(crate) fn relay_authority(&self) -> Self {
        let purpose = self.relay_purpose;
        let mut client = self.clone();
        if !matches!(purpose, crate::discovery_relay::Purpose::ApiRequests)
            && let Some(payer) = &self.metadata_payer
        {
            client.payer = payer.payer.clone();
            client.managed = payer.managed.clone();
            client.cover = payer.cover.clone();
            client.unavailable = payer.unavailable;
            client.unsigned_only = payer.unsigned_only;
            client.bootstrap = None;
            client.bootstrap_demand = None;
        }
        client.metadata_payer = None;
        client.relay_purpose = purpose;
        client
    }

    pub(crate) fn pin_document_authority(&self) -> (Self, Authority) {
        let mut client = self
            .clone()
            .with_relay_purpose(crate::discovery_relay::Purpose::ProviderMetadata)
            .relay_authority();
        let authority = if let Some(payer) = &client.payer {
            let payer = payer.read().expect("payer lock poisoned").clone();
            client.payer = Some(Arc::new(RwLock::new(payer.clone())));
            Authority::Static(payer)
        } else if let Some(pool) = &client.managed {
            Authority::Managed(pool.clone())
        } else {
            Authority::Unsigned
        };
        (client, authority)
    }

    /// Pointer identity is used only while a binding retains this client's Arc.
    pub(crate) fn document_policy_key(&self) -> (bool, Option<usize>) {
        (
            self.public_only,
            self.document_relay
                .as_ref()
                .map(|relay| Arc::as_ptr(relay) as usize),
        )
    }
    pub(crate) fn with_document_relay(
        mut self,
        relay: Option<Arc<crate::discovery_relay::SearchRelay>>,
    ) -> Self {
        self.document_relay = relay;
        self
    }
    pub(crate) fn with_relay_purpose(mut self, purpose: crate::discovery_relay::Purpose) -> Self {
        self.relay_purpose = purpose;
        self
    }
    pub(crate) fn request_relay(&self) -> Option<Arc<crate::discovery_relay::SearchRelay>> {
        self.document_relay
            .clone()
            .filter(|relay| relay.enabled(self.relay_purpose))
    }
    pub(crate) fn has_document_authority(&self) -> bool {
        !self.unsigned_only
            && self.unavailable.is_none()
            && (self.payer.is_some() || self.managed.is_some())
    }
    pub(crate) fn response_limit(&self) -> usize {
        self.max_response_bytes
    }
    pub(crate) fn document_relay(&self) -> Option<Arc<crate::discovery_relay::SearchRelay>> {
        self.document_relay.clone()
    }
}
