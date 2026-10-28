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
                    tracing::debug!(provider_index=index+1, failover,
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
        self.rpc("eth_chainId", json!([]), |value| {
            ensure!(quantity(&value)? == 8453, "wrong Base chain");
            Ok(((), true))
        })
        .await?;
        let (latest, timestamp) = self.block("latest").await?;
        let confirmed = self
            .rpc(
                "eth_getTransactionReceipt",
                json!([expected.transaction.to_string()]),
                |receipt| {
                    if receipt.is_null() {
                        return Ok((None, true));
                    }
                    let height = quantity(&receipt["blockNumber"])?;
                    if latest
                        .height
                        .checked_sub(self.confirmations)
                        .is_none_or(|confirmed_height| height > confirmed_height)
                    {
                        // Pending observations retain their existing result even
                        // when the transaction hash cannot qualify connectivity.
                        let matches_request = hash(&receipt["transactionHash"])
                            .is_ok_and(|transaction| transaction == expected.transaction);
                        return Ok((None, matches_request));
                    }
                    let claimed = Anchor {
                        height,
                        hash: hash(&receipt["blockHash"])?.to_string(),
                    };
                    let proof = validate_receipt(&receipt, expected, &claimed)?;
                    Ok((Some((claimed, proof)), true))
                },
            )
            .await?;
        let Some((claimed, proof)) = confirmed else {
            return Ok(None);
        };
        // A missing or shallow receipt is pending without financial-header work.
        // A qualified receipt still needs the complete finalized/depth bound.
        let headers = self
            .finalized_headers(latest, timestamp, &ViewProgress::new())
            .await?;
        if claimed.height > headers.confirmed.height {
            return Ok(None);
        }
        // Confirmed receipt acceptance validates transfer fields; canonical chain
        // proof remains separate and retains the existing pinned-block rechecks.
        let (block, _) = self.block(&format!("0x{:x}", claimed.height)).await?;
        ensure!(
            block.height == claimed.height,
            "receipt block height mismatch"
        );
        ensure!(
            block.hash == claimed.hash,
            "receipt transaction/block mismatch"
        );
        let mut rechecked = BTreeMap::new();
        let progress = ViewProgress::new();
        for pinned in [
            &block,
            &headers.confirmed,
            &headers.latest,
            &headers.finalized,
        ] {
            let current = self
                .recheck_anchor(
                    pinned.height,
                    &mut rechecked,
                    &progress,
                    "receipt_checkpoint",
                )
                .await?;
            ensure!(
                current.height == pinned.height && current.hash == pinned.hash,
                "Base view changed during reconciliation"
            );
        }
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
        let mut many = v.clone();
        let mut unrelated = v["logs"][1].clone();
        unrelated["address"] = json!(Address::ZERO);
        let mut logs = vec![unrelated; 4097];
        logs.extend(v["logs"].as_array().unwrap().iter().cloned());
        many["logs"] = json!(logs);
        validate_receipt(&many, &e, &b).unwrap();
    }
    #[tokio::test]
    async fn receipt_verification_waits_for_confirmations_and_rechecks_canonical_blocks() {
        use axum::{Json, Router, routing::post};
        for mode in [
            "valid",
            "old",
            "host_future",
            "fallback",
            "pending",
            "unconfirmed",
            "reorg",
            "wrong_chain",
            "wrong_receipt",
            "not_finalized",
            "finality_unavailable",
            "finality_reorg",
            "pending_without_finality",
            "unconfirmed_without_finality",
        ] {
            let (e, b, mut receipt) = fixture();
            if matches!(mode, "unconfirmed" | "unconfirmed_without_finality") {
                receipt["blockNumber"] = json!("0x63");
            }
            if mode == "wrong_receipt" {
                receipt["transactionHash"] = json!(B256::ZERO);
            }
            let stamp = match mode {
                "old" => 1,
                "host_future" => now().unwrap() + 3600,
                _ => now().unwrap() - 5,
            };
            let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = reads.clone();
            let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let requests = requested.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener,Router::new().route("/",post(move |Json(q):Json<Value>| {
                    let receipt = receipt.clone(); let b = b.clone(); let seen = seen.clone(); let requested = requested.clone();
                    async move {
                        requested.lock().unwrap().push(q.clone());
                        let result = match q["method"].as_str().unwrap() {
                            "eth_chainId" => json!(if mode == "wrong_chain" { "0x1" } else { "0x2105" }),
                            "eth_getTransactionReceipt" => if matches!(mode, "pending" | "pending_without_finality") { Value::Null } else { receipt },
                            "eth_getBlockByNumber" => {
                                let tag = q["params"][0].as_str().unwrap();
                                if tag == "finalized" && matches!(mode, "finality_unavailable" | "pending_without_finality" | "unconfirmed_without_finality") {
                                    return Json(json!({"jsonrpc":"2.0","id":1,"result":null}));
                                }
                                let height = match tag {
                                    "latest" => 100,
                                    "finalized" if mode == "not_finalized" => 70,
                                    "finalized" if mode == "finality_reorg" => 90,
                                    "finalized" => 100,
                                    _ => u64::from_str_radix(&tag[2..],16).unwrap(),
                                };
                                let changed = height == 80 && seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst)>0 && mode == "reorg";
                                json!({"number":format!("0x{height:x}"),
                                    "hash":if height == 80 {if changed {B256::ZERO.to_string()} else {b.hash}} else if tag == "0x5a" && mode == "finality_reorg" {B256::ZERO.to_string()} else {format!("0x{height:064x}")},
                                    "timestamp":format!("0x{stamp:x}")})
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
                BaseRpc::with_fallbacks(&[refused_url, url], 12).unwrap()
            } else {
                BaseRpc::new(&url, 12).unwrap()
            };
            let result = rpc.verify_transfer(&e).await;
            match mode {
                "valid" | "old" | "host_future" | "fallback" => {
                    assert_eq!(result.unwrap().unwrap().amount_atomic, "7000")
                }
                "pending"
                | "unconfirmed"
                | "not_finalized"
                | "pending_without_finality"
                | "unconfirmed_without_finality" => assert!(result.unwrap().is_none()),
                _ => assert!(result.is_err(), "{mode}"),
            }
            if matches!(
                mode,
                "pending_without_finality" | "unconfirmed_without_finality"
            ) {
                let requests = requests.lock().unwrap();
                assert_eq!(
                    requests.len(),
                    3,
                    "pending receipts need no financial header acquisition"
                );
                assert_eq!(requests[1]["params"][0], "latest");
                assert_eq!(requests[2]["method"], "eth_getTransactionReceipt");
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn finalized_receipt_fallback_reacquires_the_receipt_on_its_own_endpoint() {
        use axum::{Json, Router, routing::post};
        for healthy_fallback in [true, false] {
            let (expected, block, receipt) = fixture();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = requests.clone();
            let app = Router::new().route("/{endpoint}",post(move |uri:axum::http::Uri,Json(q):Json<Value>| {
                let (block,mut receipt,requests)=(block.clone(),receipt.clone(),requests.clone());
                async move {
                    requests.lock().unwrap().push((uri.path().to_owned(),q["method"].as_str().unwrap().to_owned()));
                    let result=match q["method"].as_str().unwrap() {
                        "eth_chainId"=>json!("0x2105"),
                        "eth_getTransactionReceipt"=>{
                            if uri.path()=="/fallback" && !healthy_fallback {receipt["transactionHash"]=json!(B256::ZERO);}
                            receipt
                        },
                        "eth_getBlockByNumber"=>{
                            let tag=q["params"][0].as_str().unwrap();
                            if uri.path()=="/primary" && tag=="finalized" {
                                return Json(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"private unsupported tag"}}));
                            }
                            let height=match tag {"latest"=>100,"finalized"=>80,_=>u64::from_str_radix(&tag[2..],16).unwrap()};
                            json!({"number":format!("0x{height:x}"),"hash":if height==80 {block.hash} else {format!("0x{height:064x}")},"timestamp":format!("0x{height:x}")})
                        },
                        method=>panic!("unexpected RPC {method}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let rpc = BaseRpc::with_fallbacks(
                &[
                    format!("http://{address}/primary"),
                    format!("http://{address}/fallback"),
                ],
                12,
            )
            .unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let result = rpc.verify_transfer(&expected).await;
            if healthy_fallback {
                assert_eq!(result.unwrap().unwrap().amount_atomic, "7000");
            } else {
                assert!(
                    result.is_err(),
                    "a valid first receipt cannot qualify the fallback's invalid receipt"
                );
            }
            let requests = observed.lock().unwrap();
            for path in ["/primary", "/fallback"] {
                assert_eq!(
                    requests
                        .iter()
                        .filter(|(p, method)| p == path && method == "eth_getTransactionReceipt")
                        .count(),
                    1
                );
            }
            server.abort();
        }
    }
}
