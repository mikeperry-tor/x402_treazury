use super::*;
use crate::rotation::base::{BaseRpc, now};
use alloy_primitives::{Address, keccak256};
use serde_json::{Value, json};
struct Fixture {
    store: Store,
    job: funding::FundingJob,
    quote: Quote,
    dir: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(
            &dir.path().join("state"),
            &dir.path().join("key"),
            1,
            b"snapshot",
        )
        .unwrap();
        super::super::test_support::declare_two_reserves(
            &mut store,
            "delivery",
            "0.007",
            Some(U256::from(1_000_000)),
            0,
        )
        .unwrap();
        let job = store.funding_jobs().unwrap().remove(0);
        let request = json!({"swapType":"EXACT_OUTPUT","originAsset":"nep141:zec.omft.near","destinationAsset":crate::rotation::near::USDC,"depositType":"ORIGIN_CHAIN","recipientType":"DESTINATION_CHAIN","amount":job.target,"recipient":job.recipient,"deadline":"2030-01-01T00:00:00Z"});
        let response = json!({"quoteRequest":request,"quote":{"depositAddress":"original","amountIn":"100000","amountOut":job.target,"deadline":"2030-01-01T00:00:00Z"}});
        let quote = Quote {
            request,
            response,
            input: 100000,
            deadline: 1893456000,
            deposit: Some("original".into()),
        };
        store
            .save_funding_quote(&job.id, &serde_json::to_vec(&quote).unwrap())
            .unwrap();
        // Inert, unfunded v5 serialization: no inputs, outputs or shielded actions.
        let raw =
            alloy_primitives::hex::decode("050000800a27a726b4d0d6c200000000640000000000000000")
                .unwrap();
        let transaction = crate::treasury::submission::decode(&raw).unwrap();
        let encrypted = seal(
            &store.key,
            &format!(
                "v1:{}:{}:outgoing:{}",
                store.id,
                store.network.name(),
                job.operation_id
            ),
            &raw,
        )
        .unwrap();
        let facts = TransactionFacts {
            txid: transaction.txid().to_string(),
            expiry_height: 100,
            amount_zatoshis: 100000,
            fee_zatoshis: 1000,
            deadline: Some(1893456000),
        };
        store
            .db
            .execute(
                "INSERT INTO budget_entries(id,pool_id,day,original_day,requested,reserved,consumed) VALUES(?1,?2,0,0,101000,101000,0)",
                params![job.operation_id, job.pool_id],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO outgoing VALUES(?1,'PREPARED',?2,1)",
                params![job.operation_id, encrypted],
            )
            .unwrap();
        store.db.execute("INSERT INTO treasury_operations(id,facts,submission,attempts) VALUES(?1,?2,'BROADCAST',1)",params![job.operation_id,serde_json::to_string(&facts).unwrap()]).unwrap();
        Self {
            store,
            job,
            quote,
            dir,
        }
    }
    fn observe(&mut self, n: u8) {
        let value = json!({"status":"REFUNDED","quoteResponse":self.quote.response,"swapDetails":{
   "originChainTxHashes":[{"hash":self.store.operation(&self.job.operation_id).unwrap().facts.txid}],
   "destinationChainTxHashes":[{"hash":B256::repeat_byte(n).to_string()}],"amountOut":self.job.target}});
        let observation =
            crate::rotation::near::restore_status_observation(&self.quote, &value).unwrap();
        self.store
            .record_delivery_observation(
                &self.job.id,
                &self.job.operation_id,
                &observation,
                now().unwrap(),
            )
            .unwrap();
    }
    fn capture(&mut self) -> DeliveryRead {
        self.store
            .capture_funding_delivery(&self.job.id, &self.job.operation_id)
            .unwrap()
            .unwrap()
    }
    fn reopen(self) -> Self {
        let Self {
            store,
            job,
            quote,
            dir,
        } = self;
        let id = store.id.clone();
        drop(store);
        let store = Store::open(&dir.path().join("state"), &dir.path().join("key"), &id).unwrap();
        Self {
            store,
            job,
            quote,
            dir,
        }
    }
}
async fn rpc(recipient: &str, amount: &str) -> (BaseRpc, tokio::task::JoinHandle<()>) {
    rpc_variant(recipient, amount, false, 5).await
}
async fn rpc_variant(
    recipient: &str,
    amount: &str,
    pending: bool,
    block_byte: u8,
) -> (BaseRpc, tokio::task::JoinHandle<()>) {
    use axum::{Json, Router, routing::post};
    let recipient: Address = recipient.parse().unwrap();
    let amount: U256 = amount.parse().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener,Router::new().route("/",post(move |Json(q):Json<Value>| async move {
  let hash=B256::repeat_byte(block_byte);
  let result=match q["method"].as_str().unwrap(){
   "eth_chainId"=>json!("0x2105"),
   "eth_getBlockByNumber"=>{let tag=q["params"][0].as_str().unwrap();let h=if tag=="latest"{100}else{u64::from_str_radix(&tag[2..],16).unwrap()};json!({"number":format!("0x{h:x}"),"hash":hash,"timestamp":format!("0x{:x}",now().unwrap()-5)})},
   "eth_getTransactionReceipt" if pending=>Value::Null,
   "eth_getTransactionReceipt"=>json!({"status":if block_byte==8 {"0x0"} else {"0x1"},"transactionHash":q["params"][0],"blockNumber":"0x50","blockHash":hash,"logs":[{
    "address":crate::payment::USDC,"removed":false,"transactionHash":q["params"][0],"blockNumber":"0x50","blockHash":hash,"logIndex":"0x2",
    "topics":[keccak256("Transfer(address,address,uint256)"),B256::ZERO,B256::left_padding_from(recipient.as_slice())],"data":format!("0x{amount:064x}") }]}),
   method=>panic!("unexpected {method}")};Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
 }))).await.unwrap();
    });
    (BaseRpc::new(&url, 12, 120).unwrap(), server)
}
#[tokio::test]
async fn slots_stay_bounded_preserve_active_reopen_and_deduplicate_canonical_support() {
    let mut f = Fixture::new();
    f.observe(1);
    let first = f.capture();
    for n in 2..=200 {
        f.observe(n);
    }
    let slots:(i64,i64,i64)=f.store.db.query_row("SELECT count(*),sum(active IS NOT NULL),sum(queued IS NOT NULL) FROM funding_delivery_hints",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(slots, (1, 1, 1));
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let proof = base
        .verify_funding_delivery(&first.expected, first.capture.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proof.transaction(), B256::repeat_byte(1));
    f = f.reopen();
    assert!(f.store.commit_funding_delivery(&first, &proof).is_err());
    let recovered = f.capture();
    let proof = base
        .verify_funding_delivery(&recovered.expected, recovered.capture.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proof.transaction(), B256::repeat_byte(1));
    f.store.commit_funding_delivery(&recovered, &proof).unwrap();
    assert!(!funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    let state: String = f
        .store
        .db
        .query_row(
            "SELECT state FROM funding_jobs WHERE id=?1",
            [&f.job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(state, "COMPLETE");
    let reserved: i64 = f
        .store
        .db
        .query_row(
            "SELECT reserved FROM budget_entries WHERE id=?1",
            [&f.job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 101000);
    // A queued candidate cannot replace original accepted event history; same proof repeats deduplicate.
    f.observe(1);
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    let count: i64 = f
        .store
        .db
        .query_row("SELECT count(*) FROM funding_delivery_proofs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    f.store
        .invalidate_endpoint_evidence(proof.endpoint())
        .unwrap();
    assert!(funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    assert!(!funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    server.abort();
}
#[tokio::test]
async fn original_binding_and_wallet_generation_races_reject_without_releasing_source() {
    let mut f = Fixture::new();
    f.observe(1);
    let read = f.capture();
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE treasury_operations SET facts=json_set(facts,'$.fee_zatoshis',999) WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    assert!(f.store.commit_funding_delivery(&read, &proof).is_err());
    let count: i64 = f
        .store
        .db
        .query_row("SELECT count(*) FROM funding_delivery_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    server.abort();
}

#[tokio::test]
async fn event_owner_survives_dispute_and_operation_rollover() {
    let mut f = Fixture::new();
    f.observe(1);
    let read = f.capture();
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    f.store
        .invalidate_endpoint_evidence(proof.endpoint())
        .unwrap();
    let old = f.job.operation_id.clone();
    let original_raw = f.store.prepared_bytes(&old).unwrap();
    f.store
        .db
        .execute("UPDATE outgoing SET state='RESOLVED' WHERE id=?1", [&old])
        .unwrap();
    f.job.operation_id = Uuid::new_v4().to_string();
    f.store
        .db
        .execute(
            "UPDATE funding_progress SET operation_id=?2 WHERE job_id=?1",
            params![f.job.id, f.job.operation_id],
        )
        .unwrap();
    f.store.db.execute("INSERT INTO budget_entries SELECT ?1,pool_id,day,original_day,requested,reserved,consumed FROM budget_entries WHERE id=?2",params![f.job.operation_id,old]).unwrap();
    f.store
        .db
        .execute(
            "INSERT INTO outgoing SELECT ?1,state,raw,revision FROM outgoing WHERE id=?2",
            params![f.job.operation_id, old],
        )
        .unwrap();
    f.store.db.execute("INSERT INTO treasury_operations SELECT ?1,facts,submission,attempts FROM treasury_operations WHERE id=?2",params![f.job.operation_id,old]).unwrap();
    let encrypted = seal(
        &f.store.key,
        &f.store
            .delivery_aad(&f.job.operation_id)
            .replace(":delivery:", ":outgoing:"),
        &original_raw,
    )
    .unwrap();
    f.store
        .db
        .execute(
            "UPDATE outgoing SET raw=?2 WHERE id=?1",
            params![f.job.operation_id, encrypted],
        )
        .unwrap();
    f.observe(1);
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(
        f.store
            .commit_funding_delivery(&read, &proof)
            .unwrap_err()
            .to_string()
            .contains("already owned")
    );
    let count: i64 = f
        .store
        .db
        .query_row("SELECT count(*) FROM funding_delivery_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    server.abort();
}
#[test]
fn absent_hint_does_not_parse_legacy_source_and_invalid_hint_does_not_hide_correction() {
    let mut f = Fixture::new();
    assert!(
        f.store
            .capture_funding_delivery(&f.job.id, "missing-operation")
            .unwrap()
            .is_none()
    );
    f.observe(1);
    let read = f.capture();
    // Simulate an old unsigned candidate whose source correlation was rejected;
    // a corrected queued candidate must survive its rejection after restart.
    let mut hint: Hint = serde_json::from_slice(
        &unseal(
            &f.store.key,
            &f.store.delivery_aad(&f.job.operation_id),
            &f.store
                .db
                .query_row(
                    "SELECT active FROM funding_delivery_hints WHERE operation_id=?1",
                    [&f.job.operation_id],
                    |r| r.get::<_, Vec<u8>>(0),
                )
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    hint.source = B256::ZERO;
    let bytes = seal(
        &f.store.key,
        &f.store.delivery_aad(&f.job.operation_id),
        &serde_json::to_vec(&hint).unwrap(),
    )
    .unwrap();
    f.store
        .db
        .execute(
            "UPDATE funding_delivery_hints SET active=?2 WHERE operation_id=?1",
            params![f.job.operation_id, bytes],
        )
        .unwrap();
    f.observe(2);
    assert!(
        f.store
            .capture_funding_delivery(&f.job.id, &f.job.operation_id)
            .is_err()
    );
    let corrected = f.capture();
    assert_ne!(corrected.ticket, read.ticket);
    let slots: (i64, i64) = f
        .store
        .db
        .query_row(
            "SELECT active IS NOT NULL,queued IS NOT NULL FROM funding_delivery_hints",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(slots, (1, 0));
}

#[tokio::test]
async fn missing_receipt_requires_saved_anchor_recheck_and_never_falls_back_to_older_proof() {
    let mut f = Fixture::new();
    f.observe(1);
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    for n in 2..=25 {
        f.observe(n);
        let read = f.capture();
        let observed = base
            .verify_funding_delivery(&read.expected, read.capture.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(observed.transaction(), B256::repeat_byte(1));
        f.store.finish_delivery_read(&read).unwrap();
    }
    // A re-included canonical transaction obtains new immutable support.
    let (new_base, new_server) = rpc_variant(&f.job.recipient, &f.job.target, false, 6).await;
    let read = f.capture();
    let proof = new_base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    let count: i64 = f
        .store
        .db
        .query_row("SELECT count(*) FROM funding_delivery_proofs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    f = f.reopen();
    let (pending, pending_server) = rpc_variant(&f.job.recipient, &f.job.target, true, 7).await;
    let read = f.capture();
    let error = pending
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .err()
        .unwrap();
    assert!(
        error
            .downcast_ref::<crate::rotation::base::EvidenceConflict>()
            .is_some()
    );
    f.store.record_base_failure(&error).unwrap();
    assert!(funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    let undisputed_old:i64=f.store.db.query_row("SELECT count(*) FROM funding_delivery_proofs p WHERE NOT EXISTS(SELECT 1 FROM funding_delivery_disputes d WHERE d.proof_id=p.id)",[],|r|r.get(0)).unwrap();
    assert_eq!(undisputed_old, 1);
    server.abort();
    new_server.abort();
    pending_server.abort();
}

#[tokio::test]
async fn reverted_reincluded_receipt_disputes_prior_canonical_support() {
    let mut f = Fixture::new();
    f.observe(1);
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    let (reverted, other) = rpc_variant(&f.job.recipient, &f.job.target, false, 8).await;
    let read = f.capture();
    let error = reverted
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .err()
        .unwrap();
    assert!(
        error
            .downcast_ref::<crate::rotation::base::EvidenceConflict>()
            .is_some()
    );
    f.store.record_base_failure(&error).unwrap();
    assert!(funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    server.abort();
    other.abort();
}

#[tokio::test]
async fn endpoint_restart_rechecks_accepted_event_and_chain_dispute_gives_correction_a_turn() {
    let mut f = Fixture::new();
    f.observe(1);
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    f.observe(2);
    f.store
        .invalidate_endpoint_evidence(proof.endpoint())
        .unwrap();
    f = f.reopen();
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proof.transaction(), B256::repeat_byte(1));
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    let (fork, fork_server) = rpc_variant(&f.job.recipient, &f.job.target, true, 7).await;
    let read = f.capture();
    let error = fork
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .err()
        .unwrap();
    f.store.record_base_failure(&error).unwrap();
    f.store.finish_delivery_read(&read).unwrap();
    let (corrected, corrected_server) =
        rpc_variant(&f.job.recipient, &f.job.target, false, 7).await;
    let read = f.capture();
    let proof = corrected
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proof.transaction(), B256::repeat_byte(2));
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    assert!(!funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    let read = f.capture();
    assert_eq!(read.capture.durable_balance_anchors.len(), 1);
    assert_eq!(
        read.capture.durable_balance_anchors[0].hash,
        B256::repeat_byte(7).to_string()
    );
    let (later, later_server) = rpc_variant(&f.job.recipient, &f.job.target, true, 9).await;
    let error = later
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .err()
        .unwrap();
    let conflict = error
        .downcast_ref::<crate::rotation::base::EvidenceConflict>()
        .unwrap();
    assert_eq!(conflict.anchor.hash, B256::repeat_byte(7).to_string());
    f.store.record_base_failure(&error).unwrap();
    assert!(funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    later_server.abort();
    server.abort();
    fork_server.abort();
    corrected_server.abort();
}

#[tokio::test]
async fn successful_delivery_does_not_hide_positive_or_unknown_return_claim() {
    let mut f = Fixture::new();
    let source = f.store.operation(&f.job.operation_id).unwrap().facts.txid;
    let mut value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{
   "originChainTxHashes":[{"hash":source}],"amountOut":f.job.target,"refundedAmount":"500"}});
    // A missing destination correlation must not erase a separately valid claim.
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    assert!(observation.settlement().is_none());
    f.store
        .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 1)
        .unwrap();
    for returned in [Value::Null, json!("0"), json!("700"), json!("malformed")] {
        value["swapDetails"]["destinationChainTxHashes"] =
            json!([{"hash":B256::repeat_byte(1).to_string()}]);
        value["swapDetails"]["refundedAmount"] = returned;
        let observation =
            crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
        f.store
            .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 2)
            .unwrap();
    }
    let claim:(String,bool)=f.store.db.query_row("SELECT declared_return,return_conflict FROM funding_delivery_hints WHERE operation_id=?1",[&f.job.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(claim, ("500".into(), true));
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    assert!(!funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    // Install already-accounted source + historical readiness; the independent
    // known return obligation must still prevent the old SUCCESS branch.
    f.store
        .db
        .execute(
            "UPDATE treasury_operations SET submission='CONFIRMED' WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE budget_entries SET consumed=101000,reserved=0 WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store.db.execute("UPDATE funding_jobs SET readiness_requirement=target,readiness_support='historical',readiness_operation=?2 WHERE id=?1",params![f.job.id,f.job.operation_id]).unwrap();
    assert!(!funding::finish_if_proven(&f.store.db, &f.job.id, &f.job.operation_id).unwrap());
    let accounted: (i64, i64) = f
        .store
        .db
        .query_row(
            "SELECT consumed,reserved FROM budget_entries WHERE id=?1",
            [&f.job.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(accounted, (101000, 0));
    f = f.reopen();
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    server.abort();
}

#[tokio::test]
async fn late_return_claim_reopens_current_observation_without_rewriting_success_audit() {
    let mut f = Fixture::new();
    let mut value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{
  "originChainTxHashes":[{"hash":f.store.operation(&f.job.operation_id).unwrap().facts.txid}],
  "destinationChainTxHashes":[{"hash":B256::repeat_byte(1).to_string()}],"amountOut":f.job.target,"refundedAmount":"0"}});
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    f.store
        .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 1)
        .unwrap();
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    f.store
        .db
        .execute(
            "UPDATE treasury_operations SET submission='CONFIRMED' WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE outgoing SET state='RESOLVED' WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE budget_entries SET consumed=101000,reserved=0 WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store.db.execute("UPDATE funding_jobs SET readiness_requirement=target,readiness_support='historical',readiness_operation=?2 WHERE id=?1",params![f.job.id,f.job.operation_id]).unwrap();
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::ZERO);
    assert!(funding::finish_if_proven(&f.store.db, &f.job.id, &f.job.operation_id).unwrap());
    super::super::source_support::tests::publish_unavailable(&mut f.store);
    let facts = f
        .store
        .funding_facts(&f.job.id, &f.job.operation_id)
        .unwrap();
    assert!(facts.readiness_recorded && facts.delivery_verified && facts.provider_success);
    assert!(facts.source_accounting_pending && facts.observations_needed());
    assert!(facts.source_reconciliation_needed());
    assert_eq!(
        f.store.funding_job(&f.job.id).unwrap().unwrap().phase,
        super::super::funding::FundingPhase::Complete
    );
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::ZERO);
    value["swapDetails"]["destinationChainTxHashes"] = Value::Null;
    value["swapDetails"]["refundedAmount"] = json!("500");
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    f.store
        .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 2)
        .unwrap();
    f = f.reopen();
    let job = f.store.funding_job(&f.job.id).unwrap().unwrap();
    assert_eq!(job.phase, funding::FundingPhase::RefundPending);
    assert!(
        job.last_error
            .as_deref()
            .unwrap_or("")
            .contains("return_accounting_pending")
    );
    let facts = f
        .store
        .funding_facts(&f.job.id, &f.job.operation_id)
        .unwrap();
    assert!(
        facts.return_pending
            && facts.delivery_verified
            && facts.observations_needed()
            && facts.status_needed()
    );
    let outcome: String = f
        .store
        .db
        .query_row(
            "SELECT completion_outcome FROM funding_jobs WHERE id=?1",
            [&f.job.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "SUCCESS");
    assert!(!funding::destination_obligation_pending(&f.store.db, &f.job.wallet_id).unwrap());
    let accounted: (i64, i64) = f
        .store
        .db
        .query_row(
            "SELECT consumed,reserved FROM budget_entries WHERE id=?1",
            [&f.job.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(accounted, (101000, 0));
    server.abort();
}
#[test]
fn malformed_return_amount_is_unknown_obligation_not_zero() {
    let mut f = Fixture::new();
    let value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{"refundedAmount":"private malformed amount"}});
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    f.store
        .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 1)
        .unwrap();
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    let amount: Option<String> = f
        .store
        .db
        .query_row(
            "SELECT declared_return FROM funding_delivery_hints WHERE operation_id=?1",
            [&f.job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(amount.is_none());
}

#[test]
fn declared_return_does_not_hide_original_source_storage_failure() {
    let mut f = Fixture::new();
    let value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{"refundedAmount":"0"}});
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    f.store
        .db
        .execute(
            "UPDATE outgoing SET raw=x'00' WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    assert!(
        f.store
            .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 1)
            .is_err()
    );
    let retained: i64 = f
        .store
        .db
        .query_row(
            "SELECT count(*) FROM funding_delivery_hints WHERE operation_id=?1",
            [&f.job.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained, 0,
        "storage failure must not become a successful incomplete observation"
    );
}

#[tokio::test]
async fn exact_zero_requires_canonical_return_pass_and_new_output_reopens_completed_job() {
    let mut f = Fixture::new();
    let mut value = json!({"status":"FAILED","quoteResponse":f.quote.response,"swapDetails":{
        "originChainTxHashes":[{"hash":f.store.operation(&f.job.operation_id).unwrap().facts.txid}],
        "destinationChainTxHashes":[{"hash":B256::repeat_byte(1).to_string()}],"amountOut":f.job.target}});
    let observe = |f: &mut Fixture, value: &Value| {
        let observation =
            crate::rotation::near::restore_status_observation(&f.quote, value).unwrap();
        f.store
            .record_delivery_observation(
                &f.job.id,
                &f.job.operation_id,
                &observation,
                now().unwrap(),
            )
            .unwrap();
    };
    observe(&mut f, &value);
    for status in [
        crate::rotation::near::SwapStatus::Refunded,
        crate::rotation::near::SwapStatus::IncompleteDeposit,
    ] {
        f.store
            .record_provider_outcome(&f.job.id, &f.job.operation_id, &status, 1)
            .unwrap();
    }
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    f.store
        .confirm_spend(&f.job.operation_id, 101000, 0)
        .unwrap();
    f.store.db.execute("UPDATE funding_jobs SET readiness_requirement=target,readiness_support='historical',readiness_operation=?2 WHERE id=?1",params![f.job.id,f.job.operation_id]).unwrap();
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::ZERO);
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "absent amount is unknown"
    );
    assert!(!funding::finish_if_proven(&f.store.db, &f.job.id, &f.job.operation_id).unwrap());
    value["swapDetails"]["refundedAmount"] = json!("0");
    observe(&mut f, &value);
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "new claim invalidates prior inventory"
    );
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::ZERO);
    let revision = f.store.return_inventory_revision().unwrap();
    observe(&mut f, &value);
    assert_eq!(
        f.store.return_inventory_revision().unwrap(),
        revision,
        "identical status must not invalidate retained history"
    );
    assert!(funding::finish_if_proven(&f.store.db, &f.job.id, &f.job.operation_id).unwrap());
    let facts = f
        .store
        .funding_facts(&f.job.id, &f.job.operation_id)
        .unwrap();
    assert!(
        !facts.provider_success
            && facts.provider_failed
            && facts.provider_refunded
            && facts.provider_incomplete
    );
    assert!(
        !facts.status_needed(),
        "historical labels are audit, not a veto after exact proofs"
    );
    f.store
        .record_refund(super::super::refunds::RefundStatus {
            txid: "new canonical output".into(),
            output_index: 0,
            operation_id: f.job.operation_id.clone(),
            amount: 1,
            height: 100,
        })
        .unwrap();
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    assert_eq!(
        f.store.funding_job(&f.job.id).unwrap().unwrap().phase,
        funding::FundingPhase::RecoveryRequired
    );
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(1));
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "positive known output contradicts retained zero"
    );
    f = f.reopen();
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    let (outcome,gross):(String,i64)=f.store.db.query_row("SELECT f.completion_outcome,b.consumed FROM funding_jobs f JOIN budget_entries b ON b.id=?2 WHERE f.id=?1",params![f.job.id,f.job.operation_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!((outcome.as_str(), gross), ("SUCCESS", 101000));
    server.abort();
}

#[test]
fn exact_external_source_closure_survives_provider_claim_but_label_alone_does_not() {
    let mut f = Fixture::new();
    let value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{"refundedAmount":"500"}});
    let observation = crate::rotation::near::restore_status_observation(&f.quote, &value).unwrap();
    f.store
        .record_delivery_observation(&f.job.id, &f.job.operation_id, &observation, 1)
        .unwrap();
    f.store
        .db
        .execute(
            "UPDATE funding_jobs SET state='COMPLETE',completion_outcome='EXTERNAL' WHERE id=?1",
            [&f.job.id],
        )
        .unwrap();
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    // Exact untouched quoted archive proves the operation had no source effects.
    f.store
        .db
        .execute(
            "DELETE FROM treasury_operations WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store
        .db
        .execute("DELETE FROM outgoing WHERE id=?1", [&f.job.operation_id])
        .unwrap();
    f.store
        .db
        .execute(
            "DELETE FROM budget_entries WHERE id=?1",
            [&f.job.operation_id],
        )
        .unwrap();
    f.store.db.execute("INSERT INTO funding_recovery(operation_id,job_id,phase,quote) SELECT operation_id,job_id,'\"QUOTED\"',quote FROM funding_progress WHERE job_id=?1",[&f.job.id]).unwrap();
    assert!(!return_pending(&f.store.db, &f.job.operation_id).unwrap());
    let claim: String = f
        .store
        .db
        .query_row(
            "SELECT declared_return FROM funding_delivery_hints WHERE operation_id=?1",
            [&f.job.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claim, "500");
}

#[tokio::test]
async fn positive_return_needs_full_canonical_coverage_and_exact_delivery_correlation() {
    let mut f = Fixture::new();
    let mut value = json!({"status":"SUCCESS","quoteResponse":f.quote.response,"swapDetails":{
        "originChainTxHashes":[{"hash":f.store.operation(&f.job.operation_id).unwrap().facts.txid}],
        "destinationChainTxHashes":[{"hash":B256::repeat_byte(1).to_string()}],"amountOut":f.job.target,"refundedAmount":"500"}});
    let observe = |f: &mut Fixture, value: &Value| {
        let observation =
            crate::rotation::near::restore_status_observation(&f.quote, value).unwrap();
        f.store
            .record_delivery_observation(
                &f.job.id,
                &f.job.operation_id,
                &observation,
                now().unwrap(),
            )
            .unwrap();
    };
    observe(&mut f, &value);
    let (base, server) = rpc(&f.job.recipient, &f.job.target).await;
    let read = f.capture();
    let proof = base
        .verify_funding_delivery(&read.expected, read.capture.clone())
        .await
        .unwrap()
        .unwrap();
    f.store.commit_funding_delivery(&read, &proof).unwrap();
    f.store
        .confirm_spend(&f.job.operation_id, 101000, 0)
        .unwrap();
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(499));
    assert!(return_pending(&f.store.db, &f.job.operation_id).unwrap());
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(500));
    assert!(!return_pending(&f.store.db, &f.job.operation_id).unwrap());
    value["swapDetails"]["destinationChainTxHashes"] =
        json!([{"hash":B256::repeat_byte(2).to_string()}]);
    observe(&mut f, &value);
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(500));
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "another delivery correlation cannot close this original"
    );
    value["swapDetails"]["destinationChainTxHashes"] =
        json!([{"hash":B256::repeat_byte(1).to_string()}]);
    observe(&mut f, &value);
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(500));
    assert!(!return_pending(&f.store.db, &f.job.operation_id).unwrap());
    f.store.revoke_return_support(&f.job.operation_id).unwrap();
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "canonical contradiction revokes before replacement pass activation"
    );
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::from(500));
    value["swapDetails"]["refundedAmount"] = json!("0");
    observe(&mut f, &value);
    super::super::source_support::tests::publish_confirmed_returns(&mut f.store, U256::ZERO);
    assert!(
        return_pending(&f.store.db, &f.job.operation_id).unwrap(),
        "later zero cannot erase retained positive claim"
    );
    server.abort();
}
