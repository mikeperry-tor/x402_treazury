//! Durable pool bookkeeping. No network, signing, or automatic funding occurs here.
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload, rand_core::RngCore},
};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
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
}
#[derive(Serialize)]
pub struct PoolStatus {
    pub id: String,
    pub name: String,
    pub deposit_atomic: String,
    pub generation: i64,
    pub enabled: bool,
    pub bootstrapped: bool,
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
pub struct Store {
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
        let encrypted = seal(&key, &format!("v1:{id}:mainnet:snapshot:1"), snapshot)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(SCHEMA)?;
        tx.execute(
            "INSERT INTO instance VALUES (?1,1,?2,'mainnet',0)",
            params![id, birthday],
        )?;
        tx.execute("INSERT INTO snapshots VALUES (1,?1)", [encrypted])?;
        tx.commit()?;
        File::open(dir)?.sync_all()?;
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        Ok(Self {
            db,
            _lock: owner,
            key,
            id,
        })
    }
    pub fn open(dir: &Path, key_file: &Path, expected_id: &str) -> Result<Self> {
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
            id == expected_id && version == 1 && network == "mainnet" && account == 0,
            "state identity/network/schema mismatch"
        );
        let store = Self {
            db,
            _lock: owner,
            key,
            id,
        };
        store.snapshot()?;
        configure(&store.db)?;
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
                &format!("v1:{}:mainnet:snapshot:{revision}", self.id),
                &bytes,
            )?,
        ))
    }
    pub fn save_snapshot(&mut self, expected: i64, bytes: &[u8]) -> Result<i64> {
        ensure!(!bytes.is_empty(), "empty snapshot");
        let revision = expected
            .checked_add(1)
            .context("snapshot revision overflow")?;
        let encrypted = seal(
            &self.key,
            &format!("v1:{}:mainnet:snapshot:{revision}", self.id),
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
        tx.commit()?;
        Ok(revision)
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
            allocate(&tx, &self.key, &self.id, &id, seq, &target.to_string())?;
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
    /// Atomic role transition only. Admission and reservation checks must precede
    /// this call in a future payment manager; this is not wired to paid transport.
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
        allocate(&tx, &self.key, &self.id, pool, seq, &target)?;
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
        let total:i64=tx.query_row("SELECT COALESCE(SUM(reserved),0)+COALESCE(SUM(CASE WHEN day=?1 THEN consumed ELSE 0 END),0) FROM budget_entries",[day],|r|r.get(0))?;
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
                    &format!("v1:{}:mainnet:outgoing:{id}", self.id),
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
                    &format!("v1:{}:mainnet:snapshot:{revision}", self.id),
                    &saved
                )?
                .as_slice()
                    == snapshot,
                "operation_conflict"
            );
            return Ok(revision);
        }
        let next = expected_revision
            .checked_add(1)
            .context("snapshot revision overflow")?;
        let encrypted = seal(
            &self.key,
            &format!("v1:{}:mainnet:outgoing:{id}", self.id),
            raw,
        )?;
        let snap = seal(
            &self.key,
            &format!("v1:{}:mainnet:snapshot:{next}", self.id),
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
        tx.execute("INSERT INTO snapshots VALUES (?1,?2)", params![next, snap])?;
        tx.execute(
            "INSERT INTO outgoing VALUES (?1,'PREPARED',?2,?3)",
            params![id, encrypted, next],
        )?;
        tx.commit()?;
        Ok(next)
    }
    pub fn prepared_bytes(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes: Vec<u8> =
            self.db
                .query_row("SELECT raw FROM outgoing WHERE id=?1", [id], |r| r.get(0))?;
        unseal(
            &self.key,
            &format!("v1:{}:mainnet:outgoing:{id}", self.id),
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
            &format!("v1:{}:mainnet:{pool}:evm:{wallet}", self.id),
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
    pool: &str,
    seq: i64,
    target: &str,
) -> Result<()> {
    let signer = PrivateKeySigner::random();
    let id = Uuid::new_v4().to_string();
    let secret = Zeroizing::new(signer.to_bytes().to_vec());
    let encrypted = seal(
        key,
        &format!("v1:{treasury}:mainnet:{pool}:evm:{id}"),
        &secret,
    )?;
    db.execute("INSERT INTO wallets(id,pool_id,sequence,address,key,target,role) VALUES (?1,?2,?3,?4,?5,?6,'ALLOCATED')",params![id,pool,seq,signer.address().to_string(),encrypted,target])?;
    db.execute(
        "INSERT INTO funding_jobs VALUES (?1,?2,'QUEUED',?3)",
        params![Uuid::new_v4().to_string(), id, target],
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
    Ok(Status {
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
