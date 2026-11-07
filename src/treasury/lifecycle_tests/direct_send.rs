//! Service admission, source scheduling and the real treasury actor share one payment.
use super::*;
use crate::direct_swap::{Config, Service};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct ObservedSubmission {
    inner: submission::GrpcSubmission,
    calls: Arc<AtomicUsize>,
    submitted: Option<tokio::sync::oneshot::Sender<(bool, Vec<u8>)>>,
}
impl TransactionSubmission for ObservedSubmission {
    async fn preflight(&mut self, transaction: &PreparedTransaction, retry: bool) -> Result<()> {
        self.inner.preflight(transaction, retry).await
    }
    async fn submit(&mut self, transaction: BroadcastTransaction) -> Result<SubmissionOutcome> {
        assert_eq!(
            self.calls.fetch_add(1, Ordering::SeqCst),
            0,
            "a duplicate must not submit another payment"
        );
        let bytes = transaction.transaction().bytes().to_vec();
        let result = self.inner.submit(transaction).await;
        if let Some(submitted) = self.submitted.take() {
            let _ = submitted.send((matches!(&result, Ok(SubmissionOutcome::Accepted)), bytes));
        }
        result
    }
    async fn lookup(&mut self, transaction: &PreparedTransaction) -> Result<TransactionPresence> {
        self.inner.lookup(transaction).await
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_service_submits_once_despite_diagnostic_failure_and_preserves_accepted_total_authority()
 {
    let (dir, net, mut owner, endpoint) = funded_fixture(2).await;
    let treasury_id = owner.status().await.unwrap().treasury_id;
    let recipient = owner
        .client
        .as_ref()
        .unwrap()
        .unified_addresses_json()
        .await[0]["encoded_address"]
        .as_str()
        .unwrap()
        .to_owned();
    let principal = 50_000u64;
    let payment = zcash_client_backend::zip321::Payment::new(
        recipient.parse().unwrap(),
        Some(zcash_protocol::value::Zatoshis::from_u64(principal).unwrap()),
        None,
        None,
        None,
        vec![],
    )
    .unwrap();
    let preview = owner
        .client
        .as_ref()
        .unwrap()
        .preview_send(
            zcash_client_backend::zip321::TransactionRequest::new(vec![payment]).unwrap(),
            zip32::AccountId::ZERO,
        )
        .await
        .unwrap();
    let actual_fee = zingolib::data::proposal::total_fee(&preview)
        .unwrap()
        .into_u64();
    let total = principal.checked_add(actual_fee).unwrap();
    assert!(
        actual_fee < 100_000,
        "fixture must distinguish actual fee from the maximum"
    );
    let config = Config {
        receive: false,
        send: true,
        max_send_zec: Some(format!(
            "{}.{:08}",
            total / 100_000_000,
            total % 100_000_000
        )),
    };
    let arguments = json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec",
        "amount":principal.to_string(),"recipient":recipient,"request_id":"native-service-payment"});
    let store = owner.store.clone();
    let (treasury, commands) = super::super::actor::channel();
    // There is deliberately no NEAR client, funding configuration or Base RPC.
    let service = Service::native(store.clone(), treasury.clone(), 100_000, 5);
    let accepted = service
        .invoke(
            "owner",
            &config,
            "x402_treazury_swap_send",
            arguments.clone(),
        )
        .await
        .unwrap();
    let id = accepted["operation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(accepted["operation"]["route"], "native");
    assert!(accepted["operation"]["quote"].is_null());
    assert_eq!(
        accepted["operation"]["authority"]["max_debit_zatoshis"],
        total
    );

    // Only advisory source-error publication is made unavailable. Input, byte,
    // snapshot and broadcast journals remain writable. FAIL preserves the trigger
    // counter, allowing the test to prove this failure path was exercised.
    uuid::Uuid::parse_str(&id).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("state/state.sqlite")).unwrap();
    db.execute_batch(&format!(
        "CREATE TABLE source_diagnostic_attempts(value INTEGER NOT NULL);
        CREATE TRIGGER reject_source_diagnostic BEFORE UPDATE OF payload ON direct_operations
        WHEN NEW.id='{id}' BEGIN
            INSERT INTO source_diagnostic_attempts VALUES(1);
            SELECT RAISE(FAIL,'fixture source diagnostic unavailable');
        END;"
    ))
    .unwrap();
    let (entered, reached) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let calculations = Arc::new(AtomicUsize::new(0));
    owner.calculation_control = Some(tokio::sync::Mutex::new(publication::CalculationControl {
        entered: Some(entered),
        before_acceptance: false,
        release: wait,
        calls: calculations.clone(),
        panic_calculation: false,
        panic_facts: false,
    }));
    let (submitted, sent) = tokio::sync::oneshot::channel();
    let submissions = Arc::new(AtomicUsize::new(0));
    let sender = ObservedSubmission {
        inner: submission::GrpcSubmission::with_network(
            endpoint.clone(),
            endpoint,
            TreasuryNetwork::Synthetic,
        )
        .unwrap(),
        calls: submissions.clone(),
        submitted: Some(submitted),
    };
    let actor_stop = CancellationToken::new();
    let actor = tokio::spawn(owner.run_commands(commands, sender, actor_stop.clone()));
    let service_stop = CancellationToken::new();
    let running_service = service.clone();
    let stopped = service_stop.clone();
    let worker = tokio::spawn(async move { running_service.run(stopped).await });
    assert_eq!(reached.await.unwrap(), id);
    assert_eq!(calculations.load(Ordering::SeqCst), 0);
    assert_eq!(submissions.load(Ordering::SeqCst), 0);
    let lower_limit = Config {
        send: true,
        max_send_zec: Some("0".into()),
        ..Default::default()
    };
    let duplicate = service
        .invoke(
            "owner",
            &lower_limit,
            "x402_treazury_swap_send",
            arguments.clone(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate["operation"]["id"], id);
    assert_eq!(
        duplicate["operation"]["authority"],
        accepted["operation"]["authority"]
    );
    let mut conflicting = arguments.clone();
    conflicting["amount"] = json!("50001");
    assert!(
        service
            .invoke("owner", &config, "x402_treazury_swap_send", conflicting)
            .await
            .is_err()
    );
    assert!(
        service
            .invoke(
                "other-owner",
                &Config::default(),
                "x402_treazury_swap_status",
                json!({"operation_id":id})
            )
            .await
            .is_err()
    );
    release.send(()).unwrap();
    // This is a fixture hang guard, not an execution deadline. The handoff itself
    // is the submitter's oneshot after the actual source-chain RPC returns.
    let (submitted, original) = tokio::time::timeout(Duration::from_secs(180), sent)
        .await
        .expect("service failed to submit after an advisory diagnostic write failed")
        .unwrap();
    assert!(submitted);
    // This command follows submission in the serialized actor and establishes
    // that its durable response publication has finished before inspection.
    treasury.sync().await.unwrap();
    let status = service
        .invoke(
            "owner",
            &Config::default(),
            "x402_treazury_swap_status",
            json!({"kind":"send","request_id":"native-service-payment"}),
        )
        .await
        .unwrap();
    assert_eq!(status["operation"]["id"], id);
    assert_eq!(status["source"]["operation"]["submission"], "BROADCAST");
    assert_eq!(status["source"]["operation"]["attempts"], 1);
    assert_eq!(
        status["source"]["operation"]["facts"]["amount_zatoshis"],
        principal
    );
    assert_eq!(
        status["source"]["operation"]["facts"]["fee_zatoshis"],
        actual_fee
    );
    assert!(status["source"]["operation"]["facts"]["deadline"].is_null());
    let operation = id.clone();
    let bytes = original.clone();
    store
        .call(move |s| {
            assert_eq!(PreparedTransaction::load(s, &operation)?.bytes(), bytes);
            assert!(s.status()?.pools.is_empty());
            assert!(s.funding_jobs()?.is_empty());
            assert_eq!(s.status()?.treasury_operations.len(), 1);
            assert!(!s.prepared_inputs(&operation)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    let attempts: i64 = db
        .query_row(
            "SELECT count(*) FROM source_diagnostic_attempts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        attempts > 0,
        "the best-effort diagnostic failure must actually execute"
    );
    assert!(net.chain.read().await.taddr_request_log().is_empty());
    assert_eq!(calculations.load(Ordering::SeqCst), 1);
    assert_eq!(submissions.load(Ordering::SeqCst), 1);
    let after_submission = service
        .invoke("owner", &lower_limit, "x402_treazury_swap_send", arguments)
        .await
        .unwrap();
    assert_eq!(after_submission["operation"]["id"], id);
    assert_eq!(after_submission["source"]["operation"]["attempts"], 1);
    service_stop.cancel();
    worker.await.unwrap().unwrap();
    actor_stop.cancel();
    drop((service, treasury, store, db));
    actor.await.unwrap().unwrap();

    let owner = Treasury::open_with_network(
        dir.path().join("state"),
        dir.path().join("key"),
        treasury_id.clone(),
        TreasuryNetwork::Synthetic,
    )
    .await
    .unwrap();
    owner
        .store
        .call(move |s| {
            let prepared = PreparedTransaction::load(s, &id)?;
            assert_eq!(prepared.bytes(), original);
            assert_eq!(
                prepared.network_identity()?,
                crate::network::IsolationId::direct_swap(&treasury_id, &id)
            );
            assert_eq!(s.operation(&id)?.attempts, 1);
            assert!(s.direct_operation("owner", &id)?.quote.is_none());
            Ok(())
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}
