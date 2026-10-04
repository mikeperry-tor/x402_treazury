//! No network or signing. Reservations survive errors, cancellations and restarts.
use super::{
    files,
    manifest::{Manifest, identifier, usdc as atomic_usdc, zec as atomic_zec},
    planner::Plan,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
#[path = "registry_application.rs"]
mod application;
#[path = "registry_baseline.rs"]
mod baseline;
#[path = "registry_cases.rs"]
mod cases;
#[cfg(test)]
pub use cases::Outcome;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub version: u32,
    pub id: String,
    pub treasury_id: String,
    pub cumulative_api_usdc: String,
    pub cumulative_source_zec: String,
    pub cumulative_new_jobs: u32,
}
impl Authorization {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && identifier(&self.id),
            "invalid authorization version/id"
        );
        ensure!(
            !uuid::Uuid::parse_str(&self.treasury_id)?.is_nil(),
            "nil treasury UUID"
        );
        usdc(&self.cumulative_api_usdc)?;
        zec(&self.cumulative_source_zec)?;
        ensure!(
            self.cumulative_new_jobs <= 10000,
            "registry funding-job limit exceeds 10000"
        );
        ensure!(
            (self.cumulative_new_jobs == 0) == (zec(&self.cumulative_source_zec)? == 0),
            "job and source limits must both be zero or positive"
        );
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pins {
    pub binary_sha256: String,
    pub cargo_lock_sha256: String,
    pub source_revision: String,
    pub source_dirty: bool,
    pub resolved_config: Value,
    /// M2 records configuration only. Catalog/schema qualification is required before execution.
    pub qualification: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogs: Option<super::preparation::CatalogPins>,
}
impl Pins {
    fn validate(&self) -> Result<()> {
        for digest in [&self.binary_sha256, &self.cargo_lock_sha256] {
            ensure!(
                digest.len() == 64 && digest.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid SHA256 pin"
            );
        }
        ensure!(
            !self.source_revision.is_empty() && self.source_revision.len() <= 128,
            "invalid source revision pin"
        );
        Ok(())
    }
}
pub struct Registry {
    db: Connection,
    _lock: files::Ownership,
    root: PathBuf,
}
const SCHEMA:&str = "
PRAGMA user_version=1;
CREATE TABLE identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),treasury TEXT NOT NULL,baseline TEXT NOT NULL,baseline_hash TEXT NOT NULL);
CREATE TABLE authorizations(seq INTEGER PRIMARY KEY,id TEXT UNIQUE NOT NULL,api INTEGER NOT NULL CHECK(api>=0),source INTEGER NOT NULL CHECK(source>=0),jobs INTEGER NOT NULL CHECK(jobs>=0),at INTEGER NOT NULL);
CREATE TABLE runs(id TEXT PRIMARY KEY,manifest TEXT NOT NULL,plan TEXT NOT NULL,authorization TEXT NOT NULL REFERENCES authorizations(id),at INTEGER NOT NULL);
CREATE TABLE pins(run TEXT NOT NULL REFERENCES runs(id),revision INTEGER NOT NULL,payload TEXT NOT NULL,digest TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(run,revision));
CREATE TABLE cases(run TEXT NOT NULL REFERENCES runs(id),id TEXT NOT NULL,reservation INTEGER NOT NULL CHECK(reservation>=0),window_start INTEGER NOT NULL,window_end INTEGER NOT NULL,phase TEXT NOT NULL,execution TEXT NOT NULL DEFAULT 'UNATTEMPTED',semantic TEXT NOT NULL DEFAULT 'UNOBSERVED',settlement TEXT NOT NULL DEFAULT 'UNOBSERVED',result_hash TEXT,PRIMARY KEY(run,id));
CREATE TABLE batches(run TEXT NOT NULL REFERENCES runs(id),id TEXT NOT NULL,members TEXT NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(run,id));
CREATE TABLE events(seq INTEGER PRIMARY KEY,run TEXT,kind TEXT NOT NULL,detail TEXT NOT NULL,at INTEGER NOT NULL);
";
impl Registry {
    pub fn authorize(state: &Path, auth: &Authorization, now: i64) -> Result<Self> {
        ensure!(now >= 0, "invalid registry timestamp");
        auth.validate()?;
        files::directory(state)?;
        let root = state.join("live-integration");
        if root.try_exists()? || root.is_symlink() {
            let mut registry = Self::open(state, false)?;
            let _treasury_lock = files::lock(&state.join("owner.lock"))?;
            ensure!(
                x402_treazury::rotation::store::status(state)?.treasury_id == auth.treasury_id,
                "current treasury differs from registry authority"
            );
            registry.add_authorization(auth, now)?;
            return Ok(registry);
        }
        let (_treasury_lock, baseline) = baseline::capture(state, &auth.treasury_id)?;
        // Creation failure leaves evidence, never silently recreates/reset a registry.
        files::create_dir(&root)?;
        files::create_file(&root.join("owner.lock"))?.sync_all()?;
        files::create_file(&root.join("registry.sqlite"))?.sync_all()?;
        let lock = files::lock(&root.join("owner.lock"))?;
        let mut db = Connection::open_with_flags(
            root.join("registry.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        db.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE; PRAGMA foreign_keys=ON;",
        )?;
        let tx = db.transaction()?;
        tx.execute_batch(SCHEMA)?;
        let raw = bounded_json(&baseline)?;
        tx.execute(
            "INSERT INTO identity VALUES(1,?1,?2,?3)",
            params![auth.treasury_id, raw, files::hash(raw.as_bytes())],
        )?;
        tx.commit()?;
        let mut registry = Self {
            db,
            _lock: lock,
            root,
        };
        registry.add_authorization(auth, now)?;
        File::open(&registry.root)?.sync_all()?;
        Ok(registry)
    }
    pub fn open(state: &Path, read_only: bool) -> Result<Self> {
        files::directory(state)?;
        let root = state.join("live-integration");
        files::directory(&root)?;
        for name in [
            "owner.lock",
            "registry.sqlite",
            "registry.sqlite-journal",
            "registry.sqlite-wal",
            "registry.sqlite-shm",
        ] {
            let p = root.join(name);
            if p.try_exists()? || p.is_symlink() {
                files::regular(&p)?;
            }
        }
        let lock = files::lock(&root.join("owner.lock"))?;
        let db = Connection::open_with_flags(
            root.join("registry.sqlite"),
            if read_only {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE
            },
        )?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version == 1,
            "unsupported/incomplete integration registry schema"
        );
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        if !read_only {
            db.execute_batch("PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;")?;
        }
        let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        ensure!(check == "ok", "integration registry integrity failure");
        Ok(Self {
            db,
            _lock: lock,
            root,
        })
    }
    fn add_authorization(&mut self, a: &Authorization, now: i64) -> Result<()> {
        ensure!(now >= 0, "invalid registry timestamp");
        a.validate()?;
        let tx = self.db.transaction()?;
        let treasury: String = tx.query_row("SELECT treasury FROM identity", [], |r| r.get(0))?;
        ensure!(treasury == a.treasury_id, "registry treasury mismatch");
        let consumed: i64 = tx.query_row(
            "SELECT COALESCE(SUM(reservation),0) FROM cases WHERE execution!='UNATTEMPTED'",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            usdc(&a.cumulative_api_usdc)? >= consumed,
            "authorization cannot erase consumed reservations"
        );
        // Positive funding is stored authority only; M5 will add transactional permits.
        tx.execute(
            "INSERT INTO authorizations(id,api,source,jobs,at) VALUES(?1,?2,?3,?4,?5)",
            params![
                a.id,
                usdc(&a.cumulative_api_usdc)?,
                zec(&a.cumulative_source_zec)?,
                a.cumulative_new_jobs,
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO events(kind,detail,at) VALUES('authorization',?1,?2)",
            params![bounded_json(a)?, now],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn prepare(&mut self, plan: &Plan, pins: &Pins, now: i64) -> Result<()> {
        ensure!(now >= 0, "invalid registry timestamp");
        pins.validate()?;
        let m = &plan.manifest;
        m.validate()?;
        let verified = super::planner::build(m, &pins.resolved_config)?;
        ensure!(
            bounded_json(&verified)? == bounded_json(plan)?,
            "plan and pinned configuration disagree"
        );
        ensure!(
            (pins.qualification == "configuration_only" && pins.catalogs.is_none())
                || (pins.qualification == "catalogs_prepared" && pins.catalogs.is_some()),
            "invalid catalog qualification state"
        );
        if let Some(catalogs) = &pins.catalogs {
            catalogs.verify(m, pins)?;
        }
        let tx = self.db.transaction()?;
        let treasury: String = tx.query_row("SELECT treasury FROM identity", [], |r| r.get(0))?;
        ensure!(treasury == m.treasury_id, "prepared run treasury mismatch");
        let (id, api, source, jobs): (String, i64, i64, u32) = tx.query_row(
            "SELECT id,api,source,jobs FROM authorizations ORDER BY seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        ensure!(
            id == m.registry_authorization,
            "run must reference current explicit authorization"
        );
        ensure!(
            usdc(&m.limits.api_reservation_usdc)? <= api
                && zec(&m.limits.source_exposure_zec)? <= source
                && m.limits.new_funding_jobs <= jobs,
            "run limits exceed cumulative authority"
        );
        ensure!(
            m.windows.iter().any(|w| w.not_after > now as u64),
            "all prepared execution windows have expired"
        );
        tx.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,?4,?5)",
            params![
                m.run_id,
                bounded_json(m)?,
                bounded_json(plan)?,
                m.registry_authorization,
                now
            ],
        )?;
        insert_pins(&tx, &m.run_id, 1, pins, now)?;
        for p in &m.phases {
            let w = m
                .windows
                .iter()
                .find(|w| w.id == p.window)
                .context("unknown phase window")?;
            for id in &p.cases {
                let c = m
                    .cases
                    .iter()
                    .find(|c| &c.id == id)
                    .context("unknown case")?;
                tx.execute("INSERT INTO cases(run,id,reservation,window_start,window_end,phase) VALUES(?1,?2,?3,?4,?5,?6)",params![m.run_id,c.id,usdc(&c.reserve_usdc)?,i64::try_from(w.not_before)?,i64::try_from(w.not_after)?,p.id])?;
            }
        }
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'prepared',?2,?3)",
            params![m.run_id, pins.qualification, now],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn revise(
        &mut self,
        run: &str,
        replacement: &Pins,
        expected: &str,
        now: i64,
    ) -> Result<Value> {
        ensure!(now >= 0, "invalid registry timestamp");
        replacement.validate()?;
        let tx = self.db.transaction()?;
        let attempted: i64 = tx.query_row(
            "SELECT COUNT(*) FROM cases WHERE run=?1 AND execution!='UNATTEMPTED'",
            [run],
            |r| r.get(0),
        )?;
        ensure!(attempted == 0, "pin revision refused after any reservation");
        let (revision, raw, digest): (i64, String, String) = tx.query_row(
            "SELECT revision,payload,digest FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        ensure!(
            digest == expected,
            "expected pin digest mismatch; review current pins first"
        );
        let old: Pins = serde_json::from_str(&raw)?;
        let mut prior = old.resolved_config.clone();
        let mut new = replacement.resolved_config.clone();
        for config in [&mut prior, &mut new] {
            if let Some(sources) = config["sources"].as_object_mut() {
                for source in sources.values_mut() {
                    source["settings"]
                        .as_object_mut()
                        .context("source settings missing")?
                        .remove("reliability_note");
                }
            }
        }
        ensure!(
            prior == new && old.qualification == replacement.qualification,
            "pin revision changes execution contract; create a new run within existing cumulative budget"
        );
        insert_pins(&tx, run, revision + 1, replacement, now)?;
        let diff = json!({"from":digest,"to":files::hash(bounded_json(replacement)?),"binary_changed":old.binary_sha256!=replacement.binary_sha256,"configuration_changed":old.resolved_config!=replacement.resolved_config,"checkout_changed":old.source_revision!=replacement.source_revision || old.source_dirty!=replacement.source_dirty});
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,'pin_revision',?2,?3)",
            params![run, bounded_json(&diff)?, now],
        )?;
        tx.commit()?;
        Ok(diff)
    }
    pub fn application_binding(
        &self,
        run: &str,
        session: &str,
        config_sha256: String,
        expires_at: i64,
    ) -> Result<x402_treazury::qualification::Binding> {
        ensure!(identifier(session), "invalid application session ID");
        let pin_digest = self.db.query_row(
            "SELECT digest FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
            [run],
            |r| r.get(0),
        )?;
        Ok(x402_treazury::qualification::Binding {
            version: 1,
            run: run.into(),
            session: session.into(),
            registry: self.root.join("registry.sqlite"),
            pin_digest,
            config_sha256,
            expires_at,
        })
    }
    pub fn begin_execution(&mut self, run: &str, now: i64) -> Result<i64> {
        let m = self.manifest(run)?;
        self.db.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS application_claim_once ON events(run,json_extract(detail,'$.case')) WHERE kind='application_claim';")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let auth: String = tx.query_row(
            "SELECT id FROM authorizations ORDER BY seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            auth == m.registry_authorization,
            "run authority changed; execution refused"
        );
        let latest: i64 = tx.query_row(
            "SELECT COALESCE(MAX(at),0) FROM events WHERE run=?1",
            [run],
            |r| r.get(0),
        )?;
        ensure!(
            now >= latest,
            "wall clock moved backwards; execution refused"
        );
        let start: Option<i64> = tx.query_row(
            "SELECT MIN(at) FROM events WHERE run=?1 AND kind='execution_started'",
            [run],
            |r| r.get(0),
        )?;
        let start = start.unwrap_or(now);
        let end = start
            .checked_add(i64::try_from(m.limits.run_seconds)?)
            .context("execution deadline overflow")?;
        ensure!(now < end, "run execution deadline expired");
        tx.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,?2,'{}',?3)",
            params![
                run,
                if start == now {
                    "execution_started"
                } else {
                    "execution_resumed"
                },
                now
            ],
        )?;
        tx.commit()?;
        Ok(end)
    }
    pub fn pins(&self, run: &str) -> Result<Pins> {
        let (raw, digest): (String, String) = self.db.query_row(
            "SELECT payload,digest FROM pins WHERE run=?1 ORDER BY revision DESC LIMIT 1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(files::hash(&raw) == digest, "registry pin digest mismatch");
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn event(&self, run: &str, kind: &str, detail: &Value, now: i64) -> Result<()> {
        ensure!(now >= 0 && identifier(kind), "invalid registry event");
        self.db.execute(
            "INSERT INTO events(run,kind,detail,at) VALUES(?1,?2,?3,?4)",
            params![run, kind, bounded_json(detail)?, now],
        )?;
        Ok(())
    }
    pub fn execution_state(&self, run: &str, id: &str) -> Result<String> {
        Ok(self.db.query_row(
            "SELECT execution FROM cases WHERE run=?1 AND id=?2",
            params![run, id],
            |r| r.get(0),
        )?)
    }
    pub fn manifest(&self, run: &str) -> Result<Manifest> {
        Ok(serde_json::from_str(&self.db.query_row::<String, _, _>(
            "SELECT manifest FROM runs WHERE id=?1",
            [run],
            |r| r.get(0),
        )?)?)
    }
    pub fn report(&self, run: Option<&str>, now: i64) -> Result<Value> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM cases WHERE (?1 IS NULL OR run=?1)",
            [run],
            |r| r.get(0),
        )?;
        ensure!(
            count <= 10000,
            "report exceeds 10000 cases; select a single run instead"
        );
        let pin_count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM pins WHERE (?1 IS NULL OR run=?1)",
            [run],
            |r| r.get(0),
        )?;
        ensure!(
            pin_count <= 10000,
            "report exceeds 10000 pin revisions; select a single run instead"
        );
        let mut stmt=self.db.prepare("SELECT run,id,reservation,window_start,window_end,execution,semantic,settlement,result_hash FROM cases WHERE (?1 IS NULL OR run=?1) ORDER BY run,id")?;
        let rows=stmt.query_map([run],|r|{
            let state:String=r.get(5)?;let start:i64=r.get(3)?;let end:i64=r.get(4)?;
            Ok(json!({"run":r.get::<_,String>(0)?,"case":r.get::<_,String>(1)?,"reservation_atomic":r.get::<_,i64>(2)?,"execution":state,"semantic":r.get::<_,String>(6)?,"settlement":r.get::<_,String>(7)?,"result_hash":r.get::<_,Option<String>>(8)?,"eligibility":if state!="UNATTEMPTED"{"observe_only"}else if now<start{"waiting_window"}else if now>=end{"expired_window"}else{"requires_execution_qualification"}}))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        let consumed: i64 = self.db.query_row(
            "SELECT COALESCE(SUM(reservation),0) FROM cases WHERE execution!='UNATTEMPTED'",
            [],
            |r| r.get(0),
        )?;
        let (auth, ceiling, source_ceiling, job_ceiling): (String, i64, i64, i64) =
            self.db.query_row(
                "SELECT id,api,source,jobs FROM authorizations ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let mut pins=self.db.prepare("SELECT run,revision,digest FROM pins WHERE (?1 IS NULL OR run=?1) ORDER BY run,revision")?;
        let revisions=pins.query_map([run],|r|Ok(json!({"run":r.get::<_,String>(0)?,"revision":r.get::<_,i64>(1)?,"digest":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if let Some(id) = run {
            let _ = self.manifest(id)?;
        }
        let mut events = self.db.prepare("SELECT run,kind,detail,at FROM events WHERE (?1 IS NULL OR run=?1) AND kind IN ('execution_started','execution_resumed','mcp_dispatch_intent','mcp_finished','mcp_failure','child_finished','application_claim','application_finished','tor_outage_started','tor_outage_qualified') ORDER BY seq LIMIT 50001")?;
        let runtime_events = events
            .query_map([run], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            runtime_events.len() <= 50000,
            "report exceeds 50000 runtime events; select a single run"
        );
        let runtime_events = runtime_events.into_iter().map(|(run,kind,detail,at)| Ok(json!({"run":run,"kind":kind,"detail":serde_json::from_str::<Value>(&detail)?,"at":at}))).collect::<Result<Vec<_>>>()?;
        let execution_available = match run {
            Some(id) => self.pins(id)?.catalogs.is_some(),
            None => false,
        };
        let application_evidence = if execution_available {
            match self.application_evidence(run.context("run missing")?) {
                Ok(value) => value,
                Err(error) => json!({"status":"invalid_or_incomplete","reason":error.to_string()}),
            }
        } else {
            Value::Null
        };
        let report = json!({"version":1,"authorization":auth,"api_reserved_atomic":consumed,"api_ceiling_atomic":ceiling,"source_ceiling_zatoshis":source_ceiling,"new_job_ceiling":job_ceiling,"api_headroom_atomic":ceiling.checked_sub(consumed).context("registry reservation integrity failure")?,"cases":rows,"pin_revisions":revisions,"execution_available":execution_available,"runtime_events":runtime_events,"application_evidence":application_evidence,"qualification_scope":if execution_available { "unsigned_mcp_only" } else { "offline_configuration" },"tor_isolation":"not_qualified","funding_permits_available":false,"baseline":"immutable private evidence; live reconciliation required","next_step":if execution_available { "run dispatches only unattempted unsigned cases; no payment or Tor isolation qualification" } else { "prepare-catalogs is required before unsigned execution" }});
        bounded_json(&report)?;
        Ok(report)
    }
}
fn bounded_json(value: &impl Serialize) -> Result<String> {
    let s = serde_json::to_string(value)?;
    ensure!(
        s.len() <= files::DOCUMENT_BYTES,
        "registry document exceeds {} byte limit",
        files::DOCUMENT_BYTES
    );
    Ok(s)
}
fn insert_pins(db: &Connection, run: &str, revision: i64, pins: &Pins, now: i64) -> Result<()> {
    let raw = bounded_json(pins)?;
    db.execute(
        "INSERT INTO pins VALUES(?1,?2,?3,?4,?5)",
        params![run, revision, raw, files::hash(raw.as_bytes()), now],
    )?;
    Ok(())
}

fn usdc(s: &str) -> Result<i64> {
    Ok(atomic_usdc(s)?.try_into()?)
}
fn zec(s: &str) -> Result<i64> {
    Ok(atomic_zec(s)?.try_into()?)
}
