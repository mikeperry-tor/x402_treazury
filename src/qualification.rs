//! Opt-in keyless qualification admission and private structured events.
//! The parent owns the registry lock. This child may only claim an already
//! dispatched case, once, and append its application observations.
mod balance;
mod observation;
pub(crate) use observation::pool_observation;
pub mod lifecycle;
mod lifecycle_estimate;
pub use balance::validate_admission_balance;
pub use lifecycle_estimate::depletion_estimate;
mod debit;
pub mod funding;
pub use debit::{receipt_expectation, reconcile_debit};
mod payment;
mod pricing;
mod receipt;
pub use payment::{HelpCache, payment_context, record_challenge, record_help, unsigned_case};
pub use pricing::{PricingStage, record as record_pricing};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
const LIMIT: usize = 16 * 1024 * 1024;
static ACTIVE: OnceLock<Arc<Guard>> = OnceLock::new();
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    HttpConnect,
    HttpTimeout,
    HttpStatus,
    Other,
}
pub fn failure_category(error: &anyhow::Error) -> FailureCategory {
    for cause in error.chain() {
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            return if http.is_timeout() {
                FailureCategory::HttpTimeout
            } else if http.is_connect() {
                FailureCategory::HttpConnect
            } else if http.is_status() {
                FailureCategory::HttpStatus
            } else {
                FailureCategory::Other
            };
        }
    }
    FailureCategory::Other
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub version: u32,
    pub run: String,
    pub session: String,
    pub registry: PathBuf,
    pub pin_digest: String,
    pub config_sha256: String,
    pub expires_at: i64,
}
struct Guard {
    binding: Binding,
    slots: Arc<tokio::sync::Semaphore>,
    origin: Instant,
    managed: bool,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn private(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "qualification evidence must be an ordinary file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            m.nlink() == 1 && m.mode() & 0o077 == 0 && m.uid() == unsafe { libc::geteuid() },
            "qualification evidence must be owner-only and singly linked"
        );
    }
    Ok(())
}
fn bounded(text: String) -> Result<Value> {
    ensure!(
        text.len() <= LIMIT,
        "qualification registry document exceeds {LIMIT}-byte limit"
    );
    Ok(serde_json::from_str(&text)?)
}
fn connection(binding: &Binding) -> Result<Connection> {
    private(&binding.registry)?;
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", binding.registry.display()));
        if sidecar.try_exists()? || sidecar.is_symlink() {
            private(&sidecar)?;
        }
    }
    let canonical_parent = binding
        .registry
        .parent()
        .context("registry parent missing")?
        .canonicalize()?;
    let database = canonical_parent.join(
        binding
            .registry
            .file_name()
            .context("registry filename missing")?,
    );
    let db = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")?;
    Ok(db)
}
fn now() -> Result<i64> {
    Ok(i64::try_from(crate::rotation::base::now()?)?)
}
fn authority(db: &Connection, binding: &Binding, at: i64) -> Result<Value> {
    ensure!(
        at < binding.expires_at,
        "qualification start authority expired"
    );
    let registered:String=db.query_row("SELECT detail FROM events WHERE run=?1 AND kind='application_session' AND json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.session')=?2 ORDER BY seq DESC LIMIT 1",params![binding.run,binding.session],|r|r.get(0))?;
    ensure!(
        bounded(registered)? == serde_json::to_value(binding)?,
        "qualification session binding differs from registry"
    );
    let (raw, digest): (String, String) = db.query_row(
        "SELECT payload,digest FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
        [&binding.run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    use sha2::{Digest, Sha256};
    ensure!(
        digest == binding.pin_digest && format!("{:x}", Sha256::digest(raw.as_bytes())) == digest,
        "qualification pin binding changed"
    );
    let pins = bounded(raw)?;
    let build: crate::build_identity::BuildIdentity =
        serde_json::from_value(pins["catalogs"]["build"].clone())?;
    ensure!(
        build == crate::build_identity::current(),
        "qualification executable build binding differs"
    );
    let (raw, auth): (String, String) = db.query_row(
        "SELECT manifest,authorization FROM runs WHERE id=?1",
        [&binding.run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let current: String = db.query_row(
        "SELECT id FROM authorizations ORDER BY seq DESC LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    ensure!(auth == current, "qualification registry authority changed");
    let manifest = bounded(raw)?;
    let treasury: String =
        db.query_row("SELECT treasury FROM identity WHERE singleton=1", [], |r| {
            r.get(0)
        })?;
    ensure!(
        manifest["treasury_id"] == treasury,
        "qualification treasury binding differs"
    );
    Ok(manifest)
}
fn require_unsigned(manifest: &Value) -> Result<()> {
    ensure!(
        manifest["cases"]
            .as_array()
            .context("qualification cases missing")?
            .iter()
            .all(|c| c["unsigned"] == true),
        "qualification child requires all cases unsigned"
    );
    Ok(())
}
pub fn install(path: &Path, config: &Path) -> Result<()> {
    install_mode(path, config, false)
}
pub fn install_managed(path: &Path, config: &Path) -> Result<()> {
    install_mode(path, config, true)
}
pub fn managed_permits() -> Option<Arc<dyn crate::rotation::restriction::FundingPermits>> {
    ACTIVE.get().filter(|g| g.managed).map(|g| {
        Arc::new(funding::registry::RegistryPermits::new(g.binding.clone()))
            as Arc<dyn crate::rotation::restriction::FundingPermits>
    })
}

fn install_mode(path: &Path, config: &Path, managed: bool) -> Result<()> {
    use std::io::Read;
    private(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= LIMIT,
        "qualification binding exceeds {LIMIT}-byte limit"
    );
    let binding: Binding = serde_json::from_slice(&bytes)?;
    ensure!(
        binding.version == 1 && identifier(&binding.run) && identifier(&binding.session),
        "invalid qualification binding"
    );
    ensure!(
        crate::build_identity::inputs::file_hash(config)? == binding.config_sha256,
        "qualification configuration binding differs"
    );
    let db = connection(&binding)?;
    let manifest = authority(&db, &binding, now()?)?;
    if !managed {
        require_unsigned(&manifest)?;
    }
    // The parent creates this unique index under exclusive supervisor ownership.
    let index: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='application_claim_once'",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        index == 1,
        "qualification registry lacks durable single-claim enforcement"
    );
    if managed {
        let indexes: i64 = db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('application_payment_case_once','application_payment_attempt_once')", [], |r| r.get(0))?;
        ensure!(
            indexes == 2,
            "qualification registry lacks single-payment enforcement"
        );
    }
    ensure!(
        ACTIVE
            .set(Arc::new(Guard {
                binding,
                managed,
                origin: Instant::now(),
                slots: Arc::new(tokio::sync::Semaphore::new(64))
            }))
            .is_ok(),
        "qualification guard already installed"
    );
    Ok(())
}
pub struct Claim {
    guard: Arc<Guard>,
    case: String,
    started: Instant,
    unsigned: bool,
}
pub async fn claim(
    server: &str,
    tool: &str,
    arguments: &serde_json::Map<String, Value>,
    id: &rmcp::model::RequestId,
) -> Result<Option<Claim>> {
    let Some(guard) = ACTIVE.get().cloned() else {
        return Ok(None);
    };
    claim_with(guard, server, tool, arguments, id)
        .await
        .map(Some)
}
async fn claim_with(
    guard: Arc<Guard>,
    server: &str,
    tool: &str,
    arguments: &serde_json::Map<String, Value>,
    id: &rmcp::model::RequestId,
) -> Result<Claim> {
    let id = serde_json::to_value(id)?;
    let case = id
        .as_str()
        .filter(|s| identifier(s))
        .context("qualification requires a reviewed string case ID")?
        .to_owned();
    let server = server.to_owned();
    let tool = tool.to_owned();
    let arguments = arguments.clone();
    let permit = guard
        .slots
        .clone()
        .try_acquire_owned()
        .context("qualification admission exceeds 64 concurrent checks")?;
    let work = guard.clone();
    let saved = case.clone();
    let unsigned = tokio::task::spawn_blocking(move || -> Result<bool> {
        let _permit=permit;
        let mut db=connection(&work.binding)?;
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let at=now()?;let manifest=authority(&tx,&work.binding,at)?;
        if !work.managed { require_unsigned(&manifest)?; }
        let reviewed=manifest["cases"].as_array().context("qualification cases missing")?.iter().find(|c|c["id"]==saved).context("unreviewed qualification case")?;
        ensure!(reviewed["server"]==server && reviewed["tool"]==tool && reviewed["arguments"]==Value::Object(arguments),"qualification case listener/tool/arguments differ from review");
        let (state,start,end):(String,i64,i64)=tx.query_row("SELECT execution,window_start,window_end FROM cases WHERE run=?1 AND id=?2",params![work.binding.run,saved],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        ensure!(state=="DISPATCHING" && at>=start && at<end,"qualification case is not dispatched within its authorized window");
        let unsigned = reviewed["unsigned"].as_bool().context("qualification case mode missing")?;
        if !unsigned { payment::reviewed_wallet(&tx, &work.binding, reviewed)?; }
        let detail=json!({"case":saved,"session":work.binding.session,"server":server,"source":reviewed["source"],"tool":tool,"mode":if unsigned { "unsigned" } else { "managed" },"started_micros":work.origin.elapsed().as_micros()});
        tx.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_claim',?2,?3)",params![work.binding.run,detail.to_string(),at]).context("qualification case already claimed or journal unavailable")?;
        tx.commit()?;Ok(unsigned)
    }).await??;
    Ok(Claim {
        guard,
        case,
        started: Instant::now(),
        unsigned,
    })
}
impl Claim {
    pub async fn finish(self, is_error: bool, failure: Option<FailureCategory>) -> Result<()> {
        ensure!(
            is_error || failure.is_none(),
            "successful completion cannot carry a failure category"
        );
        let elapsed = self.started.elapsed().as_millis();
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Completion remains permitted after expiry/revocation; this records
            // accepted work and grants no new execution authority.
            let db=connection(&self.guard.binding)?;
            let detail=json!({"case":self.case,"session":self.guard.binding.session,"is_error":is_error,"failure_category":failure,"elapsed_ms":elapsed,"finished_micros":self.guard.origin.elapsed().as_micros()});
            db.execute("INSERT INTO events(run,kind,detail,at) VALUES(?1,'application_finished',?2,?3)",params![self.guard.binding.run,detail.to_string(),now()?])?;
            Ok(())
        }).await??;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[tokio::test]
    async fn failure_categories_are_typed_and_durable_not_inferred_from_prose() {
        assert_eq!(
            failure_category(&anyhow::anyhow!("Connection refused")),
            FailureCategory::Other
        );
        let status = reqwest::Response::from(
            axum::http::Response::builder()
                .status(403)
                .body("denied")
                .unwrap(),
        )
        .error_for_status()
        .unwrap_err();
        assert_eq!(
            failure_category(&anyhow::Error::new(status)),
            FailureCategory::HttpStatus
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let context =
            crate::network::NetworkContext::new(crate::network::NetworkPolicy::default()).unwrap();
        let error = context
            .discovery(&url, Duration::from_secs(2))
            .unwrap()
            .get(&url)
            .send()
            .await
            .unwrap_err();
        let category = failure_category(&anyhow::Error::new(error).context("outer help error"));
        assert_eq!(category, FailureCategory::HttpConnect);
        let (_dir, guard, db) = fixture();
        claim_with(guard, "listener", "read", &args(), &id())
            .await
            .unwrap()
            .finish(true, Some(category))
            .await
            .unwrap();
        let recorded:String = db.query_row("SELECT json_extract(CASE WHEN json_valid(detail) THEN detail END,'$.failure_category') FROM events WHERE kind='application_finished'",[],|r|r.get(0)).unwrap();
        assert_eq!(recorded, "http_connect");
    }
    pub(super) fn fixture() -> (tempfile::TempDir, Arc<Guard>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.sqlite");
        let file = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        file.persist(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=1; CREATE TABLE pins(run TEXT,payload TEXT,digest TEXT,revision INTEGER);
        CREATE TABLE identity(singleton INTEGER,treasury TEXT);
        INSERT INTO identity VALUES(1,'treasury');
        CREATE TABLE runs(id TEXT,manifest TEXT,authorization TEXT);
        CREATE TABLE authorizations(id TEXT,seq INTEGER);
        CREATE TABLE cases(run TEXT,id TEXT,execution TEXT,window_start INTEGER,window_end INTEGER);
        CREATE TABLE events(seq INTEGER PRIMARY KEY,run TEXT,kind TEXT,detail TEXT,at INTEGER);
        CREATE UNIQUE INDEX application_claim_once ON events(run,json_extract(detail,'$.case')) WHERE kind='application_claim';
        INSERT INTO authorizations VALUES('review',1);").unwrap();
        let pins = json!({"catalogs":{"build":crate::build_identity::current()}}).to_string();
        let digest = format!("{:x}", Sha256::digest(pins.as_bytes()));
        db.execute(
            "INSERT INTO pins VALUES('run',?1,?2,1)",
            params![pins, digest],
        )
        .unwrap();
        let manifest=json!({"treasury_id":"treasury","cases":[{"id":"case","server":"listener","source":"api","tool":"read","arguments":{"n":1},"unsigned":true}]}).to_string();
        db.execute("INSERT INTO runs VALUES('run',?1,'review')", [manifest])
            .unwrap();
        db.execute(
            "INSERT INTO cases VALUES('run','case','DISPATCHING',0,?1)",
            [now().unwrap() + 600],
        )
        .unwrap();
        let guard = Arc::new(Guard {
            binding: Binding {
                version: 1,
                run: "run".into(),
                session: "session".into(),
                registry: path,
                pin_digest: digest,
                config_sha256: "unused".into(),
                expires_at: now().unwrap() + 600,
            },
            origin: Instant::now(),
            managed: false,
            slots: Arc::new(tokio::sync::Semaphore::new(64)),
        });
        db.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES('run','application_session',?1,0)",
            [serde_json::to_string(&guard.binding).unwrap()],
        )
        .unwrap();
        (dir, guard, db)
    }
    pub(super) fn id() -> rmcp::model::RequestId {
        serde_json::from_value(json!("case")).unwrap()
    }
    pub(super) fn args() -> serde_json::Map<String, Value> {
        serde_json::from_value(json!({"n":1})).unwrap()
    }
    #[tokio::test]
    async fn claims_bind_review_and_are_durably_single_use() {
        let (_dir, guard, db) = fixture();
        assert!(
            claim_with(guard.clone(), "wrong", "read", &args(), &id())
                .await
                .is_err()
        );
        assert!(
            claim_with(guard.clone(), "listener", "wrong", &args(), &id())
                .await
                .is_err()
        );
        assert!(
            claim_with(
                guard.clone(),
                "listener",
                "read",
                &serde_json::Map::new(),
                &id()
            )
            .await
            .is_err()
        );
        db.execute("UPDATE cases SET execution='UNATTEMPTED'", [])
            .unwrap();
        assert!(
            claim_with(guard.clone(), "listener", "read", &args(), &id())
                .await
                .is_err()
        );
        db.execute("UPDATE cases SET execution='DISPATCHING'", [])
            .unwrap();
        let claim = claim_with(guard.clone(), "listener", "read", &args(), &id())
            .await
            .unwrap();
        assert!(
            claim_with(guard, "listener", "read", &args(), &id())
                .await
                .is_err()
        );
        // Revocation stops starts, but must not discard completion of accepted work.
        db.execute("INSERT INTO authorizations VALUES('revoked',2)", [])
            .unwrap();
        claim.finish(false, None).await.unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }
    #[tokio::test]
    async fn expired_revoked_and_changed_pin_bindings_refuse_before_claim() {
        for mode in 0..4 {
            let (_dir, mut guard, db) = fixture();
            match mode {
                0 => Arc::get_mut(&mut guard).unwrap().binding.expires_at = 0,
                1 => {
                    db.execute("INSERT INTO authorizations VALUES('revoked',2)", [])
                        .unwrap();
                }
                2 => {
                    db.execute("UPDATE pins SET digest='changed'", []).unwrap();
                }
                _ => {
                    db.execute("UPDATE identity SET treasury='different'", [])
                        .unwrap();
                }
            }
            assert!(
                claim_with(guard, "listener", "read", &args(), &id())
                    .await
                    .is_err()
            );
            let count: i64 = db
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1);
        }
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    #[tokio::test]
    async fn competing_claims_have_one_winner_and_queue_bound_is_explicit() {
        let (_dir, guard, db) = super::tests::fixture();
        let args = super::tests::args();
        let id = super::tests::id();
        let held = guard.slots.clone().acquire_many_owned(64).await.unwrap();
        let error = claim_with(guard.clone(), "listener", "read", &args, &id)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("64 concurrent checks"));
        drop(held);
        let (a, b) = tokio::join!(
            claim_with(guard.clone(), "listener", "read", &args, &id),
            claim_with(guard, "listener", "read", &args, &id)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        a.or(b).unwrap().finish(false, None).await.unwrap();
        let claims: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind='application_claim'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(claims, 1);
    }
}
