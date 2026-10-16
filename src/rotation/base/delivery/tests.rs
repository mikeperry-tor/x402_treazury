use super::*;
fn fixture() -> (FundingDeliveryExpectation, EvidenceCapture, Anchor, Value) {
    let expected = FundingDeliveryExpectation {
        operation: uuid::Uuid::new_v4(),
        quote_digest: [7; 32],
        source_transaction: B256::repeat_byte(8),
        transaction: B256::repeat_byte(1),
        recipient: Address::repeat_byte(3),
        amount: U256::from(7000),
    };
    let capture = EvidenceCapture {
        capacity_snapshot: None,
        namespace: uuid::Uuid::new_v4(),
        wallet: "recipient-wallet".into(),
        generation: 4,
        endpoint_generations: BTreeMap::new(),
        checkpoint: None,
        previous: None,
        previous_confirmed_covered: false,
        durable_balance_anchors: vec![],
    };
    let block = Anchor {
        height: 80,
        hash: B256::repeat_byte(5).to_string(),
    };
    let log = json!({"address":USDC,"removed":false,"transactionHash":expected.transaction,
        "blockHash":block.hash,"blockNumber":"0x50","logIndex":"0x2",
        "topics":[keccak256("Transfer(address,address,uint256)"),B256::ZERO,B256::left_padding_from(expected.recipient.as_slice())],
        "data":format!("0x{:064x}", expected.amount)});
    let receipt = json!({"transactionHash":expected.transaction,"blockHash":block.hash,
        "blockNumber":"0x50","status":"0x1","logs":[log]});
    (expected, capture, block, receipt)
}
#[test]
fn event_requires_exact_asset_recipient_amount_and_unique_provenance() {
    let (e, _, b, v) = fixture();
    assert_eq!(validate_delivery(&v, &e, &b).unwrap(), 2);
    for (path, value) in [
        ("/status", json!("0x0")),
        ("/transactionHash", json!(B256::ZERO)),
        ("/blockHash", json!(B256::ZERO)),
        ("/logs/0/address", json!(Address::ZERO)),
        ("/logs/0/topics/2", json!(B256::ZERO)),
        ("/logs/0/data", json!(format!("0x{:064x}", 7001))),
        ("/logs/0/removed", json!(true)),
        ("/logs/0/logIndex", Value::Null),
        ("/logs/0/blockHash", json!(B256::ZERO)),
        ("/logs/0/transactionHash", json!(B256::ZERO)),
    ] {
        let mut bad = v.clone();
        *bad.pointer_mut(path).unwrap() = value;
        assert!(validate_delivery(&bad, &e, &b).is_err(), "{path}");
    }
    let mut duplicate = v.clone();
    duplicate["logs"]
        .as_array_mut()
        .unwrap()
        .push(v["logs"][0].clone());
    assert!(validate_delivery(&duplicate, &e, &b).is_err());
}
#[tokio::test]
async fn canonical_rpc_pins_blocks_confirmations_capture_and_expiring_clone_lease() {
    use axum::{Json, Router, routing::post};
    for mode in [
        "valid",
        "pending",
        "unconfirmed",
        "reorg",
        "saved_reorg",
        "wrong_chain",
        "reverted",
        "stale",
        "wrong_asset",
        "wrong_recipient",
        "wrong_amount",
        "wrong_hash",
        "multiple",
    ] {
        let (e, mut capture, b, mut receipt) = fixture();
        if mode == "unconfirmed" {
            receipt["blockNumber"] = json!("0x63");
        }
        if mode == "reverted" {
            receipt["status"] = json!("0x0");
        }
        match mode {
            "wrong_asset" => receipt["logs"][0]["address"] = json!(Address::ZERO),
            "wrong_recipient" => receipt["logs"][0]["topics"][2] = json!(B256::ZERO),
            "wrong_amount" => receipt["logs"][0]["data"] = json!(format!("0x{:064x}", 6999)),
            "wrong_hash" => receipt["transactionHash"] = json!(B256::ZERO),
            "multiple" => {
                let log = receipt["logs"][0].clone();
                receipt["logs"].as_array_mut().unwrap().push(log);
            }
            _ => (),
        }
        if mode == "saved_reorg" {
            capture.checkpoint = Some(b.clone());
        }
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener,Router::new().route("/",post(move |Json(q):Json<Value>| {
                let receipt=receipt.clone(); let b=b.clone(); let reads=reads.clone();
                async move {
                    let result=match q["method"].as_str().unwrap() {
                        "eth_chainId"=>json!(if mode=="wrong_chain" {"0x1"}else{"0x2105"}),
                        "eth_getTransactionReceipt"=>if mode=="pending"{Value::Null}else{receipt},
                        "eth_getBlockByNumber"=>{
                            let latest=q["params"][0]=="latest"||q["params"][0]=="0x64";
                            let changed=!latest&&reads.fetch_add(1,std::sync::atomic::Ordering::SeqCst)>0&&matches!(mode,"reorg"|"saved_reorg");
                            json!({"number":if latest{"0x64"}else{"0x50"},"hash":if latest{B256::repeat_byte(6).to_string()}else if changed{B256::ZERO.to_string()}else{b.hash},"timestamp":format!("0x{:x}",now().unwrap()-if mode=="stale"{200}else{5})})
                        },
                        method=>panic!("unexpected RPC {method}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
                }
            }))).await.unwrap();
        });
        let rpc = BaseRpc::new(&url, 12, 120).unwrap();
        capture.endpoint_generations.insert(rpc.endpoint_id(), 9);
        let result = rpc.verify_funding_delivery(&e, capture.clone()).await;
        match mode {
            "valid" => {
                let proof = result.unwrap().unwrap();
                assert!(proof.capture() == &capture);
                assert_eq!(proof.endpoint_generation(), 9);
                assert_eq!(proof.log_index(), 2);
                assert_eq!(proof.operation(), e.operation);
                assert_eq!(proof.recipient(), e.recipient);
                assert!(proof.is_fresh(now().unwrap()).unwrap());
                let cloned = proof.clone();
                proof
                    .freshness
                    .advance_for_test(std::time::Duration::from_secs(121));
                assert!(!cloned.is_fresh(now().unwrap()).unwrap());
                assert!(!proof.is_fresh(now().unwrap() - 100).unwrap());
            }
            "pending" | "unconfirmed" => assert!(result.unwrap().is_none()),
            "wrong_chain" => assert!(
                result
                    .err()
                    .unwrap()
                    .downcast_ref::<EndpointMismatch>()
                    .is_some()
            ),
            "saved_reorg" => assert!(
                result
                    .err()
                    .unwrap()
                    .downcast_ref::<EvidenceConflict>()
                    .is_some()
            ),
            "reorg" => assert!(
                result
                    .err()
                    .unwrap()
                    .downcast_ref::<EvidenceConflict>()
                    .is_none()
            ),
            _ => assert!(result.is_err(), "{mode}"),
        }
        server.abort();
    }
}
