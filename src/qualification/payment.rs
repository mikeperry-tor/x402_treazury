//! Task-scoped case identity, copied explicitly into the accepted store command.
use super::*;
use alloy_primitives::U256;
mod offers;
pub use offers::record as record_challenge;
mod help;
pub use help::{HelpCache, record as record_help};

#[derive(Clone)]
pub struct PaymentContext {
    guard: Arc<Guard>,
    case: String,
    unsigned: bool,
}
tokio::task_local! {
    static CASE: PaymentContext;
}
pub fn unsigned_case() -> bool {
    CASE.try_with(|c| c.unsigned).unwrap_or(false)
}
pub fn payment_context() -> Result<Option<PaymentContext>> {
    match CASE.try_with(Clone::clone) {
        Ok(context) => {
            ensure!(
                !context.unsigned && context.guard.managed,
                "qualification_payment_denied: unsigned case cannot sign"
            );
            Ok(Some(context))
        }
        Err(_) => {
            ensure!(
                ACTIVE.get().is_none(),
                "qualification_payment_denied: payment has no reviewed case context"
            );
            Ok(None)
        }
    }
}
impl Claim {
    pub async fn invoke<F: std::future::Future>(&self, future: F) -> F::Output {
        CASE.scope(
            PaymentContext {
                guard: self.guard.clone(),
                case: self.case.clone(),
                unsigned: self.unsigned,
            },
            future,
        )
        .await
    }
}
/// Recheck the effective immutable managed wallet and cap independently of the
/// supervisor's planner; a valid claim is not authority for a static payer.
pub(super) fn reviewed_wallet(
    db: &Connection,
    binding: &Binding,
    reviewed: &Value,
) -> Result<String> {
    let raw: String = db.query_row(
        "SELECT payload FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
        [&binding.run],
        |r| r.get(0),
    )?;
    let pins = bounded(raw)?;
    let config = &pins["resolved_config"];
    let server = reviewed["server"]
        .as_str()
        .context("case listener missing")?;
    let source = reviewed["source"].as_str().context("case source missing")?;
    let wallet = config["wallet_bindings"][server][source]["wallet"]
        .as_str()
        .context("case wallet binding missing")?;
    let profile = &config["resolved_wallets"][wallet];
    ensure!(
        profile["mode"] == "zcash_rotation",
        "qualification requires a managed payment binding"
    );
    let reserved = crate::payment::SpendPolicy::dollars(
        reviewed["reserve_usdc"]
            .as_str()
            .context("case reservation missing")?,
    )?
    .max_atomic
    .context("qualification requires finite case reservation")?;
    // Frozen historical evidence remains readable; current TOML rejects old names.
    let cap = crate::payment::SpendPolicy::dollars(
        profile
            .get("max_api_payment_usdc")
            .or_else(|| profile.get("max_price_usd"))
            .unwrap_or(&Value::Null)
            .as_str()
            .context("pinned wallet cap missing")?,
    )?
    .max_atomic
    .context("qualification requires finite wallet cap")?;
    ensure!(
        reserved >= cap && cap > U256::ZERO,
        "case reservation is below effective payment cap"
    );
    Ok(wallet.to_owned())
}
impl PaymentContext {
    pub fn elapsed_micros(&self) -> u128 {
        self.guard.origin.elapsed().as_micros()
    }
    /// Observations may finish after authority expires; they grant no new work.
    pub async fn record_response(
        &self,
        attempt: String,
        signed_request_started_micros: u128,
        response_observed_micros: u128,
        lifecycle: Result<Value>,
        result: &anyhow::Result<reqwest::Response>,
    ) -> Result<()> {
        let context = self.clone();
        let mut receipt = super::receipt::evidence(result);
        receipt["signed_request_started_micros"] = json!(signed_request_started_micros);
        receipt["response_observed_micros"] = json!(response_observed_micros);
        match lifecycle {
            Ok(observation) => receipt["lifecycle"] = observation,
            Err(_) => {
                tracing::warn!(
                    category = "qualification_lifecycle_observation_failed",
                    "post-response lifecycle evidence unavailable; retain payment receipt, do not retry the paid request"
                );
                receipt["lifecycle"] = json!({"status":"unavailable"});
            }
        }
        tokio::task::spawn_blocking(move || context.save_receipt(&attempt, receipt))
            .await
            .context("receipt evidence worker stopped")?
    }
    fn save_receipt(&self, attempt: &str, receipt: Value) -> Result<()> {
        let mut db = connection(&self.guard.binding)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let matched: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_payment' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?3 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.attempt_id')=?4)",
            params![self.guard.binding.run,self.case,self.guard.binding.session,attempt], |r| r.get(0))?;
        ensure!(matched, "receipt lacks correlated admitted payment");
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_receipt' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2)",
            params![self.guard.binding.run,self.case], |r| r.get(0))?;
        ensure!(!exists, "receipt already recorded for case");
        let event = json!({"case":self.case,"session":self.guard.binding.session,"attempt_id":attempt,
            "receipt":receipt,"meaning":"unverified seller claim; never releases payment exposure"});
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_receipt',?2,?3)",
            params![self.guard.binding.run, event.to_string(), now()?],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Invoked synchronously by the exclusive store worker after durable
    /// admission, before constructing a signer. A crash between databases leaves
    /// only ADMITTED (unsigned) state, never an uncorrelated outgoing signature.
    pub fn record(&self, attempt: &Value) -> Result<()> {
        ensure!(
            !self.unsigned && self.guard.managed,
            "qualification_payment_denied: unsigned case cannot sign"
        );
        let mut db = connection(&self.guard.binding)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let at = now()?;
        let manifest = authority(&tx, &self.guard.binding, at)?;
        let reviewed = manifest["cases"]
            .as_array()
            .context("qualification cases missing")?
            .iter()
            .find(|c| c["id"] == self.case)
            .context("unreviewed payment case")?;
        ensure!(
            reviewed["unsigned"] == false,
            "qualification unsigned case cannot pay"
        );
        let wallet = reviewed_wallet(&tx, &self.guard.binding, reviewed)?;
        ensure!(
            attempt["pool_name"] == wallet
                && manifest["start"]["pools"]
                    .as_array()
                    .context("selected pools missing")?
                    .iter()
                    .any(|p| p == &wallet),
            "payment payer is outside reviewed pool binding"
        );
        let amount = U256::from_str_radix(
            attempt["amount"]
                .as_str()
                .context("admitted amount missing")?,
            10,
        )?;
        let (reservation,state,start,end): (i64,String,i64,i64) = tx.query_row("SELECT reservation,execution,window_start,window_end FROM cases WHERE run=?1 AND id=?2", params![self.guard.binding.run,self.case], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        ensure!(
            amount > U256::ZERO && amount <= U256::from(u64::try_from(reservation)?),
            "payment exceeds reserved case budget"
        );
        ensure!(
            state == "DISPATCHING" && at >= start && at < end,
            "payment case is outside its execution window"
        );
        let claimed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE run=?1 AND kind='application_claim' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.case')=?2 AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?3)", params![self.guard.binding.run,self.case,self.guard.binding.session], |r| r.get(0))?;
        ensure!(claimed, "payment lacks application acceptance");
        let mut event = attempt
            .as_object()
            .context("invalid payment attempt evidence")?
            .clone();
        event.insert("case".into(), json!(self.case));
        event.insert("session".into(), json!(self.guard.binding.session));
        event.insert("stage".into(), json!("admitted_before_signing"));
        event.insert("admitted_micros".into(), json!(self.elapsed_micros()));
        // At most one authorization attempt per reviewed case. Failure consumes
        // the case reservation; no automatic re-admission after signing failure.
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_payment',?2,?3)",
            params![self.guard.binding.run, Value::Object(event).to_string(), at],
        )
        .context("qualification payment already recorded or evidence journal unavailable")?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn fixture(paid: bool) -> (tempfile::TempDir, Arc<Guard>, Connection) {
        let (dir, guard, db) = super::super::tests::fixture();
        let mut binding = guard.binding.clone();
        db.execute_batch("ALTER TABLE cases ADD COLUMN reservation INTEGER; UPDATE cases SET reservation=20000;
        CREATE UNIQUE INDEX application_payment_case_once ON events(run,json_extract(detail,'$.case')) WHERE kind='application_payment';
        CREATE UNIQUE INDEX application_payment_attempt_once ON events(run,json_extract(detail,'$.attempt_id')) WHERE kind='application_payment';").unwrap();
        let raw: String = db
            .query_row("SELECT manifest FROM runs", [], |r| r.get(0))
            .unwrap();
        let mut manifest: Value = serde_json::from_str(&raw).unwrap();
        manifest["cases"][0]["unsigned"] = json!(!paid);
        manifest["cases"][0]["reserve_usdc"] = json!("0.02");
        manifest["start"] = json!({"pools":["pool"]});
        db.execute("UPDATE runs SET manifest=?1", [manifest.to_string()])
            .unwrap();
        let raw: String = db
            .query_row("SELECT payload FROM pins", [], |r| r.get(0))
            .unwrap();
        let mut pins: Value = serde_json::from_str(&raw).unwrap();
        pins["resolved_config"] = json!({"resolved_wallets":{"pool":{"mode":"zcash_rotation","max_api_payment_usdc":"0.02"}},"wallet_bindings":{"listener":{"api":{"wallet":"pool"}}}});
        let raw = pins.to_string();
        binding.pin_digest = format!("{:x}", Sha256::digest(raw.as_bytes()));
        db.execute(
            "UPDATE pins SET payload=?1,digest=?2",
            params![raw, binding.pin_digest],
        )
        .unwrap();
        db.execute(
            "UPDATE events SET detail=?1 WHERE kind='application_session'",
            [serde_json::to_string(&binding).unwrap()],
        )
        .unwrap();
        (
            dir,
            Arc::new(Guard {
                binding,
                managed: true,
                origin: Instant::now(),
                slots: Arc::new(tokio::sync::Semaphore::new(64)),
            }),
            db,
        )
    }
    fn attempt(amount: &str) -> Value {
        json!({"attempt_id":"attempt","pool":"pool_uuid","pool_name":"pool","wallet":"wallet_uuid","generation":0,"amount":amount,"address":"0x00","requirements_hash":"hash"})
    }
    async fn claim(guard: Arc<Guard>) -> Claim {
        super::super::claim_with(
            guard,
            "listener",
            "read",
            &super::super::tests::args(),
            &super::super::tests::id(),
        )
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn challenge_evidence_is_case_scoped_bounded_and_never_payment_authority() {
        let (_dir, guard, db) = fixture(false);
        let claim = claim(guard).await;
        let headers = reqwest::header::HeaderMap::new();
        record_challenge(&headers).await.unwrap();
        let count = || {
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_challenge'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(count(), 0);
        claim
            .invoke(async {
                assert!(payment_context().is_err());
                record_challenge(&headers).await.unwrap();
                record_challenge(&headers).await.unwrap();
                assert!(
                    record_challenge(&headers)
                        .await
                        .unwrap_err()
                        .to_string()
                        .contains("two-per-case limit")
                );
            })
            .await;
        assert_eq!(count(), 2);
        let event: String = db
            .query_row(
                "SELECT detail FROM events WHERE kind='application_challenge' ORDER BY seq LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["case"], "case");
        assert_eq!(event["ordinal"], 0);
        assert_eq!(event["observation"]["status"], "header_absent");
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_payment'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn accepted_store_work_retains_case_context_and_journals_once_before_signing() {
        let (_dir, guard, db) = fixture(true);
        let claim = claim(guard).await;
        claim
            .invoke(async {
                assert!(!unsigned_case());
                let context = payment_context().unwrap().unwrap();
                tokio::task::spawn_blocking(move || {
                    assert!(context.record(&attempt("20001")).is_err());
                    let mut wrong = attempt("1");
                    wrong["pool_name"] = json!("other");
                    assert!(context.record(&wrong).is_err());
                    context.record(&attempt("1")).unwrap();
                    assert!(context.record(&attempt("1")).is_err());
                })
                .await
                .unwrap();
            })
            .await;
        assert!(payment_context().unwrap().is_none());
        let raw: String = db
            .query_row(
                "SELECT detail FROM events WHERE kind='application_payment'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(event["case"], "case");
        assert_eq!(event["amount"], "1");
        assert_eq!(event["stage"], "admitted_before_signing");
        assert!(event["admitted_micros"].as_u64().is_some());
        claim.finish(false, None).await.unwrap();
    }
    #[tokio::test]
    async fn receipt_observation_is_correlated_unique_and_survives_revocation() {
        let (_dir, guard, db) = fixture(true);
        let claim = claim(guard).await;
        claim
            .invoke(async {
                let context = payment_context().unwrap().unwrap();
                let receipt = json!({"classification":"missing"});
                assert!(context.save_receipt("attempt", receipt.clone()).is_err());
                context.record(&attempt("1")).unwrap();
                assert!(context.save_receipt("wrong", receipt.clone()).is_err());
                db.execute("INSERT INTO authorizations VALUES('revoked',2)", [])
                    .unwrap();
                let started = context.elapsed_micros();
                context
                    .record_response(
                        "attempt".into(),
                        started,
                        context.elapsed_micros(),
                        Ok(json!({"observed_micros":context.elapsed_micros(),"state":{"pools":[]}})),
                        &Err(anyhow::anyhow!("fixture transport failure")),
                    )
                    .await
                    .unwrap();
                assert!(context.save_receipt("attempt", receipt).is_err());
            })
            .await;
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_receipt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let raw: String = db
            .query_row(
                "SELECT detail FROM events WHERE kind='application_receipt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(event["receipt"]["classification"], "transport_unknown");
        let started = event["receipt"]["signed_request_started_micros"]
            .as_u64()
            .unwrap();
        assert!(
            event["receipt"]["response_observed_micros"]
                .as_u64()
                .unwrap()
                >= started
        );
    }
    #[tokio::test]
    async fn lifecycle_observation_failure_preserves_the_payment_receipt() {
        use base64::Engine as _;
        let (_dir, guard, db) = fixture(true);
        let claim = claim(guard).await;
        claim
            .invoke(async {
                let context = payment_context().unwrap().unwrap();
                context.record(&attempt("1")).unwrap();
                let transaction = format!("0x{:064x}", 1);
                let header = base64::engine::general_purpose::STANDARD.encode(
                    json!({"success":true,"network":"eip155:8453","transaction":transaction})
                        .to_string(),
                );
                let response = reqwest::Response::from(
                    axum::http::Response::builder()
                        .status(200)
                        .header("payment-response", header)
                        .body("")
                        .unwrap(),
                );
                let started = context.elapsed_micros();
                context
                    .record_response(
                        "attempt".into(),
                        started,
                        context.elapsed_micros(),
                        Err(anyhow::anyhow!("fixture state read failed")),
                        &Ok(response),
                    )
                    .await
                    .unwrap();
                let raw: String = db
                    .query_row(
                        "SELECT detail FROM events WHERE kind='application_receipt'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                let event: Value = serde_json::from_str(&raw).unwrap();
                assert_eq!(event["receipt"]["classification"], "seller_success");
                assert_eq!(event["receipt"]["transaction"], transaction);
                assert_eq!(event["receipt"]["lifecycle"]["status"], "unavailable");
            })
            .await;
    }
    #[tokio::test]
    async fn concurrent_unsigned_and_paid_scopes_do_not_leak_payment_permission() {
        let (_unsigned_dir, unsigned, _) = fixture(false);
        let (_paid_dir, paid, _) = fixture(true);
        let unsigned = claim(unsigned).await;
        let paid = claim(paid).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        tokio::join!(
            unsigned.invoke(async {
                barrier.wait().await;
                assert!(unsigned_case());
                assert!(payment_context().is_err());
            }),
            paid.invoke(async {
                barrier.wait().await;
                assert!(!unsigned_case());
                assert!(payment_context().unwrap().is_some());
            }),
        );
        assert!(!unsigned_case());
    }
    #[tokio::test]
    async fn revocation_between_claim_and_admission_refuses_payment_correlation() {
        let (_dir, guard, db) = fixture(true);
        let claim = claim(guard).await;
        db.execute("INSERT INTO authorizations VALUES('revoked',2)", [])
            .unwrap();
        claim
            .invoke(async {
                assert!(
                    payment_context()
                        .unwrap()
                        .unwrap()
                        .record(&attempt("1"))
                        .is_err()
                );
            })
            .await;
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_payment'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn unsigned_case_on_managed_client_never_loads_a_candidate_or_retries_402() {
        use crate::{
            payment::{PaidClient, SpendPolicy},
            rotation::{
                base::BaseRpc,
                manager::ManagedPool,
                store::{Store, StoreHandle},
            },
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (_dir, guard, db) = fixture(false);
        let claim = claim(guard).await;
        let wallet_dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            &wallet_dir.path().join("state"),
            &wallet_dir.path().join("key"),
            1,
            b"fixture",
        )
        .unwrap();
        let (handle, worker) = StoreHandle::spawn(store);
        let pool = ManagedPool::new(
            handle,
            "nonexistent_pool".into(),
            BaseRpc::new("http://127.0.0.1:1", 1, 60).unwrap(),
            "1",
            SpendPolicy::dollars("1").unwrap(),
            1,
        )
        .unwrap();
        let client =
            PaidClient::managed(Arc::new(pool)).with_transport(crate::network::HttpPolicy {
                allow_http1: true,
                allow_tls12: false,
            });
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move |headers: axum::http::HeaderMap| {
                let observed = observed.clone();
                async move {
                    assert!(!headers.contains_key("payment-signature"));
                    observed.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::PAYMENT_REQUIRED
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let error = claim
            .invoke(client.execute_response(crate::catalog::RoutedRequest {
                method: "GET".into(),
                url,
                query: Default::default(),
                body: None,
            }))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("qualification_payment_denied"),
            "{error:#}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        let raw: String = db
            .query_row(
                "SELECT detail FROM events WHERE kind='application_challenge'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(event["observation"]["status"], "header_absent");
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_payment'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(client);
        worker.await.unwrap();
        server.abort();
        let _ = server.await;
    }
}
