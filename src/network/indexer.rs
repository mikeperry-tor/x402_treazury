//! RPC adaptation over the application-owned progress transport.
//
// Adapted from Zingolabs zingolib/zingo-netutils revision
// bfb0a2156819ea7f75a9b09b1724abfd06c8925e (v7.0.0).
// Copyright (c) 2022-present Zingolabs. MIT license: vendor/ZINGO-LICENSE-MIT.
use std::time::Duration;
use tonic::{Request, codegen::http};
use zingo_netutils::lightwallet_protocol::{
    Address, AddressList, Balance, BlockId, BlockRange, ChainSpec, CompactBlock, CompactTx,
    CompactTxStreamerClient, Empty, GetAddressUtxosArg, GetAddressUtxosReply,
    GetAddressUtxosReplyList, GetMempoolTxRequest, GetSubtreeRootsArg, LightdInfo, RawTransaction,
    SubtreeRoot, TransparentAddressBlockFilter, TreeState, TxFilter,
};
use zingo_netutils::{Indexer, TransparentIndexer};

pub(super) type IndexerTransport = tower::util::BoxCloneSyncService<
    http::Request<tonic::body::Body>,
    http::Response<tonic::body::Body>,
    tonic::Status,
>;

#[derive(Clone, Debug)]
pub struct GrpcIndexer {
    pub(super) clear_net_client: CompactTxStreamerClient<IndexerTransport>,
}

// Preserve the existing gRPC submission contract: error codes become Unknown
// with the bare server message; a successful message is the optionally quoted
// transaction ID. Submission validates that ID against the durable transaction.
fn parse_send_response(error_code: i32, message: String) -> Result<String, tonic::Status> {
    if error_code != 0 {
        return Err(tonic::Status::unknown(message));
    }
    if message.len() >= 2 && message.starts_with('"') && message.ends_with('"') {
        Ok(message[1..message.len() - 1].to_owned())
    } else {
        Ok(message)
    }
}

// The traits retain legacy total-timeout arguments. ProgressChannel owns response
// inactivity and strips total deadlines; these wrappers do not create them.
impl Indexer for GrpcIndexer {
    async fn get_lightd_info(&mut self, _timeout: Duration) -> Result<LightdInfo, tonic::Status> {
        let request = Request::new(Empty {});
        Ok(self
            .clear_net_client
            .get_lightd_info(request)
            .await?
            .into_inner())
    }

    async fn get_latest_block(&mut self, _timeout: Duration) -> Result<BlockId, tonic::Status> {
        let request = Request::new(ChainSpec {});
        Ok(self
            .clear_net_client
            .get_latest_block(request)
            .await?
            .into_inner())
    }

    async fn send_transaction(
        &mut self,
        tx: RawTransaction,
        _timeout: Duration,
    ) -> Result<String, tonic::Status> {
        let request = Request::new(tx);
        let sendresponse = self
            .clear_net_client
            .send_transaction(request)
            .await?
            .into_inner();
        parse_send_response(sendresponse.error_code, sendresponse.error_message)
    }

    async fn get_tree_state(
        &mut self,
        block_id: BlockId,
        _timeout: Duration,
    ) -> Result<TreeState, tonic::Status> {
        let request = Request::new(block_id);
        Ok(self
            .clear_net_client
            .get_tree_state(request)
            .await?
            .into_inner())
    }

    async fn get_block(
        &mut self,
        block_id: BlockId,
        _timeout: Duration,
    ) -> Result<CompactBlock, tonic::Status> {
        let request = Request::new(block_id);
        Ok(self.clear_net_client.get_block(request).await?.into_inner())
    }

    #[allow(deprecated)]
    async fn get_block_nullifiers(
        &mut self,
        block_id: BlockId,
        _timeout: Duration,
    ) -> Result<CompactBlock, tonic::Status> {
        let request = Request::new(block_id);
        Ok(self
            .clear_net_client
            .get_block_nullifiers(request)
            .await?
            .into_inner())
    }

