//! Durable pool bookkeeping. No network, signing, or automatic funding occurs here.
mod backup;
mod capacity;
#[cfg(test)]
mod io_failure_tests;
mod preflight;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
pub(crate) mod test_vfs;
#[cfg(feature = "zcash")]
mod treasury_reads;
pub use capacity::AllocationKind;
mod evidence;
pub use evidence::BalanceSupport;
pub(crate) use evidence::repair::repair_wallet;
pub(crate) mod bootstrap;
#[cfg(feature = "zcash")]
mod expiry;
pub mod funding;
pub(crate) mod history;
mod ownership;
mod reporting;
pub use reporting::{BalancePointStatus, RetirementAccounting, RuntimeBalanceEvidence};
pub mod refunds;
pub(crate) mod scheduling;
pub(crate) mod selection;
use super::error::AdmissionError;
use super::transaction::{OperationStatus, TransactionFacts};
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload, rand_core::RngCore},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

mod qualification;
pub use qualification::qualification_state;

const SCHEMA: &str = include_str!("store/schema.sql");
#[derive(Serialize)]
pub struct Status {
    pub schema_version: u32,
    pub state_schema_version: u32,
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
    /// Final indexer tip; absent in historical snapshots. Never a scanned height.
    #[serde(default)]
    pub observed_tip_height: Option<u64>,
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
    #[cfg(feature = "zcash")]
    pub(crate) fn at_observed_tip(&self) -> bool {
        self.height.is_some() && self.height == self.observed_tip_height
    }

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
    /// Legacy reporting metadata, not payment admission authority.
    pub generation: i64,
    pub enabled: bool,
    /// None is unconfigured; U256::MAX represents explicit unlimited authority.
    pub payment_limit_atomic: Option<String>,
    pub retirement_tolerance_atomic: Option<String>,
    pub retirement_accounting: RetirementAccounting,
    pub serving_ready: bool,
    pub inventory_ready: bool,
    pub reserve_claims: Vec<ReserveClaimStatus>,
    pub retained_wallets: usize,
    pub retired_wallets: usize,
    pub funding_degraded: bool,
    pub addresses: Vec<AddressStatus>,
}
#[derive(Serialize)]
pub struct ReserveClaimStatus {
    pub slot: String,
    pub enabled: bool,
    pub generation: i64,
    pub wallet_id: Option<String>,
    pub target_atomic: String,
    pub maximum_output_atomic: String,
    pub destination_ready: bool,
}
#[derive(Serialize)]
pub struct RetirementStatus {
    pub confirmed_amount: String,
    pub latest_amount: String,
    pub accepted_maximum: String,
    pub observed_bound: String,
    pub confirmed_height: i64,
    pub confirmed_hash: String,
    pub latest_height: i64,
    pub latest_hash: String,
    pub disputed: Option<String>,
    pub recorded_at: i64,
}
#[derive(Serialize)]
pub struct AddressStatus {
    pub id: String,
    pub address: String,
    pub role: String,
    /// Original immutable allocation target, never current payment capacity.
    pub target: String,
    /// Last saved confirmed observation, not fresh admission authority.
    pub confirmed_balance: String,
    pub confirmed_height: Option<i64>,
    pub confirmed_hash: Option<String>,
    pub signed_exposure_atomic: Option<String>,
    pub financial_repair: Option<String>,
    pub destination_recorded: bool,
    pub destination_disputed: Option<String>,
    pub retirement: Option<RetirementStatus>,
    /// None means this reader has no current-process evidence, not zero capacity.
    pub runtime_balance_evidence: Option<RuntimeBalanceEvidence>,
}
/// Production builds have exactly one treasury network. Synthetic and consensus
/// test chains are available only inside explicitly opted-in test binaries.
#[derive(Clone, Copy)]
pub(crate) enum TreasuryNetwork {
    Mainnet,
    #[cfg(all(test, feature = "zcash-testutils"))]
    Synthetic,
    #[cfg(all(test, feature = "zcash-regtest"))]
    Regtest,
}
impl TreasuryNetwork {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            #[cfg(all(test, feature = "zcash-testutils"))]
            Self::Synthetic => "synthetic",
            #[cfg(all(test, feature = "zcash-regtest"))]
            Self::Regtest => "regtest",
        }
    }
    #[cfg(feature = "zcash")]
    pub(crate) fn rpc_name(self) -> &'static str {
        match self {
            Self::Mainnet => "main",
            #[cfg(all(test, feature = "zcash-testutils"))]
            Self::Synthetic => "regtest",
            #[cfg(all(test, feature = "zcash-regtest"))]
            Self::Regtest => "test",
        }
    }
}
pub struct Store {
    funding_restriction: super::restriction::FundingRestriction,
    funding_permits: Option<std::sync::Arc<dyn super::restriction::FundingPermits>>,
    network: TreasuryNetwork,
    evidence_namespace: Uuid,
    evidence_cache: std::collections::BTreeMap<String, (u64, super::base::BalanceCertificate)>,
    evidence_order: u64,
    // Diagnostic freshness only: never spending authority. Historical shortages
    // must be retried by this owner before they can obstruct startup.
    funding_conditions:
        std::collections::BTreeMap<String, (String, super::error::FundingCondition)>,
    group_gates: GroupGates,
    db: Connection,
    // Keep ownership until SQLite has closed (fields drop in declaration order).
    _lock: ownership::Owner,
    key: Zeroizing<[u8; 32]>,
    id: String,
}
#[cfg(unix)]
fn private_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
}
#[cfg(not(unix))]
fn private_options(_: &mut OpenOptions) {}
fn validate_private_metadata(metadata: &fs::Metadata) -> Result<()> {
    ensure!(
        metadata.file_type().is_file(),
        "state path must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.nlink() == 1,
            "state/key file must not have hard links"
        );
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "state/key file must be owner-only"
        );
    }
    Ok(())
}
fn existing_private_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_private_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn database_paths(dir: &Path) -> Result<()> {
    for name in [
        "state.sqlite",
        "state.sqlite-wal",
        "state.sqlite-shm",
        "state.sqlite-journal",
    ] {
        existing_private_path(&dir.join(name))?;
    }
    Ok(())
}
fn private_file(path: &Path, create: bool) -> Result<File> {
    existing_private_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    private_options(&mut options);
    let file = options.open(path)?;
    validate_private_metadata(&file.metadata()?)?;
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
fn lock(path: &Path) -> Result<ownership::Owner> {
    directory(path)?;
    let path = path.join("owner.lock");
    let file = private_file(&path, !path.exists())?;
    ownership::Owner::acquire(file)
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
        tx.execute_batch(include_str!("store/evidence/repair_schema.sql"))?;
        tx.execute(
            "INSERT INTO instance VALUES (?1,1,?2,?3,0)",
            params![id, birthday, network.name()],
        )?;
        tx.execute("INSERT INTO snapshots VALUES (1,?1)", [encrypted])?;
        tx.commit()?;
        evidence::initialize_runtime(&db)?;
        File::open(dir)?.sync_all()?;
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(parent)?.sync_all()?;
        }
        Ok(Self {
            funding_restriction: Default::default(),
            funding_permits: None,
            evidence_namespace: Uuid::new_v4(),
            evidence_cache: Default::default(),
            evidence_order: 0,
            funding_conditions: Default::default(),
            group_gates: Default::default(),
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
        directory(dir)?;
        let owner = ownership::Owner::acquire(private_file(&dir.join("owner.lock"), false)?)?;
        database_paths(dir)?;
        private_file(&dir.join("state.sqlite"), false)?;
        {
            let preflight = preflight::snapshot(dir)?;
            let version: u32 = preflight
                .db
                .query_row("PRAGMA user_version", [], |r| r.get(0))?;
            ensure!(
                version == 16,
                "unsupported state schema; stopped-service conversion required"
            );
        }
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
            funding_restriction: Default::default(),
            funding_permits: None,
            evidence_namespace: Uuid::new_v4(),
            evidence_cache: Default::default(),
            evidence_order: 0,
            funding_conditions: Default::default(),
            group_gates: Default::default(),
            db,
            _lock: owner,
            key,
            id,
            network: expected_network,
        };
        store.snapshot()?;
        configure(&store.db)?;
        admission_schema(&store.db)?;
        evidence::initialize_runtime(&store.db)?;
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
        let observation: Option<String> = self
            .db
            .query_row(
                "SELECT observation FROM treasury_sync WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(observation) = observation {
            let mut observation: SyncObservation = serde_json::from_str(&observation)?;
            observation.phase = phase;
            self.db.execute(
                "INSERT OR REPLACE INTO treasury_sync VALUES (1,?1)",
                [serde_json::to_string(&observation)?],
            )?;
        }
        Ok(())
    }
    /// Monotonic for this store owner. A supervised launch must install it again on reopen.
    /// No setter can restore ordinary funding on an already restricted owner.
    pub fn deny_new_funding(&mut self) {
        self.funding_restriction = super::restriction::FundingRestriction::DenyNewFunding;
        tracing::warn!(
            "qualification funding restriction installed: zero new jobs and zero new ZEC spending; existing reconciliation remains available"
        );
    }
    /// Ordinary pause cannot weaken a previously installed qualification restriction.
    pub fn pause_new_funding(&mut self) {
        if self.funding_restriction != super::restriction::FundingRestriction::DenyNewFunding {
            self.funding_restriction = super::restriction::FundingRestriction::Paused;
        }
    }
    /// Install once, before pool setup. Reopening requires explicit reinstallation.
    pub fn install_funding_permits(
        &mut self,
        permits: std::sync::Arc<dyn super::restriction::FundingPermits>,
    ) -> Result<()> {
        ensure!(
            self.funding_permits.is_none(),
            "funding permits already installed"
        );
        permits.validate_treasury(&self.id)?;
        permits.validate_state(&self.qualification_state()?)?;
        self.funding_permits = Some(permits);
        Ok(())
    }
    pub fn check_job_permit(&self, job: &str, input_with_fee: u64) -> Result<()> {
        if let Some(permits) = &self.funding_permits {
            super::restriction::permit_result(
                permits.check_preparation(&self.id, job, input_with_fee),
                "preparation",
            )?;
        }
        Ok(())
    }
    fn check_operation_permit(&self, operation: &str, input_with_fee: u64) -> Result<()> {
        if let Some(permits) = &self.funding_permits {
            let job: String = self
                .db
                .query_row(
                    "SELECT job_id FROM funding_progress WHERE operation_id=?1",
                    [operation],
                    |r| r.get(0),
                )
                .context("qualification_funding_denied: operation lacks a current funding job")?;
            self.check_job_permit(&job, input_with_fee)?;
            // Recovery may have committed before the registry acknowledgement.
            // Only durable archived operations with no bytes or consumed funds
            // can release an old source reservation. Current operations cannot.
            let mut query = self.db.prepare("SELECT r.operation_id FROM funding_recovery r WHERE r.job_id=?1 AND NOT EXISTS(SELECT 1 FROM outgoing o WHERE o.id=r.operation_id) AND NOT EXISTS(SELECT 1 FROM budget_entries b WHERE b.id=r.operation_id AND b.consumed!=0)")?;
            for old in query.query_map([&job], |r| r.get::<_, String>(0))? {
                super::restriction::permit_result(
                    permits.retire_unprepared(&self.id, &job, &old?),
                    "unprepared_recovery",
                )?;
            }
            super::restriction::permit_result(
                permits.reserve_preparation(&self.id, &job, operation, input_with_fee),
                "source_reservation",
            )?;
        }
        Ok(())
    }
    pub fn require_spend_ready(&self, now: u64, input_zatoshis: u64) -> Result<()> {
        self.funding_restriction
            .require_new_funding("treasury_preparation")?;
        let outgoing_pending: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE state='PREPARED')",
            [],
            |row| row.get(0),
        )?;
        ensure!(!outgoing_pending, "treasury_send_pending");
        let observation: String = self
            .db
            .query_row(
                "SELECT observation FROM treasury_sync WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .context("treasury_not_synced")?;
        let observation: SyncObservation = serde_json::from_str(&observation)?;
        let revision: i64 = self.db.query_row(
            "SELECT revision FROM snapshots ORDER BY revision DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        ensure!(observation.fresh(now, revision), "treasury_sync_stale");
        if input_zatoshis > observation.spendable_shielded_zatoshis {
            tracing::warn!(
                category = "treasury_insufficient_spendable_funds",
                required_zatoshis = input_zatoshis,
                spendable_zatoshis = observation.spendable_shielded_zatoshis,
                confirmed_shielded_zatoshis = observation.confirmed_shielded_zatoshis,
                "Zcash treasury cannot fund this refill; fund and sync the treasury; existing funded EVM wallets remain usable"
            );
        }
        ensure!(
            input_zatoshis > 0 && input_zatoshis <= observation.spendable_shielded_zatoshis,
            super::error::FundingCondition::InsufficientFunds
        );
        Ok(())
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
    /// Unfunded unit-fixture convenience. Production requires opaque Base evidence.
    #[cfg(test)]
    pub fn record_credit(
        &mut self,
        wallet: &str,
        balance: &str,
        _block_hash: &str,
        height: i64,
    ) -> Result<()> {
        let address: String =
            self.db
                .query_row("SELECT address FROM wallets WHERE id=?1", [wallet], |r| {
                    r.get(0)
                })?;
        let height = u64::try_from(height)?;
        let point = super::base::evidence::BalancePoint {
            anchor: super::base::Anchor {
                height,
                hash: format!("0x{height:064x}"),
            },
            amount: amount(balance)?,
        };
        let support = evidence::BalanceSupport {
            checkpoint: None,
            observation: super::base::evidence::BalanceObservation {
                wallet: wallet.into(),
                address,
                network: "eip155:8453".into(),
                asset: crate::payment::USDC.into(),
                endpoint: "a".repeat(64),
                confirmed: point.clone(),
                latest: super::base::evidence::BalancePoint {
                    anchor: super::base::Anchor {
                        height: height + 1,
                        hash: format!("0x{:064x}", height + 1),
                    },
                    amount: point.amount,
                },
                confirmations: 1,
                valid_until: u64::MAX,
            },
        };
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            funding::record_readiness(&tx, wallet, &support)?,
            "insufficient confirmed credit"
        );
        tx.execute(
            "UPDATE wallets SET balance=?2,block_hash=?3,block_height=?4 WHERE id=?1",
            params![
                wallet,
                balance,
                support.observation.confirmed.anchor.hash,
                i64::try_from(height)?
            ],
        )?;
        funding::publish_destination_readiness(&tx, wallet)?;
        tx.commit()?;
        Ok(())
    }
    /// Reserve total input+fee across all pools. Unknown exposure survives UTC rollover.
    pub fn reserve(&mut self, id: &str, pool: Option<&str>, day: u32, zatoshis: i64) -> Result<()> {
        ensure!(zatoshis > 0, "invalid ZEC reservation amount");
        self.check_operation_permit(id, u64::try_from(zatoshis)?)?;
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
        self.funding_restriction
            .require_new_funding("source_reservation")?;
        if let Some(pool) = pool {
            let enabled: bool =
                tx.query_row("SELECT enabled FROM pools WHERE id=?1", [pool], |r| {
                    r.get(0)
                })?;
            ensure!(enabled, "pool disabled");
        }
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
        self.funding_restriction
            .require_new_funding("source_preparation_commit")?;
        if self.funding_permits.is_some() {
            let requested: i64 = self.db.query_row(
                "SELECT requested FROM budget_entries WHERE id=?1",
                [id],
                |r| r.get(0),
            )?;
            self.check_operation_permit(id, u64::try_from(requested)?)?;
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
        let funding: Option<String> = tx
            .query_row(
                "SELECT phase FROM funding_progress WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(phase) = funding {
            ensure!(
                serde_json::from_str::<funding::FundingPhase>(&phase)?
                    == funding::FundingPhase::Preparing,
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
    /// Proof for the pre-preparation path only; historical effects forbid deferral.
    pub fn require_unstarted_preparation(&self, id: &str) -> Result<()> {
        let touched: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE id=?1) OR EXISTS(SELECT 1 FROM budget_entries WHERE id=?1)",
            [id], |row| row.get(0),
        )?;
        ensure!(
            !touched,
            "preparation has durable effects; recovery required"
        );
        Ok(())
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
        reporting::runtime_status(self, super::base::now()?)
    }
}
fn allocation_jobs(
    permits: Option<&std::sync::Arc<dyn super::restriction::FundingPermits>>,
    db: &Connection,
    treasury: &str,
    pool: &str,
    sequences: &[i64],
) -> Result<Vec<String>> {
    let Some(permits) = permits else {
        return Ok(sequences
            .iter()
            .map(|_| Uuid::new_v4().to_string())
            .collect());
    };
    let name: String = db.query_row("SELECT name FROM pools WHERE id=?1", [pool], |r| r.get(0))?;
    let jobs = super::restriction::permit_result(
        permits.reserve_allocations(treasury, &name, sequences),
        "allocation",
    )?;
    ensure!(
        jobs.len() == sequences.len(),
        "qualification_funding_denied: incomplete allocation permits"
    );
    let mut unique = std::collections::BTreeSet::new();
    for job in &jobs {
        let id = Uuid::parse_str(job)?;
        ensure!(
            !id.is_nil() && unique.insert(id),
            "qualification_funding_denied: invalid allocation job identity"
        );
    }
    Ok(jobs)
}
fn inspect_state<T>(dir: &Path, read: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    directory(dir)?;
    database_paths(dir)?;
    let path = dir.join("state.sqlite");
    private_file(&path, false)?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.unchecked_transaction()?;
    let result = read(&tx)?;
    tx.commit()?;
    Ok(result)
}

/// Runtime identity discovery reads no financial history and grants no spending
/// authority. Opening the owner still authenticates the encrypted snapshot.
pub fn treasury_id(dir: &Path) -> Result<String> {
    inspect_state(dir, read_runtime_identity)
}

fn read_runtime_identity(db: &Connection) -> Result<String> {
    let schema: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        schema == 16,
        "unsupported state schema; stopped-service conversion required"
    );
    let mut query = db.prepare("SELECT id,version,network,account FROM instance")?;
    let mut rows = query.query([])?;
    let row = rows.next()?.context("missing state identity")?;
    let id: String = row.get(0)?;
    let version: u32 = row.get(1)?;
    let network: String = row.get(2)?;
    let account: u32 = row.get(3)?;
    ensure!(
        version == 1 && network == "mainnet" && account == 0,
        "state identity/network/schema mismatch"
    );
    ensure!(!Uuid::parse_str(&id)?.is_nil(), "invalid state identity");
    ensure!(rows.next()?.is_none(), "ambiguous state identity");
    Ok(id)
}

/// Identity and managed profile bindings from one coherent read transaction.
/// Pool names remain reserved even when their financial reports cannot decode.
pub fn static_ownership(dir: &Path) -> Result<(String, Vec<String>)> {
    inspect_state(dir, |db| {
        let id = read_runtime_identity(db)?;
        let names = db
            .prepare("SELECT name FROM pools ORDER BY name")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok((id, names))
    })
}

pub fn status(dir: &Path) -> Result<Status> {
    inspect_state(dir, read_status)
}
fn read_status(db: &Connection) -> Result<Status> {
    let (id, birthday, version): (String, u32, u32) =
        db.query_row("SELECT id,birthday,version FROM instance", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    ensure!(version == 1, "unsupported state schema");
    let revision = db.query_row("SELECT MAX(revision) FROM snapshots", [], |r| r.get(0))?;
    let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut pools = reporting::read_pools(db, version)?;
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
    let treasury_operations = reporting::read_operations(db, version)?;
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
        schema_version: 2,
        state_schema_version: version,
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
#[derive(Default)]
struct GroupOwners {
    payment: std::sync::Arc<tokio::sync::Mutex<()>>,
    reconciliation: std::sync::Arc<tokio::sync::Mutex<()>>,
    capacity_changed: std::sync::Arc<tokio::sync::Notify>,
}
type GroupGates = std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, GroupOwners>>>;
#[derive(Clone)]
pub struct StoreHandle {
    evidence_namespace: Uuid,
    sender: tokio::sync::mpsc::Sender<Work>,
    group_gates: GroupGates,
    #[cfg(feature = "zcash")]
    funding_owner: std::sync::Arc<tokio::sync::Mutex<super::funding::scheduler::Owner>>,
}
impl StoreHandle {
    pub fn spawn(store: Store) -> (Self, tokio::task::JoinHandle<()>) {
        let evidence_namespace = store.evidence_namespace;
        let group_gates = store.group_gates.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Work>(32);
        let task = tokio::task::spawn_blocking(move || {
            let mut store = store;
            while let Some(work) = receiver.blocking_recv() {
                work(&mut store);
            }
        });
        (
            Self {
                evidence_namespace,
                sender,
                group_gates,
                #[cfg(feature = "zcash")]
                funding_owner: Default::default(),
            },
            task,
        )
    }
    #[cfg(feature = "zcash")]
    pub(crate) fn funding_owner(
        &self,
    ) -> std::sync::Arc<tokio::sync::Mutex<super::funding::scheduler::Owner>> {
        self.funding_owner.clone()
    }
    pub(crate) fn runtime_namespace(&self) -> Uuid {
        self.evidence_namespace
    }
    /// All independently constructed payment owners for this store/group share
    /// the same gate, including while an accepted SQL job outlives its waiter.
    pub(crate) fn group_gate(&self, pool: &str) -> Result<std::sync::Arc<tokio::sync::Mutex<()>>> {
        let mut gates = self
            .group_gates
            .lock()
            .map_err(|_| anyhow::anyhow!("group gate registry poisoned"))?;
        Ok(gates.entry(pool.to_owned()).or_default().payment.clone())
    }
    /// Maintenance lifetime ownership is shared across separately constructed
    /// handles, but never holds the payment gate while acquiring network evidence.
    pub(crate) fn reconciliation_gate(
        &self,
        pool: &str,
    ) -> Result<std::sync::Arc<tokio::sync::Mutex<()>>> {
        let mut gates = self
            .group_gates
            .lock()
            .map_err(|_| anyhow::anyhow!("group gate registry poisoned"))?;
        Ok(gates
            .entry(pool.to_owned())
            .or_default()
            .reconciliation
            .clone())
    }
    /// Keep group ownership with an accepted store job even if its response
    /// waiter is cancelled. The caller may retain the same guard through signing
    /// and journal acknowledgement; no second lock is acquired here.
    pub(crate) async fn call_with_guard<T: Send + 'static>(
        &self,
        guard: std::sync::Arc<tokio::sync::OwnedMutexGuard<()>>,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.call(move |store| {
            let _guard = guard;
            f(store)
        })
        .await
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
    ensure!(
        version == 16,
        "unsupported state schema; use the separately reviewed stopped-service conversion utility"
    );
    db.execute_batch(include_str!("store/evidence/repair_schema.sql"))?;
    // Additive lookup optimization for existing schema16 stores; financial
    // records and the state format are unchanged.
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS wallet_role_sequence ON wallets(pool_id,role,sequence);
         CREATE INDEX IF NOT EXISTS wallet_live_sequence ON wallets(pool_id,sequence) WHERE role!='RETIRED';
         CREATE INDEX IF NOT EXISTS funding_claim_lookup ON funding_jobs(pool_id,allocation_kind,claim_generation);
         CREATE INDEX IF NOT EXISTS funding_recovery_job ON funding_recovery(job_id,operation_id);",
    )?;
    Ok(())
}
pub(crate) struct PreparedPayment {
    pub id: String,
    pub wallet: String,
    pub address: String,
    pub key: Zeroizing<Vec<u8>>,
    pool: String,
    cost: U256,
    requirements_hash: String,
    ownership: capacity::PaymentOwnership,
    balance: U256,
    reserved: U256,
    anchor: super::base::Anchor,
    pub(crate) valid_until: u64,
    support: Option<evidence::BalanceSupport>,
    certificate: Option<super::base::BalanceCertificate>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdmissionExpired;
impl std::fmt::Display for AdmissionExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("wallet_not_ready: payment evidence or unpublished authorization expired; no payment submitted")
    }
}
impl std::error::Error for AdmissionExpired {}
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
    /// Read-only query construction is also used by funding credit without the
    /// payment gate. It must never reclaim a live signer's unsigned reservation.
    pub fn chain_query(&self, pool: &str) -> Result<super::base::ChainQuery> {
        self.chain_query_inner(pool, true)
    }
    /// Historical authorizations still reconcile, but retired balances cannot
    /// fund a payment and must not delay acquisition of active admission evidence.
    #[cfg(test)]
    pub(crate) fn payment_chain_query(&self, pool: &str) -> Result<super::base::ChainQuery> {
        self.chain_query_inner(pool, false)
    }
    fn chain_query_inner(
        &self,
        pool: &str,
        retired_balances: bool,
    ) -> Result<super::base::ChainQuery> {
        let wallets: Vec<(String,String)> = self
            .db
            .prepare("SELECT id,address FROM wallets WHERE pool_id=?1 AND (role!='RETIRED' OR (?2 AND EXISTS (SELECT 1 FROM payment_attempts a WHERE a.wallet_id=wallets.id AND a.state='POSSIBLY_SUBMITTED')))")?
            .query_map(params![pool, retired_balances], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let pending: Vec<super::base::PendingAuthorization> = self.db.prepare("SELECT a.id,a.wallet_id,a.payer,a.nonce,a.valid_before FROM payment_attempts a WHERE pool_id=?1 AND state='POSSIBLY_SUBMITTED'")?.query_map([pool], |r| Ok(super::base::PendingAuthorization { id:r.get(0)?,wallet:r.get(1)?,payer:r.get(2)?,nonce:r.get(3)?,valid_before:sql_u64(r,4)? }))?.collect::<rusqlite::Result<_>>()?;
        let captures = wallets
            .iter()
            .map(|(wallet, _)| wallet)
            .chain(pending.iter().map(|auth| &auth.wallet))
            .map(|wallet| Ok((wallet.clone(), self.capture_evidence(wallet)?)))
            .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
        Ok(super::base::ChainQuery {
            wallets,
            pending,
            // Legacy pool anchors remain audit data. Verification is wallet scoped.
            anchor: None,
            captures,
        })
    }
    /// Ordinary retirement authority comes from the explicit tolerance, even
    /// while replenishment is paused. Qualification funding permits authorize
    /// allocation/preparation only; they confer no retirement mutation authority.
    fn automatic_retirement_allowed(&self) -> bool {
        self.funding_restriction != super::restriction::FundingRestriction::DenyNewFunding
            && self.funding_permits.is_none()
    }

    /// Chain-only reconciliation. Never admits a payment or promotes a payer.
    pub fn reconcile_pool(&mut self, pool: &str, view: super::base::ChainView) -> Result<()> {
        let retirement_allowed = self.automatic_retirement_allowed();
        let view = self.prepare_evidence_view(view)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let depleted = apply_chain_view(
            &tx,
            pool,
            &view,
            self.evidence_namespace,
            true,
            retirement_allowed,
        )?;
        commit_chain_view(tx, pool, &view, &depleted)?;
        Ok(())
    }
    /// Capture a bounded explicit wallet or authorization page. Historical work
    /// carries no unrelated balance inventory or pool-wide chain dependency.
    pub(crate) fn scoped_chain_query(
        &self,
        pool: &str,
        wallets: Vec<String>,
        pending: Vec<super::base::PendingAuthorization>,
    ) -> Result<super::base::ChainQuery> {
        ensure!(
            wallets.len() <= history::PAGE_SIZE && pending.len() <= history::PAGE_SIZE,
            "evidence page exceeds resource bound"
        );
        let pending = self.pending_history(pool, pending)?;
        let wallets = wallets
            .into_iter()
            .map(|wallet| {
                let address = self.db.query_row(
                    "SELECT address FROM wallets WHERE id=?1 AND pool_id=?2",
                    params![wallet, pool],
                    |row| row.get(0),
                )?;
                Ok((wallet, address))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut captures = std::collections::BTreeMap::new();
        for wallet in wallets
            .iter()
            .map(|(wallet, _)| wallet)
            .chain(pending.iter().map(|auth| &auth.wallet))
        {
            let belongs: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM wallets WHERE id=?1 AND pool_id=?2)",
                params![wallet, pool],
                |row| row.get(0),
            )?;
            ensure!(belongs, "evidence wallet group mismatch");
            captures.insert(wallet.clone(), self.capture_evidence(wallet)?);
        }
        Ok(super::base::ChainQuery {
            wallets,
            pending,
            anchor: None,
            captures,
        })
    }

    pub(crate) fn publish_scoped_evidence(
        &mut self,
        pool: &str,
        view: super::base::ChainView,
    ) -> Result<()> {
        let retirement_allowed = self.automatic_retirement_allowed();
        let view = self.prepare_evidence_view(view)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let depleted = apply_chain_view(
            &tx,
            pool,
            &view,
            self.evidence_namespace,
            false,
            retirement_allowed,
        )?;
        commit_chain_view(tx, pool, &view, &depleted)?;
        self.notify_capacity_changed(pool);
        Ok(())
    }

    /// Bind an operation's network identity before calculation; bindings are immutable.
    pub fn bind_operation_recipient(&mut self, operation: &str, recipient: &str) -> Result<()> {
        let address: alloy_primitives::Address = recipient.parse()?;
        let recipient = address.to_string();
        self.db.execute(
            "INSERT OR IGNORE INTO operation_network(operation_id,recipient) VALUES (?1,?2)",
            params![operation, recipient],
        )?;
        let saved: String = self.db.query_row(
            "SELECT recipient FROM operation_network WHERE operation_id=?1",
            [operation],
            |r| r.get(0),
        )?;
        ensure!(saved == recipient, "network_identity_conflict");
        Ok(())
    }
    pub fn operation_recipient(&self, operation: &str) -> Result<Option<String>> {
        let rows = self.db.prepare("SELECT recipient FROM operation_network WHERE operation_id=?1 UNION SELECT w.address FROM funding_progress p JOIN funding_jobs j ON j.id=p.job_id JOIN wallets w ON w.id=j.wallet_id WHERE p.operation_id=?1 UNION SELECT w.address FROM funding_recovery p JOIN funding_jobs j ON j.id=p.job_id JOIN wallets w ON w.id=j.wallet_id WHERE p.operation_id=?1")?.query_map([operation], |r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut address = None;
        for row in rows {
            let parsed: alloy_primitives::Address = row.parse()?;
            ensure!(
                address.is_none_or(|previous| previous == parsed),
                "network_identity_conflict"
            );
            address = Some(parsed);
        }
        Ok(address.map(|a| a.to_string()))
    }
    pub(crate) fn validate_payment_price(&self, pool: &str, cost: U256) -> Result<()> {
        let (enabled, limit): (bool, Option<String>) = self.db.query_row(
            "SELECT enabled,payment_limit FROM pools WHERE id=?1",
            [pool],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(enabled, AdmissionError::WalletNotReady("pool disabled"));
        let limit = limit.context(AdmissionError::WalletNotReady(
            "payment authority unavailable",
        ))?;
        ensure!(
            cost > U256::ZERO && cost <= amount(&limit)?,
            AdmissionError::PriceLimit("payment exceeds configured API payment ceiling")
        );
        Ok(())
    }
    pub(crate) fn payment_candidate(
        &mut self,
        pool: &str,
    ) -> Result<super::manager::PaymentCandidateHandle> {
        let wallet =
            self.resolve_payment_identity(pool)?
                .context(AdmissionError::WalletNotReady(
                    "funded payment identity unavailable",
                ))?;
        self.db
            .query_row(
                "SELECT address FROM wallets WHERE id=?1 AND pool_id=?2",
                params![wallet, pool],
                |r| {
                    Ok(super::manager::PaymentCandidateHandle {
                        wallet: wallet.clone(),
                        address: r.get(0)?,
                    })
                },
            )
            .map_err(Into::into)
    }
    #[cfg(test)]
    pub(crate) fn admit_for(
        &mut self,
        pool: &str,
        cost: U256,
        requirements_hash: &str,
        view: super::base::ChainView,
        expected: Option<&super::manager::PaymentCandidateHandle>,
    ) -> Result<PreparedPayment> {
        self.prepare_payment(pool, cost, requirements_hash, view, expected)
    }
    pub(crate) fn prepare_payment(
        &mut self,
        pool: &str,
        cost: U256,
        requirements_hash: &str,
        mut view: super::base::ChainView,
        expected: Option<&super::manager::PaymentCandidateHandle>,
    ) -> Result<PreparedPayment> {
        ensure!(cost > U256::ZERO, "invalid payment amount");
        let expected = match expected {
            Some(value) => value.clone(),
            None => self.payment_candidate(pool)?,
        };
        let wallet = expected.wallet.clone();
        view.balances.retain(|id, _| id == &wallet);
        view.observations.retain(|id, _| id == &wallet);
        view.balance_certificates.retain(|id, _| id == &wallet);
        let view = self.prepare_evidence_view(view)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ownership = capacity::payment_ownership(&tx, pool, &wallet)?.context(
            AdmissionError::WalletNotReady("payment wallet no longer eligible"),
        )?;
        ensure!(
            cost <= ownership.payment_limit,
            AdmissionError::PriceLimit("payment exceeds configured API payment ceiling")
        );
        let address: String = tx.query_row(
            "SELECT address FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet, pool],
            |r| r.get(0),
        )?;
        ensure!(address == expected.address, AdmissionError::PayerChanged);
        let valid_until = view
            .observations
            .get(&wallet)
            .map_or(view.admission_valid_until, |observation| {
                observation.valid_until
            });
        ensure!(
            super::base::now()? <= valid_until,
            super::base::evidence::EvidenceFailure::Expired
        );
        let balance = *view
            .balances
            .get(&wallet)
            .context(AdmissionError::WalletNotReady("payer balance unavailable"))?;
        // Evidence publication cannot allocate or consume a reserve. The immutable
        // funded-role binding was checked before accepting the observation.
        let depleted = apply_chain_view(&tx, pool, &view, self.evidence_namespace, false, false)?;
        let reserved = payment_exposure(&tx, &wallet)?;
        ensure!(
            balance.saturating_sub(reserved) >= cost,
            AdmissionError::FundingUnavailable(
                "bound payer cannot cover payment; no payment signed"
            )
        );
        commit_chain_view(tx, pool, &view, &depleted)?;
        let key = self.wallet_secret(pool, &wallet)?;
        Ok(PreparedPayment {
            id: Uuid::new_v4().to_string(),
            pool: pool.into(),
            cost,
            requirements_hash: requirements_hash.into(),
            ownership,
            balance,
            reserved,
            support: view
                .balance_certificates
                .get(&wallet)
                .map(|certificate| {
                    evidence::check_balance(&self.db, self.evidence_namespace, certificate)
                })
                .transpose()?,
            certificate: view.balance_certificates.get(&wallet).cloned(),
            anchor: view.observations.get(&wallet).map_or_else(
                || view.anchor.clone(),
                |observation| observation.confirmed.anchor.clone(),
            ),
            valid_until,
            wallet,
            address,
            key,
        })
    }
    pub(crate) fn journal_authorization(
        &mut self,
        admission: &PreparedPayment,
        auth: Authorization,
        qualification: Option<&crate::qualification::PaymentContext>,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let support = admission
            .certificate
            .as_ref()
            .map(|certificate| evidence::check_balance(&tx, self.evidence_namespace, certificate))
            .transpose()?;
        ensure!(
            support.as_ref().map(|support| &support.observation)
                == admission
                    .support
                    .as_ref()
                    .map(|support| &support.observation),
            "payment_evidence_changed"
        );
        let (enabled, role, address, balance, height, hash): (bool, String, String, String, i64, String) = tx.query_row(
            "SELECT p.enabled,w.role,w.address,w.balance,w.block_height,w.block_hash FROM wallets w JOIN pools p ON p.id=w.pool_id WHERE w.id=?1 AND p.id=?2",
            params![admission.wallet, admission.pool],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        )?;
        ensure!(
            enabled
                && role == admission.ownership.role
                && address == admission.address
                && amount(&balance)? == admission.balance
                && height == i64::try_from(admission.anchor.height)?
                && hash == admission.anchor.hash,
            "payment_admission_ownership_conflict"
        );
        let ownership = capacity::payment_ownership(&tx, &admission.pool, &admission.wallet)?
            .context("payment_admission_ownership_conflict")?;
        ensure!(
            ownership.role == admission.ownership.role
                && ownership.slot == admission.ownership.slot
                && ownership.claim_generation == admission.ownership.claim_generation,
            "payment_admission_ownership_conflict"
        );
        ensure!(
            admission.cost > U256::ZERO && admission.cost <= ownership.payment_limit,
            AdmissionError::PriceLimit("payment exceeds configured API payment ceiling")
        );
        let reserved = payment_exposure(&tx, &admission.wallet)?;
        ensure!(
            reserved == admission.reserved,
            "payment_admission_ownership_conflict"
        );
        ensure!(
            admission.balance.saturating_sub(reserved) >= admission.cost,
            AdmissionError::PaymentPending("unresolved authorizations")
        );
        ensure!(
            auth.valid_before > auth.valid_after && auth.payer == admission.address,
            "invalid authorization interval"
        );
        let clock = super::base::now()?;
        ensure!(auth.valid_after <= clock, "authorization not yet valid");
        ensure!(
            clock <= admission.valid_until && clock < auth.valid_before,
            AdmissionExpired
        );
        // Preserve legacy report metadata without using it as admission authority.
        tx.execute("INSERT INTO payment_attempts(id,pool_id,wallet_id,generation,amount,requirements_hash,state,payer,payee,nonce,valid_after,valid_before,reconciliation_sequence) VALUES (?1,?2,?3,(SELECT generation FROM pools WHERE id=?2),?4,?5,'POSSIBLY_SUBMITTED',?6,?7,?8,?9,?10,?11)",params![admission.id,admission.pool,admission.wallet,admission.cost.to_string(),admission.requirements_hash,auth.payer,auth.payee,auth.nonce,i64::try_from(auth.valid_after)?,i64::try_from(auth.valid_before)?,evidence::next_sequence(&tx)?])?;
        evidence::add_exposure(&tx, &admission.wallet, admission.cost)?;
        tx.execute(
            "INSERT INTO payment_admission_evidence(attempt_id,block_height,block_hash,balance,reserved_before,valid_until,support,latest_height,latest_hash,endpoint,payer_role,reserve_slot,claim_generation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                admission.id,
                height,
                hash,
                balance,
                reserved.to_string(),
                admission.valid_until.to_string(),
                admission.support.as_ref().map(serde_json::to_string).transpose()?,
                admission.support.as_ref().map(|support| i64::try_from(support.observation.latest.anchor.height)).transpose()?,
                admission.support.as_ref().map(|support| &support.observation.latest.anchor.hash),
                admission.support.as_ref().map(|support| &support.observation.endpoint),
                admission.ownership.role,
                admission.ownership.slot,
                admission.ownership.claim_generation,
            ],
        )?;
        capacity::consume_reserve(&tx, &admission.pool, &admission.wallet)?;
        if let Some(context) = qualification {
            // The separate qualification journal claims this case before the
            // financial commit can dispatch. A crash between databases keeps
            // the case charged, never grants permission to retry it.
            let qualification_deadline =
                context.record(&qualification::payment_evidence(&tx, &admission.id)?)?;
            let clock = super::base::now()?;
            ensure!(
                clock <= admission.valid_until
                    && clock < auth.valid_before
                    && clock < u64::try_from(qualification_deadline)?,
                AdmissionError::WalletNotReady(
                    "qualified payment authority expired before commit; no submission"
                )
            );
        }
        tx.commit()?;
        Ok(())
    }
}

fn payment_exposure(db: &Connection, wallet: &str) -> Result<U256> {
    evidence::exposure(db, wallet)
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
    pub(crate) fn operation_facts(
        &self,
        id: &str,
    ) -> Result<Option<super::transaction::TransactionFacts>> {
        let facts: Option<String> = self
            .db
            .query_row(
                "SELECT facts FROM treasury_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        facts
            .map(|facts| serde_json::from_str(&facts).map_err(Into::into))
            .transpose()
    }

    pub fn operation(&self, id: &str) -> Result<OperationStatus> {
        let (facts, submission, attempts): (String, String, i64) = self.db.query_row(
            "SELECT facts,CASE WHEN EXISTS(SELECT 1 FROM expired_operations e WHERE e.id=o.id) THEN 'EXPIRED' ELSE submission END,attempts FROM treasury_operations o WHERE id=?1",
            [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?.context("operation metadata missing")?;
        Ok(OperationStatus {
            operation_id: id.to_owned(),
            facts: serde_json::from_str(&facts)?,
            submission,
            attempts,
        })
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
            let candidate: Option<bool> = self.db.query_row("SELECT j.phase IN ('\"PREPARED\"','\"DEPOSIT_PENDING\"') FROM funding_progress j WHERE j.operation_id=?1", [id], |r| r.get(0)).optional()?;
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

#[derive(Default)]
struct ChainPublication {
    depleted: Vec<String>,
    retired_remainders: Vec<(U256, U256)>,
}

/// Emit depletion only after reconciliation has durably committed. Reservations
/// and unfunded allocations do not constitute a confirmed balance transition.
fn commit_chain_view(
    tx: rusqlite::Transaction<'_>,
    pool: &str,
    view: &super::base::ChainView,
    depleted: &ChainPublication,
) -> Result<()> {
    tx.commit()?;
    for wallet in &depleted.depleted {
        tracing::warn!(
            category = "evm_wallet_depleted",
            pool,
            wallet,
            block_height = view.anchor.height,
            balance_atomic = 0,
            "EVM wallet confirmed USDC balance reached zero; payment admission checks remaining funded capacity"
        );
    }
    for (confirmed, latest) in &depleted.retired_remainders {
        tracing::warn!(reason="automatic_retirement", confirmed_atomic=%confirmed, latest_atomic=%latest, "retained wallet retired with an observed remainder; keys and accounting remain available");
    }
    Ok(())
}

fn apply_chain_view(
    tx: &Connection,
    pool: &str,
    view: &super::base::ChainView,
    namespace: Uuid,
    complete: bool,
    retire: bool,
) -> Result<ChainPublication> {
    let mut depleted = ChainPublication::default();
    validate_chain_balances(tx, pool, view, namespace)?;
    apply_chain_resolutions(tx, pool, view, namespace)?;
    let rows: Vec<(String, String, String)> = if complete {
        tx.prepare("SELECT id,role,target FROM wallets WHERE pool_id=?1 ORDER BY sequence")?
            .query_map([pool], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        view.balances
            .keys()
            .map(|wallet| {
                tx.query_row(
                    "SELECT id,role,target FROM wallets WHERE pool_id=?1 AND id=?2",
                    params![pool, wallet],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .collect::<rusqlite::Result<_>>()?
    };
    publish_chain_balances(tx, pool, view, namespace, retire, &rows, &mut depleted)?;
    Ok(depleted)
}

fn validate_chain_balances(
    tx: &Connection,
    pool: &str,
    view: &super::base::ChainView,
    namespace: Uuid,
) -> Result<()> {
    #[cfg(not(test))]
    ensure!(
        view.observations.len() == view.balances.len()
            && view.balance_certificates.len() == view.balances.len()
            && view.resolution_certificates.len() == view.resolutions.len(),
        "missing verified evidence certificates"
    );
    if !view.observations.is_empty() {
        ensure!(
            view.observations.len() == view.balances.len()
                && view.balance_certificates.len() == view.observations.len(),
            "missing verified balance certificates"
        );
    }
    for (wallet, observation) in &view.observations {
        observation.validate()?;
        let certificate = view
            .balance_certificates
            .get(wallet)
            .context("missing verified balance certificate")?;
        ensure!(
            certificate.observation() == observation,
            "balance_certificate_binding_mismatch"
        );
        evidence::check_balance(tx, namespace, certificate)?;
        let address: String = tx.query_row(
            "SELECT address FROM wallets WHERE id=?1 AND pool_id=?2",
            params![wallet, pool],
            |row| row.get(0),
        )?;
        ensure!(
            observation.wallet == *wallet
                && observation.address.parse::<alloy_primitives::Address>()?
                    == address.parse::<alloy_primitives::Address>()?
                && view.balances.get(wallet) == Some(&observation.available()),
            super::base::evidence::EvidenceFailure::ScopeMismatch
        );
    }
    Ok(())
}

fn apply_chain_resolutions(
    tx: &Connection,
    pool: &str,
    view: &super::base::ChainView,
    namespace: Uuid,
) -> Result<()> {
    ensure!(
        view.released.len() == view.resolutions.len()
            && view
                .released
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == view.released.len()
            && view
                .released
                .iter()
                .all(|id| view.resolutions.contains_key(id)),
        "authorization release lacks complete canonical resolution evidence"
    );
    for id in &view.released {
        let resolution = &view.resolutions[id];
        let anchor = resolution.anchor.as_ref().unwrap_or(&view.anchor);
        ensure!(
            anchor.height <= view.anchor.height,
            "authorization evidence is newer than admission view"
        );
        let outcome = match resolution.outcome {
            super::base::AuthorizationOutcome::Used => "USED",
            super::base::AuthorizationOutcome::ExpiredUnused => "EXPIRED_UNUSED",
        };
        let support = view
            .resolution_certificates
            .get(id)
            .map(|certificate| {
                ensure!(
                    certificate.anchor() == anchor
                        && certificate.outcome() == &resolution.outcome
                        && certificate.block_time() == resolution.block_time,
                    "resolution_certificate_binding_mismatch"
                );
                evidence::check_resolution(tx, namespace, certificate)
            })
            .transpose()?;
        let existing: Option<(String,i64,String,u64)> = tx.query_row("SELECT outcome,height,hash,block_time FROM payment_resolutions WHERE attempt_id=?1",[id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,sql_u64(row,3)?))).optional()?;
        if let Some(existing) = existing {
            ensure!(
                existing
                    == (
                        outcome.into(),
                        i64::try_from(anchor.height)?,
                        anchor.hash.clone(),
                        resolution.block_time
                    ),
                "conflicting_authorization_resolution"
            );
        }
        let changed = tx.execute("UPDATE payment_attempts SET state='RESOLVED' WHERE id=?1 AND pool_id=?2 AND state='POSSIBLY_SUBMITTED'",params![id,pool])?;
        if changed == 1 {
            let (wallet, cost): (String, String) = tx.query_row(
                "SELECT wallet_id,amount FROM payment_attempts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            evidence::subtract_exposure(tx, &wallet, amount(&cost)?)?;
            tx.execute("INSERT INTO payment_resolutions(attempt_id,outcome,height,hash,block_time,support) VALUES(?1,?2,?3,?4,?5,?6)",
                params![id,outcome,i64::try_from(anchor.height)?,anchor.hash,i64::try_from(resolution.block_time)?,support])?;
            if support.is_some() {
                evidence::advance_checkpoint(tx, &wallet, anchor, outcome == "USED")?;
            }
        }
    }
    Ok(())
}

fn publish_chain_balances(
    tx: &Connection,
    pool: &str,
    view: &super::base::ChainView,
    namespace: Uuid,
    retire: bool,
    rows: &[(String, String, String)],
    depleted: &mut ChainPublication,
) -> Result<()> {
    for (id, role, _target) in rows {
        let Some(balance) = view.balances.get(id) else {
            ensure!(role == "RETIRED", "incomplete Base view");
            continue;
        };
        let previous: String =
            tx.query_row("SELECT balance FROM wallets WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        if *balance == U256::ZERO && amount(&previous)? > U256::ZERO {
            depleted.depleted.push(id.clone());
        }
        tx.execute(
            "UPDATE wallets SET balance=?1,block_hash=?2,block_height=?3 WHERE id=?4",
            params![
                balance.to_string(),
                view.observations
                    .get(id)
                    .map_or(&view.anchor.hash, |observation| &observation
                        .confirmed
                        .anchor
                        .hash),
                i64::try_from(
                    view.observations
                        .get(id)
                        .map_or(view.anchor.height, |observation| observation
                            .confirmed
                            .anchor
                            .height)
                )?,
                id
            ],
        )?;
        if let Some(certificate) = view.balance_certificates.get(id) {
            let support = evidence::check_balance(tx, namespace, certificate)?;
            if funding::record_readiness(tx, id, &support)? {
                funding::publish_destination_readiness(tx, id)?;
            }
            if retire
                && capacity::retire_with_support(tx, pool, id, &support, super::base::now()?)?
                && support.observation.remainder() > U256::ZERO
            {
                depleted.retired_remainders.push((
                    support.observation.confirmed.amount,
                    support.observation.latest.amount,
                ));
            }
        }
        #[cfg(test)]
        if view.balance_certificates.is_empty()
            && role == "ALLOCATED"
            && *balance >= amount(_target)?
        {
            // Legacy unsigned unit views exercise role mechanics only. Production
            // reaches readiness exclusively through the verified branch above.
            let address: String =
                tx.query_row("SELECT address FROM wallets WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
            let point = super::base::evidence::BalancePoint {
                anchor: super::base::Anchor {
                    height: view.anchor.height,
                    hash: format!("0x{:064x}", view.anchor.height),
                },
                amount: *balance,
            };
            let support = evidence::BalanceSupport {
                checkpoint: None,
                observation: super::base::evidence::BalanceObservation {
                    wallet: id.clone(),
                    address,
                    network: "eip155:8453".into(),
                    asset: crate::payment::USDC.into(),
                    endpoint: "a".repeat(64),
                    confirmed: point.clone(),
                    latest: super::base::evidence::BalancePoint {
                        anchor: super::base::Anchor {
                            height: view.anchor.height + 1,
                            hash: format!("0x{:064x}", view.anchor.height + 1),
                        },
                        amount: point.amount,
                    },
                    confirmations: 1,
                    valid_until: view.admission_valid_until,
                },
            };
            if funding::record_readiness(tx, id, &support)? {
                funding::publish_destination_readiness(tx, id)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/support/store_integrity.rs"]
mod integrity_tests;

#[cfg(all(test, unix))]
#[path = "../../tests/support/store_filesystem.rs"]
mod filesystem_tests;

#[cfg(test)]
#[path = "../../tests/support/funding_restriction.rs"]
mod funding_restriction_tests;
