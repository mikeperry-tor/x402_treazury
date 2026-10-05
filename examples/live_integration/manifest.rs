use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub run_id: String,
    pub treasury_id: String,
    pub deployment: PathBuf,
    pub binary: PathBuf,
    pub evidence_dir: PathBuf,
    pub registry_authorization: String,
    pub start: Start,
    pub network: Network,
    pub limits: Limits,
    pub catalog: Catalog,
    pub windows: Vec<Window>,
    pub cases: Vec<Case>,
    pub phases: Vec<Phase>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pricing_checks: BTreeMap<String, crate::pricing_stages::Expectation>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub mode: StartMode,
    pub pools: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartMode {
    TreasuryOnly,
    FundedPools,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub tor_mode: TorMode,
    pub tor_binary: Option<PathBuf>,
    pub confinement: Confinement,
    pub require_isolation_evidence: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorMode {
    Owned,
    External,
    Direct,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confinement {
    MacosSandbox,
    None,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub api_reservation_usdc: String,
    pub new_funding_jobs: u32,
    pub source_exposure_zec: String,
    pub max_in_flight: usize,
    pub run_seconds: u64,
    pub phase_seconds: u64,
    pub call_seconds: u64,
    pub cleanup_seconds: u64,
    pub result_bytes: usize,
    /// Explicit experiment settling interval after connected batches, including the last.
    #[serde(default)]
    pub post_batch_wait_ms: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub execution: CatalogMode,
    pub record_live_discovery: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogMode {
    Frozen,
    Live,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub id: String,
    pub not_before: u64,
    pub not_after: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub server: String,
    pub source: String,
    pub tool: String,
    pub arguments: Map<String, Value>,
    pub reserve_usdc: String,
    pub reviewed_read_only: bool,
    #[serde(default)]
    pub unsigned: bool,
    /// Reviewed provider-body assertions; omission never certifies provider semantics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<crate::semantics::Check>,
    /// Successful help response must use this cache path; never grants a new request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_cache: Option<crate::help::ExpectedCache>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub id: String,
    pub window: String,
    pub cases: Vec<String>,
    pub pools: Vec<String>,
    pub required: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub scenario: Scenario,
}
// Nested tagged enum keeps scenario-specific unknown fields strictly rejected.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scenario {
    Smoke {},
    Unsigned {},
    TorOutage {
        warm_case: String,
        cached_case: String,
        uncached_case: String,
    },
    ProviderSweep {},
    Concurrency {
        batches: Vec<Vec<String>>,
    },
    Rotation {
        refill_slots: u32,
        rounds: Vec<RotationRound>,
    },
    RefillService {
        refill_slots: u32,
        rounds: Vec<RotationRound>,
    },
    Lifecycle {
        refill_slots: u32,
        restart: Restart,
        rounds: Vec<RotationRound>,
    },
    Reliability {
        min_spacing_seconds: u64,
        later_utc_day: bool,
    },
}
/// Each round stops depletion at one observed promotion, then uses separate calls.
/// Price is a reviewed estimate only; production challenges and caps remain authoritative.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RotationRound {
    pub pool: String,
    pub expected_price_usdc: String,
    pub depletion_cases: Vec<String>,
    pub service_cases: Vec<String>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    QueuedRefill,
    SubmittedDeposit,
}
impl Scenario {
    pub fn refill_slots(&self) -> u32 {
        match self {
            Self::Rotation { refill_slots, .. }
            | Self::RefillService { refill_slots, .. }
            | Self::Lifecycle { refill_slots, .. } => *refill_slots,
            _ => 0,
        }
    }
}
pub fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn unique<'a>(values: impl IntoIterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        ensure!(
            identifier(value) && seen.insert(value),
            "invalid/duplicate {label}: {value}"
        );
    }
    Ok(())
}
pub fn usdc(s: &str) -> Result<u64> {
    let n = x402_treazury::payment::SpendPolicy::dollars(s)?
        .max_atomic
        .context("finite USDC amount required")?;
    let n = u64::try_from(n)?;
    ensure!(n <= i64::MAX as u64, "USDC amount exceeds registry range");
    Ok(n)
}
pub fn zec(s: &str) -> Result<u64> {
    // Preserve the production decimal grammar while allowing zero as deny-all.
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    if !whole.is_empty()
        && whole.bytes().all(|b| b == b'0')
        && fraction.len() <= 8
        && fraction.bytes().all(|b| b == b'0')
    {
        return Ok(0);
    }
    Ok(x402_treazury::rotation::config::zatoshis(s)?.try_into()?)
}
impl Manifest {
    pub async fn load(path: &Path) -> Result<Self> {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        tokio::fs::File::open(path)
            .await?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "manifest exceeds 4194304 byte limit"
        );
        let mut m: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        let file = std::fs::canonicalize(path)?;
        let parent = file.parent().context("manifest parent missing")?;
        for p in [&mut m.deployment, &mut m.binary, &mut m.evidence_dir] {
            if p.is_relative() {
                *p = parent.join(&*p);
            }
        }
        if let Some(p) = &mut m.network.tor_binary
            && p.is_relative()
        {
            *p = parent.join(&*p);
        }
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && identifier(&self.run_id)
                && identifier(&self.registry_authorization),
            "unsupported version or invalid run/authorization identifier"
        );
        ensure!(
            !uuid::Uuid::parse_str(&self.treasury_id)?.is_nil(),
            "treasury UUID must not be nil"
        );
        unique(self.start.pools.iter().map(String::as_str), "starting pool")?;
        ensure!(
            !self.cases.is_empty() && self.cases.len() <= 10000,
            "case count must be 1..10000"
        );
        ensure!(
            !self.phases.is_empty() && self.phases.len() <= 1000,
            "phase count must be 1..1000"
        );
        ensure!(
            !self.windows.is_empty() && self.windows.len() <= 1000,
            "window count must be 1..1000"
        );
        unique(self.cases.iter().map(|x| x.id.as_str()), "case")?;
        unique(self.phases.iter().map(|x| x.id.as_str()), "phase")?;
        unique(self.windows.iter().map(|x| x.id.as_str()), "window")?;
        for w in &self.windows {
            ensure!(
                w.not_before < w.not_after && w.not_after <= i64::MAX as u64,
                "window {} requires ordered nonzero end timestamps",
                w.id
            );
        }
        for c in &self.cases {
            ensure!(
                c.reviewed_read_only,
                "case {} requires explicit read-only review",
                c.id
            );
            unique([c.server.as_str()], "server")?;
            unique([c.source.as_str()], "source")?;
            ensure!(
                !c.tool.is_empty() && c.tool.len() <= 256,
                "invalid tool name"
            );
            let reserve = usdc(&c.reserve_usdc)?;
            crate::semantics::validate(&c.checks)?;
            ensure!(
                c.help_cache.is_none() || c.unsigned,
                "help_cache assertions require an unsigned case"
            );
            ensure!(
                (c.unsigned && reserve == 0) || (!c.unsigned && reserve > 0),
                "case {} unsigned/reservation mismatch",
                c.id
            );
        }
        let l = &self.limits;
        usdc(&l.api_reservation_usdc)?;
        zec(&l.source_exposure_zec)?;
        ensure!(
            (1..=64).contains(&l.max_in_flight) && l.new_funding_jobs <= 1000,
            "invalid concurrency/funding-job limits"
        );
        ensure!(
            (1..=604800).contains(&l.run_seconds),
            "run deadline must be 1..604800 seconds (seven days); no automatic extension"
        );
        for n in [l.phase_seconds, l.call_seconds, l.cleanup_seconds] {
            ensure!(
                (1..=86400).contains(&n),
                "call/phase/cleanup deadline must be 1..86400 seconds"
            );
        }
        ensure!(
            l.call_seconds <= l.phase_seconds && l.phase_seconds <= l.run_seconds,
            "call/phase/run deadlines must be ordered"
        );
        ensure!(
            l.post_batch_wait_ms <= 60_000 && l.post_batch_wait_ms <= l.phase_seconds * 1000,
            "post_batch_wait_ms must be 0..60000 and fit within the phase budget"
        );
        ensure!(
            (1..=64 * 1024 * 1024).contains(&l.result_bytes),
            "result_bytes must be 1..67108864"
        );
        ensure!(
            (l.new_funding_jobs == 0) == (zec(&l.source_exposure_zec)? == 0),
            "funding jobs and source exposure must both be zero or positive"
        );
        match self.network.tor_mode {
            TorMode::Owned => ensure!(
                self.network.tor_binary.is_some(),
                "owned Tor requires executable reference"
            ),
            TorMode::Direct => ensure!(
                !self.network.require_isolation_evidence
                    && self.network.confinement == Confinement::None
                    && self.network.tor_binary.is_none(),
                "direct mode cannot claim Tor qualification"
            ),
            TorMode::External => ensure!(
                self.network.tor_binary.is_none(),
                "external Tor must not specify an owned executable"
            ),
        }
        Ok(())
    }
}