    async fn get_block_range(
        &mut self,
        range: BlockRange,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<CompactBlock>, tonic::Status> {
        let request = Request::new(range);
        Ok(self
            .clear_net_client
            .get_block_range(request)
            .await?
            .into_inner())
    }

    #[allow(deprecated)]
    async fn get_block_range_nullifiers(
        &mut self,
        range: BlockRange,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<CompactBlock>, tonic::Status> {
        let request = Request::new(range);
        Ok(self
            .clear_net_client
            .get_block_range_nullifiers(request)
            .await?
            .into_inner())
    }

    async fn get_transaction(
        &mut self,
        filter: TxFilter,
        _timeout: Duration,
    ) -> Result<RawTransaction, tonic::Status> {
        let request = Request::new(filter);
        Ok(self
            .clear_net_client
            .get_transaction(request)
            .await?
            .into_inner())
    }

    async fn get_mempool_tx(
        &mut self,
        request: GetMempoolTxRequest,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<CompactTx>, tonic::Status> {
        let request = Request::new(request);
        Ok(self
            .clear_net_client
            .get_mempool_tx(request)
            .await?
            .into_inner())
    }

    async fn get_mempool_stream(
        &mut self,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<RawTransaction>, tonic::Status> {
        let request = Request::new(Empty {});
        Ok(self
            .clear_net_client
            .get_mempool_stream(request)
            .await?
            .into_inner())
    }

    async fn get_latest_tree_state(
        &mut self,
        _timeout: Duration,
    ) -> Result<TreeState, tonic::Status> {
        let request = Request::new(Empty {});
        Ok(self
            .clear_net_client
            .get_latest_tree_state(request)
            .await?
            .into_inner())
    }

    async fn get_subtree_roots(
        &mut self,
        arg: GetSubtreeRootsArg,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<SubtreeRoot>, tonic::Status> {
        let request = Request::new(arg);
        Ok(self
            .clear_net_client
            .get_subtree_roots(request)
            .await?
            .into_inner())
    }
}

impl TransparentIndexer for GrpcIndexer {
    #[allow(deprecated)]
    async fn get_taddress_txids(
        &mut self,
        filter: TransparentAddressBlockFilter,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<RawTransaction>, tonic::Status> {
        let request = Request::new(filter);
        Ok(self
            .clear_net_client
            .get_taddress_txids(request)
            .await?
            .into_inner())
    }

    async fn get_taddress_transactions(
        &mut self,
        filter: TransparentAddressBlockFilter,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<RawTransaction>, tonic::Status> {
        let request = Request::new(filter);
        Ok(self
            .clear_net_client
            .get_taddress_transactions(request)
            .await?
            .into_inner())
    }

    async fn get_taddress_balance(
        &mut self,
        addresses: AddressList,
        _timeout: Duration,
    ) -> Result<Balance, tonic::Status> {
        let request = Request::new(addresses);
        Ok(self
            .clear_net_client
            .get_taddress_balance(request)
            .await?
            .into_inner())
    }

    async fn get_taddress_balance_stream(
        &mut self,
        addresses: Vec<Address>,
    ) -> Result<Balance, tonic::Status> {
        let stream = tokio_stream::iter(addresses);
        Ok(self
            .clear_net_client
            .get_taddress_balance_stream(stream)
            .await?
            .into_inner())
    }

    async fn get_address_utxos(
        &mut self,
        arg: GetAddressUtxosArg,
        _timeout: Duration,
    ) -> Result<GetAddressUtxosReplyList, tonic::Status> {
        let request = Request::new(arg);
        Ok(self
            .clear_net_client
            .get_address_utxos(request)
            .await?
            .into_inner())
    }

    async fn get_address_utxos_stream(
        &mut self,
        arg: GetAddressUtxosArg,
        _timeout: Duration,
    ) -> Result<tonic::Streaming<GetAddressUtxosReply>, tonic::Status> {
        let request = Request::new(arg);
        Ok(self
            .clear_net_client
            .get_address_utxos_stream(request)
            .await?
            .into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_send_response;

    #[test]
    fn successful_submission_preserves_bare_and_safely_unquotes_paired_txids() {
        for (message, expected) in [
            ("deadbeef", "deadbeef"),
            ("\"deadbeef\"", "deadbeef"),
            ("\"", "\""),
            ("\"\"", ""),
            ("", ""),
        ] {
            assert_eq!(
                parse_send_response(0, message.to_owned()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rejected_submission_preserves_unknown_status_and_bare_server_message() {
        for code in [-25, 1] {
            let error = parse_send_response(code, "failed to validate".to_owned()).unwrap_err();
            assert_eq!(error.code(), tonic::Code::Unknown);
            assert_eq!(error.message(), "failed to validate");
        }
    }
}
