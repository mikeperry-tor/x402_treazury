//! Durable pool bookkeeping. No network, signing, or automatic funding occurs here.
mod backup;
#[cfg(feature = "zcash")]
mod expiry;
pub mod funding;
pub mod refunds;
use super::error::AdmissionError;
use super::transaction::{OperationStatus, TransactionFacts};
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload, rand_core::RngCore},
};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

const SCHEMA: &str = "
CREATE TABLE instance(id TEXT PRIMARY KEY,version INTEGER NOT NULL CHECK(version=1),birthday INTEGER NOT NULL,network TEXT NOT NULL CHECK(network='mainnet'),account INTEGER NOT NULL CHECK(account=0));
CREATE TABLE snapshots(revision INTEGER PRIMARY KEY,bytes BLOB NOT NULL);
CREATE TABLE pools(id TEXT PRIMARY KEY,name TEXT UNIQUE NOT NULL,target TEXT NOT NULL,generation INTEGER NOT NULL DEFAULT 0,enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),bootstrapped INTEGER NOT NULL DEFAULT 0);
CREATE TABLE wallets(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),sequence INTEGER NOT NULL,address TEXT UNIQUE NOT NULL,key BLOB NOT NULL,target TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('ALLOCATED','READY','ACTIVE','RETIRED')),balance TEXT NOT NULL DEFAULT '0',block_hash TEXT,block_height INTEGER,UNIQUE(pool_id,sequence));
CREATE UNIQUE INDEX one_active ON wallets(pool_id) WHERE role='ACTIVE';
CREATE UNIQUE INDEX one_ready ON wallets(pool_id) WHERE role='READY';
CREATE TABLE funding_jobs(id TEXT PRIMARY KEY,wallet_id TEXT UNIQUE NOT NULL REFERENCES wallets(id),state TEXT NOT NULL CHECK(state IN ('QUEUED','COMPLETE')),target TEXT NOT NULL);
CREATE TABLE budget_entries(id TEXT PRIMARY KEY,pool_id TEXT REFERENCES pools(id),day INTEGER NOT NULL,original_day INTEGER NOT NULL,requested INTEGER NOT NULL CHECK(requested>0),reserved INTEGER NOT NULL CHECK(reserved>=0),consumed INTEGER NOT NULL CHECK(consumed>=0));
CREATE TABLE outgoing(id TEXT PRIMARY KEY REFERENCES budget_entries(id),state TEXT NOT NULL CHECK(state IN ('PREPARED','RESOLVED')),raw BLOB NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
CREATE UNIQUE INDEX one_outgoing ON outgoing((1)) WHERE state='PREPARED';
";
#[derive(Serialize)]
pub struct Status {
    pub treasury_id: String,
    pub birthday: u32,
    pub snapshot_revision: i64,
    pub pools: Vec<PoolStatus>,
    pub sync: Option<SyncObservation>,
    pub outgoing_pending: bool,
    pub sync_fresh: bool,
    pub treasury_operations: Vec<OperationStatus>,
    pub funding_jobs: Vec<funding::FundingJob>,
    pub refunds: Vec<refunds::RefundStatus>,
}
/// Persisted observations, never an authorization to spend on their own.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SyncObservation {
    pub phase: SyncPhase,
    pub last_error: Option<String>,
    pub snapshot_revision: i64,
    pub checked_at: Option<u64>,
    pub checkpoint_at: u64,
    pub scanned_blocks: u32,
    pub target_height: Option<u64>,
    pub height: Option<u64>,
    pub confirmations: u32,
    pub max_age_seconds: u64,
    #[serde(default)]
    pub confirmed_pool_balances_zatoshis: Option<PoolBalances>,
    pub confirmed_shielded_zatoshis: u64,
    pub spendable_shielded_zatoshis: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PoolBalances {
    pub ironwood: Option<u64>,
    pub orchard: Option<u64>,
    pub sapling: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Syncing,
    Preparing,
    Ready,
    Failed,
    Offline,
}
impl SyncObservation {
    pub fn fresh(&self, now: u64, revision: i64) -> bool {
        self.phase == SyncPhase::Ready
            && self.snapshot_revision == revision
            && self
                .checked_at
                .is_some_and(|at| now >= at && now - at <= self.max_age_seconds)
    }
}
#[derive(Serialize)]
pub struct PoolStatus {
    pub id: String,
    pub name: String,
    pub deposit_atomic: String,
    pub generation: i64,
    pub enabled: bool,
    pub bootstrapped: bool,
    pub funding_degraded: bool,
    pub addresses: Vec<AddressStatus>,
}
#[derive(Serialize)]
pub struct AddressStatus {
    pub id: String,
    pub address: String,
    pub role: String,
    pub target: String,
    pub confirmed_balance: String,
}
/// Production builds have exactly one treasury network. Regtest is available
/// only inside this crate's explicitly opted-in test binary.
#[derive(Clone, Copy)]
pub(crate) enum TreasuryNetwork {
    Mainnet,
    #[cfg(all(test, feature = "zcash-regtest"))]
    Regtest,
}
impl TreasuryNetwork {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            #[cfg(all(test, feature = "zcash-regtest"))]
            Self::Regtest => "regtest",
        }
    }
    #[cfg(feature = "zcash")]
    pub(crate) fn rpc_name(self) -> &'static str {
        match self {
            Self::Mainnet => "main",
            #[cfg(all(test, feature = "zcash-regtest"))]
            Self::Regtest => "test",
        }
    }
}
pub struct Store {
    network: TreasuryNetwork,
    db: Connection,
    _lock: File,
    key: Zeroizing<[u8; 32]>,
    id: String,
}
#[cfg(unix)]
fn private_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}
#[cfg(not(unix))]
fn private_options(_: &mut OpenOptions) {}
fn private_file(path: &Path, create: bool) -> Result<File> {
    if path.exists() {
        ensure!(
            fs::symlink_metadata(path)?.file_type().is_file(),
            "state path must be a regular file"
        );
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    private_options(&mut options);
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            file.metadata()?.permissions().mode() & 0o077 == 0,
            "state/key file must be owner-only"
        );
    }
    Ok(file)
}
fn directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_dir(),
        "state directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "state directory must be owner-only"
        );
    }
    Ok(())
}
fn lock(path: &Path) -> Result<File> {
    directory(path)?;
    let path = path.join("owner.lock");
    let file = private_file(&path, !path.exists())?;
    file.try_lock_exclusive().context("state_in_use")?;
    Ok(file)
}
fn configure(db: &Connection) -> Result<()> {
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    Ok(())
}
fn amount(s: &str) -> Result<U256> {
    U256::from_str_radix(s, 10).context("invalid atomic amount")
}
fn seal(key: &[u8; 32], aad: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let encrypted = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: bytes,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut output = nonce.to_vec();
    output.extend(encrypted);
    Ok(output)
}
fn unseal(key: &[u8; 32], aad: &str, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(bytes.len() >= 28, "invalid encrypted record");
    let cipher = ChaCha20Poly1305::new(key.into());
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                &Nonce::from(<[u8; 12]>::try_from(&bytes[..12]).expect("checked nonce length")),
                Payload {
                    msg: &bytes[12..],
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("wrong encryption key or corrupt state"))?,
    ))
}
impl Store {
    pub fn create(dir: &Path, key_file: &Path, birthday: u32, snapshot: &[u8]) -> Result<Self> {
        Self::create_with_network(dir, key_file, birthday, snapshot, TreasuryNetwork::Mainnet)
    }
    pub(crate) fn create_with_network(
        dir: &Path,
        key_file: &Path,
        birthday: u32,
        snapshot: &[u8],
        network: TreasuryNetwork,
    ) -> Result<Self> {
        ensure!(!snapshot.is_empty(), "treasury snapshot cannot be empty");
        ensure!(!dir.exists(), "refusing to overwrite state directory");
        ensure!(!key_file.exists(), "refusing to overwrite encryption key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(dir)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(dir)?;
        let owner = lock(dir)?;
        let mut key = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(key.as_mut());
        let mut file = private_file(key_file, true)?;
        use std::io::Write;
        file.write_all(key.as_ref())?;
        file.sync_all()?;
        if let Some(parent) = key_file.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        private_file(&dir.join("state.sqlite"), true)?.sync_all()?;
        let mut db = Connection::open(dir.join("state.sqlite"))?;
        configure(&db)?;
        let id = Uuid::new_v4().to_string();
        let encrypted = seal(
            &key,
            &format!("v1:{id}:{}:snapshot:1", network.name()),
            snapshot,
        )?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(&SCHEMA.replace(
            "network='mainnet'",
            &format!("network='{}'", network.name()),
        ))?;
        tx.execute(
            "INSERT INTO instance VALUES (?1,1,?2,?3,0)",
            params![id, birthday, network.name()],
        )?;
        tx.execute("INSERT INTO snapshots VALUES (1,?1)", [encrypted])?;
        tx.commit()?;
        admission_schema(&db)?;
        File::open(dir)?.sync_all()?;
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        Ok(Self {
            db,
            _lock: owner,
            key,
            id,
            network,
        })
    }
    pub fn open(dir: &Path, key_file: &Path, expected_id: &str) -> Result<Self> {
        Self::open_with_network(dir, key_file, expected_id, TreasuryNetwork::Mainnet)
    }
    pub(crate) fn open_with_network(
        dir: &Path,
        key_file: &Path,
        expected_id: &str,
        expected_network: TreasuryNetwork,
    ) -> Result<Self> {
        let owner = lock(dir)?;
        private_file(&dir.join("state.sqlite"), false)?;
        let mut file = private_file(key_file, false)?;
        use std::io::Read;
        let mut key = Zeroizing::new([0u8; 32]);
        file.read_exact(key.as_mut())?;
        let mut extra = [0u8; 1];
        ensure!(
            file.read(&mut extra)? == 0,
            "encryption key must be exactly 32 bytes"
        );
        let db = Connection::open_with_flags(
            dir.join("state.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        let (id, version, network, account): (String, u32, String, u32) =
            db.query_row("SELECT id,version,network,account FROM instance", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
        ensure!(
            id == expected_id,
            "treasury ID does not match this wallet; use the treasury_id from `wallet status --state-dir PATH` (it is a UUID, not an account number)"
        );
        ensure!(
            version == 1 && network == expected_network.name() && account == 0,
            "state identity/network/schema mismatch"
        );
        let store = Self {
            db,
            _lock: owner,
            key,
            id,
            network: expected_network,
        };
        store.snapshot()?;
        configure(&store.db)?;
        admission_schema(&store.db)?;
        Ok(store)
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn snapshot(&self) -> Result<(i64, Zeroizing<Vec<u8>>)> {
        let (revision, bytes): (i64, Vec<u8>) = self.db.query_row(
            "SELECT revision,bytes FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((
            revision,
            unseal(
                &self.key,
                &format!("v1:{}:{}:snapshot:{revision}", self.id, self.network.name()),
                &bytes,
            )?,
        ))
    }
    pub fn save_snapshot(&mut self, expected: i64, bytes: &[u8]) -> Result<i64> {
        self.save_sync_snapshot(expected, bytes, None)
    }
    /// Commit encrypted wallet and its observation under the same revision CAS.
    pub fn save_sync_snapshot(
        &mut self,
        expected: i64,
        bytes: &[u8],
        observation: Option<SyncObservation>,
    ) -> Result<i64> {
        self.save_wallet_snapshot(expected, bytes, observation, None)
    }
    fn save_wallet_snapshot(
        &mut self,
        expected: i64,
        bytes: &[u8],
        observation: Option<SyncObservation>,
        refund: Option<(&str, &str)>,
    ) -> Result<i64> {
        ensure!(!bytes.is_empty(), "empty snapshot");
        let revision = expected
            .checked_add(1)
            .context("snapshot revision overflow")?;
        let encrypted = seal(
            &self.key,
            &format!("v1:{}:{}:snapshot:{revision}", self.id, self.network.name()),
            bytes,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: i64 = tx.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        ensure!(current == expected, "snapshot_revision_conflict");
        tx.execute(
            "INSERT INTO snapshots VALUES (?1,?2)",
            params![revision, encrypted],
        )?;
        if let Some((job, address)) = refund {
            let encrypted = seal(
                &self.key,
                &format!("v1:{}:{}:refund:{job}", self.id, self.network.name()),
                address.as_bytes(),
            )?;
            tx.execute(
                "INSERT INTO funding_refunds VALUES (?1,?2)",
                params![job, encrypted],
            )?;
        }
        tx.execute("DELETE FROM snapshots WHERE revision < ?1 AND revision NOT IN (SELECT revision FROM outgoing UNION SELECT revision FROM expired_operations)", [revision])?;
        if let Some(mut observation) = observation {
            observation.snapshot_revision = revision;
            observation.checkpoint_at = crate::rotation::base::now()?;
            tx.execute(
                "INSERT OR REPLACE INTO treasury_sync VALUES (1,?1)",
                [serde_json::to_string(&observation)?],
            )?;
        }
        tx.commit()?;
        Ok(revision)
    }
    pub fn set_sync_phase(&mut self, phase: SyncPhase) -> Result<()> {
        if let Some(mut observation) = self.status()?.sync {
            observation.phase = phase;
            self.db.execute(
                "INSERT OR REPLACE INTO treasury_sync VALUES (1,?1)",
                [serde_json::to_string(&observation)?],
            )?;
        }
        Ok(())
    }
    pub fn require_spend_ready(&self, now: u64, input_zatoshis: u64) -> Result<()> {
        let status = self.status()?;
        ensure!(!status.outgoing_pending, "treasury_send_pending");
        let observation = status.sync.context("treasury_not_synced")?;
        ensure!(
            observation.fresh(now, status.snapshot_revision),
            "treasury_sync_stale"
        );
        ensure!(
            input_zatoshis > 0 && input_zatoshis <= observation.spendable_shielded_zatoshis,
            "treasury_insufficient_spendable_funds"
        );
        Ok(())
    }
    pub fn ensure_pool(&mut self, name: &str, deposit_size: &str) -> Result<String> {
        ensure!(
            regex::Regex::new("^[a-z][a-z0-9_]*$")?.is_match(name),
            "invalid pool name"
        );
        let target = crate::payment::SpendPolicy::dollars(deposit_size)?
            .max_atomic
            .context("deposit_size must be a decimal")?;
        ensure!(target > U256::ZERO, "deposit_size must be positive");
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id) = tx
            .query_row("SELECT id FROM pools WHERE name=?1", [name], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        {
            tx.execute(
                "UPDATE pools SET target=?1,enabled=1 WHERE id=?2",
                params![target.to_string(), id],
            )?;
            tx.commit()?;
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO pools(id,name,target) VALUES (?1,?2,?3)",
            params![id, name, target.to_string()],
        )?;
        for seq in 0..2 {
            allocate(
                &tx,
                &self.key,
                &self.id,
                self.network,
                &id,
                seq,
                &target.to_string(),
            )?;
        }
        tx.commit()?;
        Ok(id)
    }
    pub fn disable_pool(&mut self, name: &str) -> Result<()> {
        ensure!(
            self.db
                .execute("UPDATE pools SET enabled=0 WHERE name=?1", [name])?
                == 1,
            "unknown pool"
        );
        Ok(())
    }
    /// Called only by a trusted chain-reconciliation adapter after confirming credit.
    /// This module does not implement that adapter and exposes no CLI credit override.
    pub fn record_credit(
        &mut self,
        wallet: &str,
        balance: &str,
        block_hash: &str,
        height: i64,
    ) -> Result<()> {
        let balance = amount(balance)?;
        ensure!(!block_hash.is_empty(), "missing chain evidence");
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (pool, role, target): (String, String, String) = tx.query_row(
            "SELECT pool_id,role,target FROM wallets WHERE id=?1",
            [wallet],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        ensure!(
            role == "ALLOCATED",
            "credit verification requires an allocated candidate"
        );
        ensure!(balance >= amount(&target)?, "insufficient confirmed credit");
        tx.execute(
            "UPDATE wallets SET balance=?1,block_hash=?2,block_height=?3 WHERE id=?4",
            params![balance.to_string(), block_hash, height, wallet],
        )?;
        tx.execute(
            "UPDATE funding_jobs SET state='COMPLETE' WHERE wallet_id=?1",
            [wallet],
        )?;
        let bootstrapped: bool =
            tx.query_row("SELECT bootstrapped FROM pools WHERE id=?1", [&pool], |r| {
                r.get(0)
            })?;
        if bootstrapped {
            tx.execute("UPDATE wallets SET role='READY' WHERE id=?1", [wallet])?;
        } else {
            let pending:u32=tx.query_row("SELECT COUNT(*) FROM funding_jobs f JOIN wallets w ON w.id=f.wallet_id WHERE w.pool_id=?1 AND f.state!='COMPLETE'",[&pool],|r|r.get(0))?;
            if pending == 0 {
                tx.execute("UPDATE wallets SET role=CASE sequence WHEN 0 THEN 'ACTIVE' ELSE 'READY' END WHERE pool_id=?1",[&pool])?;
                tx.execute("UPDATE pools SET bootstrapped=1 WHERE id=?1", [&pool])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Offline role-transition primitive. Managed serving uses `admit` to commit
    /// verified evidence, promotion and the payment reservation together.
    pub fn promote(&mut self, pool: &str, expected: i64) -> Result<i64> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (generation, enabled, target): (i64, bool, String) = tx.query_row(
            "SELECT generation,enabled,target FROM pools WHERE id=?1",
            [pool],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        ensure!(enabled, "pool disabled");
        ensure!(generation == expected, "pool_generation_conflict");
        let ready: String = tx
            .query_row(
                "SELECT id FROM wallets WHERE pool_id=?1 AND role='READY'",
                [pool],
                |r| r.get(0),
            )
            .context("standby_not_ready")?;
        ensure!(
            tx.execute(
                "UPDATE wallets SET role='RETIRED' WHERE pool_id=?1 AND role='ACTIVE'",
                [pool]
            )? == 1,
            "active wallet missing"
        );
        tx.execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [ready])?;
        let seq: i64 = tx.query_row(
            "SELECT MAX(sequence)+1 FROM wallets WHERE pool_id=?1",
            [pool],
            |r| r.get(0),
        )?;
        allocate(&tx, &self.key, &self.id, self.network, pool, seq, &target)?;
        let next = generation.checked_add(1).context("generation overflow")?;
        tx.execute(
            "UPDATE pools SET generation=?1 WHERE id=?2",
            params![next, pool],
        )?;
        tx.commit()?;
        Ok(next)
    }
    /// Reserve total input+fee across all pools. Unknown exposure survives UTC rollover.
    pub fn reserve(
        &mut self,
        id: &str,
        pool: Option<&str>,
        day: u32,
        zatoshis: i64,
        limit: i64,
    ) -> Result<()> {
        ensure!(zatoshis > 0 && limit > 0, "invalid ZEC budget amount");
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<(Option<String>, u32, i64, i64)> = tx
            .query_row(
                "SELECT pool_id,original_day,requested,consumed FROM budget_entries WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((p, d, r, _consumed)) = existing {
            ensure!(
                p.as_deref() == pool && d == day && r == zatoshis,
                "operation_conflict"
            );
            return Ok(());
        }
        if let Some(pool) = pool {
            let enabled: bool =
                tx.query_row("SELECT enabled FROM pools WHERE id=?1", [pool], |r| {
                    r.get(0)
                })?;
            ensure!(enabled, "pool disabled");
        }
        let total:i64=tx.query_row("SELECT COALESCE(SUM(reserved),0)+COALESCE(SUM(CASE WHEN day=?1 THEN MAX(0,consumed-COALESCE((SELECT SUM(credited) FROM refund_outputs r WHERE r.operation_id=budget_entries.id),0)) ELSE 0 END),0) FROM budget_entries",[day],|r|r.get(0))?;
        ensure!(
            total.checked_add(zatoshis).is_some_and(|n| n <= limit),
            "treasury_budget_exceeded"
        );
        tx.execute(
            "INSERT INTO budget_entries VALUES (?1,?2,?3,?3,?4,?4,0)",
            params![id, pool, day, zatoshis],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Persist exact prepared bytes and the corresponding wallet snapshot in one commit.
    pub fn prepare(
        &mut self,
        id: &str,
        expected_revision: i64,
        snapshot: &[u8],
        raw: &[u8],
    ) -> Result<i64> {
        self.prepare_with_facts(id, expected_revision, snapshot, raw, None)
    }
    pub fn prepare_with_facts(
        &mut self,
        id: &str,
        expected_revision: i64,
        snapshot: &[u8],
        raw: &[u8],
        facts: Option<TransactionFacts>,
    ) -> Result<i64> {
        ensure!(
            !raw.is_empty() && !snapshot.is_empty(),
            "empty prepared operation"
        );
        if let Some((bytes, revision)) = self
            .db
            .query_row("SELECT raw,revision FROM outgoing WHERE id=?1", [id], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        {
            ensure!(
                unseal(
                    &self.key,
                    &format!("v1:{}:{}:outgoing:{id}", self.id, self.network.name()),
                    &bytes
                )?
                .as_slice()
                    == raw,
                "operation_conflict"
            );
            let saved: Vec<u8> = self.db.query_row(
                "SELECT bytes FROM snapshots WHERE revision=?1",
                [revision],
                |r| r.get(0),
            )?;
            ensure!(
                unseal(
                    &self.key,
                    &format!("v1:{}:{}:snapshot:{revision}", self.id, self.network.name()),
                    &saved
                )?
                .as_slice()
                    == snapshot,
                "operation_conflict"
            );
            if let Some(facts) = facts {
                ensure!(self.operation(id)?.facts == facts, "operation_conflict");
            }
            return Ok(revision);
        }
        let next = expected_revision
            .checked_add(1)
            .context("snapshot revision overflow")?;
        let encrypted = seal(
            &self.key,
            &format!("v1:{}:{}:outgoing:{id}", self.id, self.network.name()),
            raw,
        )?;
        let snap = seal(
            &self.key,
            &format!("v1:{}:{}:snapshot:{next}", self.id, self.network.name()),
            snapshot,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n: u32 = tx.query_row(
            "SELECT COUNT(*) FROM outgoing WHERE state='PREPARED'",
            [],
            |r| r.get(0),
        )?;
        ensure!(n == 0, "treasury_send_pending");
        let current: i64 = tx.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
        ensure!(current == expected_revision, "snapshot_revision_conflict");
        let reserved: i64 = tx.query_row(
            "SELECT reserved FROM budget_entries WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        ensure!(reserved > 0, "operation has no reservation");
        let enabled:bool=tx.query_row("SELECT COALESCE(p.enabled,1) FROM budget_entries b LEFT JOIN pools p ON p.id=b.pool_id WHERE b.id=?1",[id],|r|r.get(0))?;
        ensure!(enabled, "pool disabled");
        let funding: Option<(String, String)> = tx.query_row("SELECT j.phase,w.role FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.operation_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((phase, role)) = funding {
            ensure!(
                serde_json::from_str::<funding::FundingPhase>(&phase)?
                    == funding::FundingPhase::Preparing
                    && role == "ALLOCATED",
                "funding candidate changed during preparation"
            );
        }
        tx.execute("INSERT INTO snapshots VALUES (?1,?2)", params![next, snap])?;
        tx.execute(
            "INSERT INTO outgoing VALUES (?1,'PREPARED',?2,?3)",
            params![id, encrypted, next],
        )?;
        if let Some(facts) = facts {
            let cost = facts
                .amount_zatoshis
                .checked_add(facts.fee_zatoshis)
                .context("source cost overflow")?;
            ensure!(
                cost > 0 && cost <= reserved as u64 && facts.expiry_height > 0,
                "invalid source cost or expiry"
            );
            tx.execute(
                "INSERT INTO treasury_operations VALUES (?1,?2,'PREPARED',0)",
                params![id, serde_json::to_string(&facts)?],
            )?;
            tx.execute(
                "UPDATE budget_entries SET reserved=?1 WHERE id=?2",
                params![i64::try_from(cost)?, id],
            )?;
        }
        tx.execute("UPDATE funding_progress SET phase='\"PREPARED\"' WHERE operation_id=?1 AND phase='\"PREPARING\"'", [id])?;
        tx.commit()?;
        Ok(next)
    }
    pub fn operation_pending(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state='PREPARED')",
            [id],
            |r| r.get(0),
        )?)
    }
    pub fn prepared_bytes(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes: Vec<u8> =
            self.db
                .query_row("SELECT raw FROM outgoing WHERE id=?1", [id], |r| r.get(0))?;
        unseal(
            &self.key,
            &format!("v1:{}:{}:outgoing:{id}", self.id, self.network.name()),
            &bytes,
        )
    }
    /// Future verified reconciliation calls this after confirming source spend.
    pub fn confirm_spend(&mut self, id: &str, actual: i64, confirmation_day: u32) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state: String =
            tx.query_row("SELECT state FROM outgoing WHERE id=?1", [id], |r| r.get(0))?;
        let (reserved, consumed): (i64, i64) = tx.query_row(
            "SELECT reserved,consumed FROM budget_entries WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if state == "RESOLVED" {
            ensure!(consumed == actual, "operation_conflict");
            return Ok(());
        }
        ensure!(
            actual > 0 && actual <= reserved,
            "source cost exceeds reservation"
        );
        tx.execute("UPDATE outgoing SET state='RESOLVED' WHERE id=?1", [id])?;
        tx.execute(
            "UPDATE treasury_operations SET submission='CONFIRMED' WHERE id=?1",
            [id],
        )?;
        tx.execute(
            "UPDATE budget_entries SET reserved=0,consumed=?1,day=MAX(day,?3) WHERE id=?2",
            params![actual, id, confirmation_day],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn wallet_secret(&self, pool: &str, wallet: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes: Vec<u8> = self.db.query_row(
            "SELECT key FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet, pool],
            |r| r.get(0),
        )?;
        unseal(
            &self.key,
            &format!("v1:{}:{}:{pool}:evm:{wallet}", self.id, self.network.name()),
            &bytes,
        )
    }
    pub fn status(&self) -> Result<Status> {
        read_status(&self.db)
    }
}
fn allocate(
    db: &Connection,
    key: &[u8; 32],
    treasury: &str,
    network: TreasuryNetwork,
    pool: &str,
    seq: i64,
    target: &str,
) -> Result<()> {
    let signer = PrivateKeySigner::random();
    let id = Uuid::new_v4().to_string();
    let secret = Zeroizing::new(signer.to_bytes().to_vec());
    let encrypted = seal(
        key,
        &format!("v1:{treasury}:{}:{pool}:evm:{id}", network.name()),
        &secret,
    )?;
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,?2,?3,?4,?5,?6,'ALLOCATED')",params![id,pool,seq,signer.address().to_string(),encrypted,target])?;
    let job = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO funding_jobs VALUES (?1,?2,'QUEUED',?3)",
        params![job, id, target],
    )?;
    db.execute(
        "INSERT INTO funding_progress(job_id,operation_id,phase) VALUES (?1,?2,'\"ALLOCATED\"')",
        params![job, Uuid::new_v4().to_string()],
    )?;
    Ok(())
}
pub fn status(dir: &Path) -> Result<Status> {
    directory(dir)?;
    let path = dir.join("state.sqlite");
    private_file(&path, false)?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.unchecked_transaction()?;
    let status = read_status(&tx)?;
    tx.commit()?;
    Ok(status)
}
fn read_status(db: &Connection) -> Result<Status> {
    let (id, birthday, version): (String, u32, u32) =
        db.query_row("SELECT id,birthday,version FROM instance", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    ensure!(version == 1, "unsupported state schema");
    let revision = db.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
    let mut stmt = db.prepare(
        "SELECT id,name,target,generation,enabled,bootstrapped FROM pools ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(PoolStatus {
            id: r.get(0)?,
            name: r.get(1)?,
            deposit_atomic: r.get(2)?,
            generation: r.get(3)?,
            enabled: r.get(4)?,
            bootstrapped: r.get(5)?,
            funding_degraded: false,
            addresses: vec![],
        })
    })?;
    let mut pools = Vec::new();
    for row in rows {
        let mut pool = row?;
        let mut stmt = db.prepare(
            "SELECT id,address,role,target,balance FROM wallets WHERE pool_id=?1 ORDER BY sequence",
        )?;
        pool.addresses = stmt
            .query_map([&pool.id], |r| {
                Ok(AddressStatus {
                    id: r.get(0)?,
                    address: r.get(1)?,
                    role: r.get(2)?,
                    target: r.get(3)?,
                    confirmed_balance: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        pools.push(pool);
    }
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let sync: Option<SyncObservation> = if version >= 2 {
        db.query_row(
            "SELECT observation FROM treasury_sync WHERE singleton=1",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|s| serde_json::from_str(&s))
        .transpose()?
    } else {
        None
    };
    let outgoing_pending = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
        [],
        |r| r.get(0),
    )?;
    let sync_fresh = sync.as_ref().is_some_and(|observation| {
        crate::rotation::base::now().is_ok_and(|now| observation.fresh(now, revision))
    });
    let treasury_operations = if version >= 3 {
        let mut stmt =
            db.prepare("SELECT id,facts,submission,attempts FROM treasury_operations ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| -> Result<OperationStatus> {
            let (operation_id, facts, mut submission, attempts) = row?;
            if version >= 8
                && db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM expired_operations WHERE id=?1)",
                    [&operation_id],
                    |r| r.get::<_, bool>(0),
                )?
            {
                submission = "EXPIRED".into();
            }
            Ok(OperationStatus {
                operation_id,
                facts: serde_json::from_str(&facts)?,
                submission,
                attempts,
            })
        })
        .collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    let funding_jobs = if version >= 4 {
        funding::read_jobs(db)?
    } else {
        vec![]
    };
    for pool in &mut pools {
        pool.funding_degraded = funding_jobs.iter().any(|j| {
            j.pool_id == pool.id
                && (j.timed_out
                    || matches!(
                        j.phase,
                        funding::FundingPhase::RecoveryRequired
                            | funding::FundingPhase::RefundPending
                    ))
        });
    }
    Ok(Status {
        refunds: if version >= 7 {
            refunds::read_refunds(db)?
        } else {
            vec![]
        },
        funding_jobs,
        treasury_operations,
        sync_fresh,
        sync,
        outgoing_pending,
        treasury_id: id,
        birthday,
        snapshot_revision: revision,
        pools,
    })
}

// The async facade serializes all SQL on a dedicated blocking task. A cancelled
// caller drops only its response receiver; an accepted transaction still finishes.
type Work = Box<dyn FnOnce(&mut Store) + Send>;
#[derive(Clone)]
pub struct StoreHandle {
    sender: tokio::sync::mpsc::Sender<Work>,
}
impl StoreHandle {
    pub fn spawn(store: Store) -> (Self, tokio::task::JoinHandle<()>) {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Work>(32);
        let task = tokio::task::spawn_blocking(move || {
            let mut store = store;
            while let Some(work) = receiver.blocking_recv() {
                work(&mut store);
            }
        });
        (Self { sender }, task)
    }
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (send, recv) = tokio::sync::oneshot::channel();
        self.sender
            .send(Box::new(move |store| {
                let _ = send.send(f(store));
            }))
            .await
            .map_err(|_| anyhow::anyhow!("store worker stopped"))?;
        recv.await.context("store worker stopped")?
    }
}

// Additive admission schema, versioned independently of the encrypted record format.
fn admission_schema(db: &Connection) -> Result<()> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version <= 9, "unsupported state schema");
    if version == 0 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE payment_attempts(id TEXT PRIMARY KEY,pool_id TEXT NOT NULL REFERENCES pools(id),wallet_id TEXT NOT NULL REFERENCES wallets(id),generation INTEGER NOT NULL,amount TEXT NOT NULL,requirements_hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('ADMITTED','POSSIBLY_SUBMITTED','RESOLVED')),payer TEXT,payee TEXT,nonce TEXT,valid_after INTEGER,valid_before INTEGER,UNIQUE(wallet_id,nonce));
CREATE TABLE payment_anchors(pool_id TEXT PRIMARY KEY REFERENCES pools(id),height INTEGER NOT NULL,hash TEXT NOT NULL);
PRAGMA user_version=1;
COMMIT;")?;
    }
    if version < 2 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE treasury_sync(singleton INTEGER PRIMARY KEY CHECK(singleton=1),observation TEXT NOT NULL);
PRAGMA user_version=2;
COMMIT;")?;
    }
    if version < 3 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE treasury_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),facts TEXT NOT NULL,submission TEXT NOT NULL CHECK(submission IN ('PREPARED','BROADCAST_REQUESTED','BROADCAST','UNKNOWN','CONFIRMED')),attempts INTEGER NOT NULL DEFAULT 0);
PRAGMA user_version=3;
COMMIT;")?;
    }
    if version < 4 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS funding_progress(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),operation_id TEXT NOT NULL UNIQUE,phase TEXT NOT NULL,quote BLOB,attempts INTEGER NOT NULL DEFAULT 0,next_poll INTEGER NOT NULL DEFAULT 0,last_error TEXT,turn INTEGER NOT NULL DEFAULT 0);
