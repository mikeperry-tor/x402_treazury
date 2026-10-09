//! Canonical Base USDC transfer evidence, including the EIP-3009 nonce event.
//! Event definition: https://eips.ethereum.org/EIPS/eip-3009#event
use super::*;
#[derive(Clone, Debug)]
pub struct TransferExpectation {
    pub transaction: B256,
    pub payer: Address,
    pub payee: Address,
    pub amount: U256,
    pub nonce: B256,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct TransferProof {
    pub transaction: String,
    pub block_height: u64,
    pub block_hash: String,
    pub payer: String,
    pub payee: String,
    pub amount_atomic: String,
    pub nonce: String,
}
impl BaseRpc {
    /// Read-only, payer-isolated verification. Missing/unconfirmed receipts remain
    /// pending. Provider failures may use only the configured RPC fallbacks.
    pub async fn verify_transfer(
        &self,
        expected: &TransferExpectation,
    ) -> Result<Option<TransferProof>> {
        ensure!(expected.amount > U256::ZERO, "zero expected payment amount");
        for (index, endpoint) in std::iter::once(self).chain(&self.fallbacks).enumerate() {
            let client = endpoint.for_address(&expected.payer.to_string())?;
            let result = client.transfer_inner(expected).await;
            match result {
                Ok(proof) => return Ok(proof),
                Err(error) => {
                    let error = error.context(VerificationStage("receipt transaction"));
                    let failover = index < self.fallbacks.len()
                        && error
                            .downcast_ref::<RpcFailure>()
                            .is_some_and(RpcFailure::can_failover);
                    tracing::warn!(provider_index=index+1, failover,
                        category=%safe_diagnostic(&error),
                        "receipt chain verification failed; no payment outcome inferred");
                    if !failover {
                        return Err(error);
                    }
                }
            }
        }
        unreachable!("primary RPC always exists")
    }
    async fn transfer_inner(
        &self,
        expected: &TransferExpectation,
    ) -> Result<Option<TransferProof>> {
        ensure!(
            quantity(&self.rpc("eth_chainId", json!([])).await?)? == 8453,
            "wrong Base chain"
        );
        let (latest, stamp) = self.block("latest").await?;
        validate_timestamp(stamp, now()?, self.max_age)?;
        let receipt = self
            .rpc(
                "eth_getTransactionReceipt",
                json!([expected.transaction.to_string()]),
            )
            .await?;
        if receipt.is_null() {
            return Ok(None);
        }
        let height = quantity(&receipt["blockNumber"])?;
        let Some(confirmed_height) = latest.height.checked_sub(self.confirmations) else {
            return Ok(None);
        };
        if height > confirmed_height {
            return Ok(None);
        }
        let (block, _) = self.block(&format!("0x{height:x}")).await?;
        ensure!(block.height == height, "receipt block height mismatch");
        let proof = validate_receipt(&receipt, expected, &block)?;
        for pinned in [&block, &latest] {
            let (current, _) = self.block(&format!("0x{:x}", pinned.height)).await?;
            ensure!(
                current.height == pinned.height && current.hash == pinned.hash,
                "Base view changed during reconciliation"
            );
        }
        validate_timestamp(stamp, now()?, self.max_age)?;
        Ok(Some(proof))
    }
}
fn hash(value: &Value) -> Result<B256> {
    value
        .as_str()
        .context("receipt hash missing")?
        .parse()
        .context("receipt hash malformed")
}
fn address(value: &Value) -> Result<Address> {
    value
        .as_str()
        .context("receipt address missing")?
        .parse()
        .context("receipt address malformed")
}
fn validate_receipt(
    value: &Value,
    expected: &TransferExpectation,
    block: &Anchor,
) -> Result<TransferProof> {
    ensure!(
        quantity(&value["status"])? == 1,
        "receipt transaction reverted"
    );
    ensure!(
        hash(&value["transactionHash"])? == expected.transaction
            && hash(&value["blockHash"])? == block.hash.parse::<B256>()?
            && quantity(&value["blockNumber"])? == block.height,
        "receipt transaction/block mismatch"
    );
    let logs = value["logs"].as_array().context("receipt logs missing")?;
    if logs.len() > 4096 {
        tracing::warn!(
            limit = 4096,
            actual = logs.len(),
            "receipt log limit exceeded; evidence rejected without truncation"
        );
        anyhow::bail!("receipt log limit exceeded");
    }
    let token: Address = USDC.parse()?;
    let transfer = keccak256("Transfer(address,address,uint256)");
    let authorized = keccak256("AuthorizationUsed(address,bytes32)");
    let payer = B256::left_padding_from(expected.payer.as_slice());
    let payee = B256::left_padding_from(expected.payee.as_slice());
    let mut transfers = 0;
    let mut authorizations = 0;
    for log in logs {
        if address(&log["address"])? != token {
            continue;
        }
        ensure!(
            log["removed"] != true
                && hash(&log["transactionHash"])? == expected.transaction
                && hash(&log["blockHash"])? == block.hash.parse::<B256>()?
                && quantity(&log["blockNumber"])? == block.height,
            "receipt log provenance mismatch"
        );
        let topics = log["topics"]
            .as_array()
            .context("receipt log topics missing")?;
        let Some(first) = topics.first() else {
            continue;
        };
        let first = hash(first)?;
        if first != transfer && first != authorized {
            continue;
        }
        ensure!(topics.len() == 3, "receipt event topics malformed");
        if first == transfer && hash(&topics[1])? == payer && hash(&topics[2])? == payee {
            let data = hash(&log["data"])?;
            if U256::from_be_bytes(data.0) == expected.amount {
                transfers += 1;
            }
        }
        if first == authorized && hash(&topics[1])? == payer && hash(&topics[2])? == expected.nonce
        {
            ensure!(log["data"] == "0x", "authorization event data malformed");
            authorizations += 1;
        }
    }
    ensure!(
        transfers == 1 && authorizations == 1,
        "receipt lacks unique matching USDC transfer and authorization nonce"
    );
    Ok(TransferProof {
        transaction: expected.transaction.to_string(),
        block_height: block.height,
        block_hash: block.hash.clone(),
        payer: expected.payer.to_string(),
        payee: expected.payee.to_string(),
        amount_atomic: expected.amount.to_string(),
        nonce: expected.nonce.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (TransferExpectation, Anchor, Value) {
        let e = TransferExpectation {
            transaction: B256::repeat_byte(1),
            payer: Address::repeat_byte(2),
            payee: Address::repeat_byte(3),
            amount: U256::from(7000),
            nonce: B256::repeat_byte(4),
        };
        let block = Anchor {
            height: 80,
            hash: B256::repeat_byte(5).to_string(),
        };
        let log = |topics: Vec<B256>, data: String| {
            json!({
                "address":USDC,"removed":false,"transactionHash":e.transaction,
                "blockHash":block.hash,"blockNumber":"0x50","topics":topics,"data":data
            })
        };
        let transfer = log(
            vec![
                keccak256("Transfer(address,address,uint256)"),
                B256::left_padding_from(e.payer.as_slice()),
                B256::left_padding_from(e.payee.as_slice()),
            ],
            format!("0x{:064x}", e.amount),
        );
        let authorization = log(
            vec![
                keccak256("AuthorizationUsed(address,bytes32)"),
                B256::left_padding_from(e.payer.as_slice()),
                e.nonce,
            ],
            "0x".into(),
        );
        let receipt = json!({"transactionHash":e.transaction,"blockHash":block.hash,
            "blockNumber":"0x50","status":"0x1","logs":[authorization,transfer]});
        (e, block, receipt)
    }
    #[test]
    fn exact_transfer_nonce_and_log_provenance_are_required() {
        let (e, b, v) = fixture();
        assert_eq!(validate_receipt(&v, &e, &b).unwrap().amount_atomic, "7000");
        for (path, value) in [
            ("/status", json!("0x0")),
            ("/transactionHash", json!(B256::ZERO)),
            ("/blockHash", json!(B256::ZERO)),
            (
                "/logs/0/topics/0",
                json!(keccak256("AuthorizationCanceled(address,bytes32)")),
            ),
            ("/logs/0/topics/2", json!(B256::ZERO)),
            ("/logs/0/data", json!("0x00")),
            ("/logs/1/topics/1", json!(B256::ZERO)),
            ("/logs/1/topics/2", json!(B256::ZERO)),
            ("/logs/1/data", json!(format!("0x{:064x}", 7001))),
            ("/logs/1/address", json!(Address::ZERO)),
            ("/logs/1/removed", json!(true)),
            ("/logs/1/blockHash", json!(B256::ZERO)),
            ("/logs/1/transactionHash", json!(B256::ZERO)),
        ] {
            let mut changed = v.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(validate_receipt(&changed, &e, &b).is_err(), "{path}");
        }
        let mut duplicate = v.clone();
        duplicate["logs"]
            .as_array_mut()
            .unwrap()
            .push(v["logs"][1].clone());
        assert!(validate_receipt(&duplicate, &e, &b).is_err());
        let mut excessive = v.clone();
        excessive["logs"] = json!(vec![v["logs"][1].clone(); 4097]);
        assert!(
            validate_receipt(&excessive, &e, &b)
                .unwrap_err()
                .to_string()
                .contains("log limit")
        );
    }
    #[tokio::test]
    async fn receipt_verification_waits_for_confirmations_and_rechecks_canonical_blocks() {
        use axum::{Json, Router, routing::post};
        for mode in [
            "valid",
            "fallback",
            "pending",
            "unconfirmed",
            "reorg",
            "wrong_chain",
            "wrong_receipt",
        ] {
            let (e, b, mut receipt) = fixture();
            if mode == "unconfirmed" {
                receipt["blockNumber"] = json!("0x63");
            }
            if mode == "wrong_receipt" {
                receipt["transactionHash"] = json!(B256::ZERO);
            }
            let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = reads.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener,Router::new().route("/",post(move |Json(q):Json<Value>| {
                    let receipt = receipt.clone(); let b = b.clone(); let seen = seen.clone();
                    async move {
                        let result = match q["method"].as_str().unwrap() {
                            "eth_chainId" => json!(if mode == "wrong_chain" { "0x1" } else { "0x2105" }),
                            "eth_getTransactionReceipt" => if mode == "pending" { Value::Null } else { receipt },
                            "eth_getBlockByNumber" => {
                                let latest = q["params"][0] == "latest" || q["params"][0] == "0x64";
                                let changed = !latest && seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst)>0 && mode == "reorg";
                                json!({"number":if latest {"0x64"} else {"0x50"},
                                    "hash":if latest {B256::repeat_byte(6).to_string()} else if changed {B256::ZERO.to_string()} else {b.hash},
                                    "timestamp":format!("0x{:x}",now().unwrap()-5)})
                            },
                            method => panic!("unexpected RPC {method}"),
                        };
                        Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
                    }
                }))).await.unwrap();
            });
            let rpc = if mode == "fallback" {
                let refused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let refused_url = format!("http://{}", refused.local_addr().unwrap());
                drop(refused);
                BaseRpc::with_fallbacks(&[refused_url, url], 12, 120).unwrap()
            } else {
                BaseRpc::new(&url, 12, 120).unwrap()
            };
            let result = rpc.verify_transfer(&e).await;
            match mode {
                "valid" | "fallback" => assert_eq!(result.unwrap().unwrap().amount_atomic, "7000"),
                "pending" | "unconfirmed" => assert!(result.unwrap().is_none()),
                _ => assert!(result.is_err(), "{mode}"),
            }
            server.abort();
        }
    }
}
