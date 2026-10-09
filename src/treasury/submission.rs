//! Explicit raw gRPC sender. No upstream transmit helper, fallback endpoint or retries.
use crate::rotation::{
    base::secure_endpoint,
    transaction::{
        BroadcastTransaction, PreparedTransaction, SubmissionOutcome, TransactionPresence,
        TransactionSubmission,
    },
};
use anyhow::{Context, Result, ensure};
use std::{io::Cursor, time::Duration};
use zcash_primitives::transaction::{Transaction, TxVersion};
use zcash_protocol::consensus::BranchId;
use zingo_netutils::{
    GrpcIndexer, Indexer,
    lightwallet_protocol::{BlockId, RawTransaction, TxFilter},
};
fn request_timeout() -> Duration {
    Duration::from_secs(15)
}

pub struct GrpcSubmission {
    network: crate::rotation::store::TreasuryNetwork,
    submission: String,
    indexer: String,
    ready: Option<(String, GrpcIndexer, u64)>,
}
impl GrpcSubmission {
    pub fn new(submission: String, indexer: String) -> Result<Self> {
        Self::with_network(
            submission,
            indexer,
            crate::rotation::store::TreasuryNetwork::Mainnet,
        )
    }
    pub(crate) fn with_network(
        submission: String,
        indexer: String,
        network: crate::rotation::store::TreasuryNetwork,
    ) -> Result<Self> {
        secure_endpoint(&submission)?;
        secure_endpoint(&indexer)?;
        Ok(Self {
            network,
            submission,
            indexer,
            ready: None,
        })
    }
    async fn connect(
        &self,
        endpoint: &str,
        identity: &crate::network::IsolationId,
    ) -> Result<GrpcIndexer> {
        let mut client = crate::network::global().grpc(identity, endpoint).await?;
        let info = client
            .get_lightd_info(request_timeout())
            .await
            .map_err(|_| anyhow::anyhow!("treasury network check failed"))?;
        ensure!(
            info.chain_name == self.network.rpc_name(),
            "treasury endpoint is not on mainnet"
        );
        Ok(client)
    }
}
pub(crate) fn decode(raw: &[u8]) -> Result<Transaction> {
    let mut reader = Cursor::new(raw);
    let transaction =
        Transaction::read(&mut reader, BranchId::Nu5).context("invalid durable transaction")?;
    ensure!(
        reader.position() == raw.len() as u64
            && matches!(transaction.version(), TxVersion::V5 | TxVersion::V6),
        "unsupported durable transaction encoding"
    );
    Ok(transaction)
}
impl TransactionSubmission for GrpcSubmission {
    async fn preflight(
        &mut self,
        transaction: &PreparedTransaction,
        rebroadcast: bool,
    ) -> Result<()> {
        self.ready = None;
        let raw = decode(transaction.bytes())?;
        let facts = transaction
            .facts()
            .context("durable transaction metadata missing")?;
        ensure!(
            facts.txid == raw.txid().to_string()
                && facts.expiry_height == u32::from(raw.expiry_height()),
            "durable transaction identity mismatch"
        );
        let mut client = self
            .connect(&self.submission, &transaction.network_identity()?)
            .await?;
        let tip = client
            .get_latest_block(request_timeout())
            .await
            .map_err(|_| anyhow::anyhow!("treasury tip unavailable"))?;
        ensure!(
            tip.height < u64::from(facts.expiry_height),
            "transaction_expired"
        );
        ensure!(
            facts
                .deadline
                .checked_sub(crate::rotation::base::now()?)
                .is_some_and(|remaining| remaining
                    >= if rebroadcast {
                        1
                    } else {
                        crate::rotation::transaction::MIN_QUOTE_VALIDITY_SECONDS
                    }),
            "funding_deadline_expired"
        );
        self.ready = Some((transaction.operation_id().to_owned(), client, tip.height));
        Ok(())
    }
    async fn submit(&mut self, transaction: BroadcastTransaction) -> Result<SubmissionOutcome> {
        let first_attempt = transaction.attempt() == 1;
        let transaction = transaction.transaction();
        let _identity = transaction.network_identity()?;
        let raw = decode(transaction.bytes())?;
        let expected = raw.txid().to_string();
        let facts = transaction
            .facts()
            .context("durable transaction metadata missing")?;
        ensure!(
            facts.txid == expected && facts.expiry_height == u32::from(raw.expiry_height()),
            "durable transaction identity mismatch"
        );
        let result: Result<_> = async {
            // Normal treasury submission preflights before minting intent. Keep
            // the legacy direct trait entry conservative if it has no preflight.
            if !self
                .ready
                .as_ref()
                .is_some_and(|(id, _, _)| id == transaction.operation_id())
            {
                self.preflight(transaction, !first_attempt).await?;
            }
            let (_, mut client, tip) = self.ready.take().context("submission preflight missing")?;
            ensure!(tip < u64::from(facts.expiry_height), "transaction_expired");
            // Network readiness completed before durable intent. A second unsigned
            // tip read here could strand a never-sent operation as UNKNOWN.
            // Retain immediate local expiry checks without new I/O.
            ensure!(
                facts
                    .deadline
                    .checked_sub(crate::rotation::base::now()?)
                    .is_some_and(|remaining| remaining
                        >= if first_attempt {
                            crate::rotation::transaction::MIN_QUOTE_VALIDITY_SECONDS
                        } else {
                            1
                        }),
                "funding_deadline_expired"
            );
            client
                .send_transaction(
                    RawTransaction {
                        data: transaction.bytes().to_vec(),
                        height: 0,
                    },
                    request_timeout(),
                )
                .await
                .map_err(|_| anyhow::anyhow!("submission outcome unknown"))
        }
        .await;
        Ok(match result {
            Ok(id) if id.eq_ignore_ascii_case(&expected) => SubmissionOutcome::Accepted,
            _ => SubmissionOutcome::Unknown,
        })
    }
    async fn lookup(&mut self, transaction: &PreparedTransaction) -> Result<TransactionPresence> {
        let _identity = transaction.network_identity()?;
        let raw = decode(transaction.bytes())?;
        let hash = raw.txid().as_ref().to_vec();
        let result: Result<_> = async {
            let mut client = self
                .connect(&self.indexer, &transaction.network_identity()?)
                .await?;
            let found = match client
                .get_transaction(
                    TxFilter {
                        hash: hash.clone(),
                        ..Default::default()
                    },
                    request_timeout(),
                )
                .await
            {
                Ok(found) => found,
                Err(error) if error.code() == tonic::Code::NotFound => {
                    return Ok(TransactionPresence::Absent);
                }
                Err(_) => return Ok(TransactionPresence::Unknown),
            };
            ensure!(
                found.data == transaction.bytes(),
                "lookup returned conflicting bytes"
            );
            if found.height == 0 {
                return Ok(TransactionPresence::Mempool);
            }
            ensure!(
                found.height <= u64::from(u32::MAX),
                "transaction is not on the main chain"
            );
            let block = client
                .get_block(
                    BlockId {
                        height: found.height,
                        hash: vec![],
                    },
                    request_timeout(),
                )
                .await
                .map_err(|_| anyhow::anyhow!("confirmation block unavailable"))?;
            ensure!(
                block.height == found.height
                    && block.hash.len() == 32
                    && block.vtx.iter().any(|t| t.txid == hash),
                "transaction absent from confirmation block"
            );
            let check = client
                .get_block(
                    BlockId {
                        height: found.height,
                        hash: vec![],
                    },
                    request_timeout(),
                )
                .await
                .map_err(|_| anyhow::anyhow!("confirmation recheck unavailable"))?;
            ensure!(check.hash == block.hash, "confirmation block changed");
            Ok(TransactionPresence::Confirmed {
                height: found.height,
            })
        }
        .await;
        Ok(match result {
            Ok(presence) => presence,
            _ => TransactionPresence::Unknown,
        })
    }
}