PRAGMA user_version=4;
COMMIT;")?;
    }
    if version < 5 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS funding_refunds(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),address BLOB NOT NULL);
PRAGMA user_version=5;
COMMIT;")?;
    }
    if version < 6 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS funding_recovery(operation_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES funding_jobs(id),phase TEXT NOT NULL,quote BLOB,refund BLOB);
PRAGMA user_version=6;
COMMIT;")?;
    }
    if version < 7 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS refund_outputs(txid TEXT NOT NULL,output_index INTEGER NOT NULL,operation_id TEXT NOT NULL REFERENCES treasury_operations(id),amount INTEGER NOT NULL CHECK(amount>0),height INTEGER NOT NULL,credited INTEGER NOT NULL CHECK(credited>=0),PRIMARY KEY(txid,output_index));
PRAGMA user_version=7;
COMMIT;")?;
    }
    if version < 8 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS expired_operations(id TEXT PRIMARY KEY REFERENCES outgoing(id),height INTEGER NOT NULL,revision INTEGER NOT NULL REFERENCES snapshots(revision));
PRAGMA user_version=8;
COMMIT;")?;
    }
    if version < 9 {
        db.execute_batch("BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS funding_health(job_id TEXT PRIMARY KEY REFERENCES funding_jobs(id),started_at INTEGER,error_streak INTEGER NOT NULL DEFAULT 0,timed_out INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO funding_health(job_id,started_at) SELECT j.job_id,unixepoch() FROM funding_progress j JOIN treasury_operations o ON o.id=j.operation_id WHERE o.attempts>0;
PRAGMA user_version=9;
COMMIT;")?;
    }
    Ok(())
}
pub(crate) struct Admission {
    pub id: String,
    pub wallet: String,
    pub generation: i64,
    pub address: String,
    pub key: Zeroizing<Vec<u8>>,
}
pub(crate) struct Authorization {
    pub payer: String,
    pub payee: String,
    pub nonce: String,
    pub valid_after: u64,
    pub valid_before: u64,
}
impl Store {
    /// Check mode conflicts before changing any pool, then disable removed profiles.
    pub fn configure_profiles(
        &mut self,
        managed: &std::collections::BTreeSet<String>,
        statics: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let names: Vec<String> = tx
            .prepare("SELECT name FROM pools")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for name in &names {
            ensure!(
                !statics.contains(name),
                "managed pool {name} cannot become static without explicit retirement"
            );
        }
        for name in names {
            if !managed.contains(&name) {
                tx.execute("UPDATE pools SET enabled=0 WHERE name=?1", [name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Called while holding this pool's payment gate. ADMITTED rows cannot have escaped
    /// the process: every send must first durably change state to POSSIBLY_SUBMITTED.
    pub fn chain_query(&mut self, pool: &str) -> Result<super::base::ChainQuery> {
        self.db.execute(
            "DELETE FROM payment_attempts WHERE pool_id=?1 AND state='ADMITTED'",
            [pool],
        )?;
        let wallets = self
            .db
            .prepare("SELECT id,address FROM wallets WHERE pool_id=?1 AND (role!='RETIRED' OR EXISTS (SELECT 1 FROM payment_attempts a WHERE a.wallet_id=wallets.id AND a.state='POSSIBLY_SUBMITTED'))")?
            .query_map([pool], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let pending = self.db.prepare("SELECT a.id,a.wallet_id,a.payer,a.nonce,a.valid_before FROM payment_attempts a WHERE pool_id=?1 AND state='POSSIBLY_SUBMITTED'")?.query_map([pool], |r| Ok(super::base::PendingAuthorization { id:r.get(0)?,wallet:r.get(1)?,payer:r.get(2)?,nonce:r.get(3)?,valid_before:sql_u64(r,4)? }))?.collect::<rusqlite::Result<_>>()?;
        let anchor = self
            .db
            .query_row(
                "SELECT height,hash FROM payment_anchors WHERE pool_id=?1",
                [pool],
                |r| {
                    Ok(super::base::Anchor {
                        height: sql_u64(r, 0)?,
                        hash: r.get(1)?,
                    })
                },
            )
            .optional()?;
        Ok(super::base::ChainQuery {
            wallets,
            pending,
            anchor,
        })
    }
    /// Chain-only reconciliation. Never admits a payment or promotes a payer.
    pub fn reconcile_pool(&mut self, pool: &str, view: super::base::ChainView) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let bootstrapped =
            tx.query_row("SELECT bootstrapped FROM pools WHERE id=?1", [pool], |r| {
                r.get(0)
            })?;
        apply_chain_view(&tx, pool, bootstrapped, &view)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn admit(
        &mut self,
        pool: &str,
        cost: U256,
        requirements_hash: &str,
        view: super::base::ChainView,
    ) -> Result<Admission> {
        ensure!(cost > U256::ZERO, "invalid payment amount");
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (mut generation, enabled, target, bootstrapped): (i64, bool, String, bool) = tx
            .query_row(
                "SELECT generation,enabled,target,bootstrapped FROM pools WHERE id=?1",
                [pool],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        ensure!(enabled, AdmissionError::WalletNotReady("pool disabled"));
        ensure!(
            cost <= amount(&target)?,
            AdmissionError::PriceLimit("payment exceeds deposit_size")
        );
        apply_chain_view(&tx, pool, bootstrapped, &view)?;
        // Commit evidence even when not ready; payment failures must not lose reconciliation.
        let active: Option<String> = tx
            .query_row(
                "SELECT id FROM wallets WHERE pool_id=?1 AND role='ACTIVE'",
                [pool],
                |r| r.get(0),
            )
            .optional()?;
        let Some(mut wallet) = active else {
            tx.commit()?;
            anyhow::bail!(AdmissionError::WalletNotReady(
                "both bootstrap addresses require confirmed funding"
            ));
        };
        let reserved = |wallet: &str| -> Result<U256> {
            let values: Vec<String> = tx
                .prepare(
                    "SELECT amount FROM payment_attempts WHERE wallet_id=?1 AND state!='RESOLVED'",
                )?
                .query_map([wallet], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            values.iter().try_fold(U256::ZERO, |sum, v| {
                sum.checked_add(amount(v)?).context("reservation overflow")
            })
        };
        let balance = view.balances[&wallet];
        let exposure = reserved(&wallet)?;
        if balance.saturating_sub(exposure) < cost {
            // A live authorization could explain depletion. Never churn a busy slot.
            if exposure > U256::ZERO {
                tx.commit()?;
                anyhow::bail!(AdmissionError::PaymentPending("unresolved authorizations"));
            }
            let ready: Option<String> = tx
                .query_row(
                    "SELECT id FROM wallets WHERE pool_id=?1 AND role='READY'",
                    [pool],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(ready) = ready else {
                tx.commit()?;
                anyhow::bail!(AdmissionError::FundingUnavailable("standby not ready"));
            };
            if view.balances[&ready].saturating_sub(reserved(&ready)?) < cost {
                tx.commit()?;
                anyhow::bail!(AdmissionError::FundingUnavailable(
                    "standby cannot cover payment"
                ));
            }
            tx.execute("UPDATE wallets SET role='RETIRED' WHERE id=?1", [&wallet])?;
            tx.execute("UPDATE wallets SET role='ACTIVE' WHERE id=?1", [&ready])?;
            let seq: i64 = tx.query_row(
                "SELECT MAX(sequence)+1 FROM wallets WHERE pool_id=?1",
                [pool],
                |r| r.get(0),
            )?;
            allocate(&tx, &self.key, &self.id, self.network, pool, seq, &target)?;
            generation = generation.checked_add(1).context("generation overflow")?;
            tx.execute(
                "UPDATE pools SET generation=?1 WHERE id=?2",
                params![generation, pool],
            )?;
            wallet = ready;
        }
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state) VALUES (?1,?2,?3,?4,?5,?6,'ADMITTED')",params![id,pool,wallet,generation,cost.to_string(),requirements_hash])?;
        tx.commit()?;
        let address =
            self.db
                .query_row("SELECT address FROM wallets WHERE id=?1", [&wallet], |r| {
                    r.get(0)
                })?;
        let key = self.wallet_secret(pool, &wallet)?;
        Ok(Admission {
            id,
            wallet,
            generation,
            address,
            key,
        })
    }
    pub(crate) fn journal_authorization(
        &mut self,
        id: &str,
        wallet: &str,
        generation: i64,
        requirements_hash: &str,
        auth: Authorization,
    ) -> Result<()> {
        ensure!(
            auth.valid_before > auth.valid_after,
            "invalid authorization interval"
        );
        let changed=self.db.execute("UPDATE payment_attempts SET state='POSSIBLY_SUBMITTED',payer=?1,payee=?2,nonce=?3,valid_after=?4,valid_before=?5 WHERE id=?6 AND wallet_id=?7 AND generation=?8 AND requirements_hash=?9 AND state='ADMITTED'",params![auth.payer,auth.payee,auth.nonce,i64::try_from(auth.valid_after)?,i64::try_from(auth.valid_before)?,id,wallet,generation,requirements_hash])?;
        ensure!(changed == 1, "payment_journal_conflict");
        Ok(())
    }
}

fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    row.get::<_, i64>(index)?.try_into().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(e),
        )
    })
}

impl Store {
    pub fn operation(&self, id: &str) -> Result<OperationStatus> {
        self.status()?
            .treasury_operations
            .into_iter()
            .find(|o| o.operation_id == id)
            .context("operation metadata missing")
    }
    pub fn abandon_unprepared(&mut self, id: &str) -> Result<()> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        ensure!(!exists, "cannot abandon prepared transaction");
        self.db.execute(
            "DELETE FROM budget_entries WHERE id=?1 AND consumed=0",
            [id],
        )?;
        Ok(())
    }
    /// The network request may occur only after this returns successfully.
    pub fn request_broadcast(
        &mut self,
        id: &str,
        now: u64,
        height: u64,
        retry: bool,
    ) -> Result<i64> {
        ensure!(self.operation_pending(id)?, "transaction is not pending");
        let operation = self.operation(id)?;
        ensure!(now < operation.facts.deadline, "funding_deadline_expired");
        ensure!(
            height < u64::from(operation.facts.expiry_height),
            "transaction_expired"
        );
        ensure!(
            retry || operation.attempts == 0,
            "explicit_rebroadcast_required"
        );
        if operation.attempts == 0 {
            let enabled: bool = self.db.query_row("SELECT COALESCE(p.enabled,1) FROM budget_entries b LEFT JOIN pools p ON p.id=b.pool_id WHERE b.id=?1", [id], |r| r.get(0))?;
            ensure!(enabled, "pool disabled");
            let candidate: Option<bool> = self.db.query_row("SELECT f.state!='COMPLETE' AND w.role='ALLOCATED' FROM funding_progress j JOIN funding_jobs f ON f.id=j.job_id JOIN wallets w ON w.id=f.wallet_id WHERE j.operation_id=?1", [id], |r| r.get(0)).optional()?;
            ensure!(
                candidate.unwrap_or(true),
                "funding candidate no longer needs deposit"
            );
            ensure!(
                operation.facts.deadline - now >= super::transaction::MIN_QUOTE_VALIDITY_SECONDS,
                "funding_deadline_too_close"
            );
        }
        let attempt = operation
            .attempts
            .checked_add(1)
            .context("attempt overflow")?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE treasury_operations SET submission='BROADCAST_REQUESTED',attempts=?1 WHERE id=?2", params![attempt,id])?;
        tx.execute("INSERT INTO funding_health(job_id,started_at) SELECT job_id,?2 FROM funding_progress WHERE operation_id=?1 ON CONFLICT(job_id) DO UPDATE SET started_at=COALESCE(started_at,excluded.started_at)",params![id,i64::try_from(now)?])?;
        tx.commit()?;
        Ok(attempt)
    }
    pub fn broadcast_result(&mut self, id: &str, attempt: i64, accepted: bool) -> Result<()> {
        ensure!(self.db.execute("UPDATE treasury_operations SET submission=?1 WHERE id=?2 AND attempts=?3 AND submission='BROADCAST_REQUESTED'", params![if accepted {"BROADCAST"} else {"UNKNOWN"},id,attempt])? == 1, "submission_attempt_conflict");
        Ok(())
    }
}

fn apply_chain_view(
    tx: &Connection,
    pool: &str,
    bootstrapped: bool,
    view: &super::base::ChainView,
) -> Result<()> {
    for id in &view.released {
        tx.execute("UPDATE payment_attempts SET state='RESOLVED' WHERE id=?1 AND pool_id=?2 AND state='POSSIBLY_SUBMITTED'",params![id,pool])?;
    }
    let rows: Vec<(String, String, String)> = tx
        .prepare("SELECT id,role,target FROM wallets WHERE pool_id=?1 ORDER BY sequence")?
        .query_map([pool], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, role, target) in &rows {
        let Some(balance) = view.balances.get(id) else {
            ensure!(role == "RETIRED", "incomplete Base view");
            continue;
        };
        tx.execute(
            "UPDATE wallets SET balance=?1,block_hash=?2,block_height=?3 WHERE id=?4",
            params![
                balance.to_string(),
                view.anchor.hash,
                i64::try_from(view.anchor.height)?,
                id
            ],
        )?;
        if role == "ALLOCATED" && *balance >= amount(target)? {
            tx.execute(
                "UPDATE funding_jobs SET state='COMPLETE' WHERE wallet_id=?1",
                [id],
            )?;
            if bootstrapped {
                tx.execute("UPDATE wallets SET role='READY' WHERE id=?1", [id])?;
            }
        }
    }
    if !bootstrapped
        && rows.len() == 2
        && rows
            .iter()
            .all(|(id, _, target)| view.balances[id] >= amount(target).unwrap_or(U256::MAX))
    {
        tx.execute("UPDATE wallets SET role=CASE sequence WHEN 0 THEN 'ACTIVE' ELSE 'READY' END WHERE pool_id=?1",[pool])?;
        tx.execute("UPDATE pools SET bootstrapped=1 WHERE id=?1", [pool])?;
    }
    tx.execute("INSERT INTO payment_anchors VALUES (?1,?2,?3) ON CONFLICT(pool_id) DO UPDATE SET height=excluded.height,hash=excluded.hash",params![pool,i64::try_from(view.anchor.height)?,view.anchor.hash])?;
    Ok(())
}
