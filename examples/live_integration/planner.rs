use super::manifest::*;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use x402_treazury::deployment::Deployment;

#[derive(Serialize)]
pub struct Plan {
    pub version: u32,
    pub run_id: String,
    pub offline: bool,
    pub execution_authorized: bool,
    pub api_reservation_atomic: u64,
    pub api_headroom_atomic: u64,
    pub planned_new_funding_jobs: u32,
    pub funding_job_headroom: u32,
    pub source_exposure_limit_zatoshis: u64,
    pub funding_feasibility: &'static str,
    pub prerequisites: Vec<&'static str>,
    pub case_bindings: BTreeMap<String, Value>,
    pub expanded_phases: Vec<Value>,
    pub declared_managed_pools: Vec<String>,
    pub network_disclosures: Value,
    pub manifest: Manifest,
}
fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64> {
    values.try_fold(0u64, |a, b| {
        a.checked_add(b).context("reservation sum overflow")
    })
}
pub async fn plan(m: &Manifest) -> Result<Plan> {
    m.validate()?;
    let config = Deployment::show_config(&m.deployment).await?;
    build(m, &config)
}
pub fn build(m: &Manifest, config: &Value) -> Result<Plan> {
    m.validate()?;
    if let Some(servers) = config.get("servers").and_then(Value::as_object) {
        ensure!(servers.values().all(|server| server.get("auth") != Some(&Value::Bool(false))),
            "live qualification requires authenticated listeners");
    }
    crate::pricing_stages::validate(m, config)?;
    let wallets = config["resolved_wallets"]
        .as_object()
        .context("resolved wallets missing")?;
    let declared: Vec<String> = wallets
        .iter()
        .filter(|(_, v)| v["mode"] == "zcash_rotation")
        .map(|(k, _)| k.clone())
        .collect();
    for pool in &m.start.pools {
        ensure!(declared.contains(pool), "unknown managed pool {pool}");
    }
    if !m.start.pools.is_empty() {
        ensure!(
            config["treasury"]["id"].as_str() == Some(&m.treasury_id),
            "treasury UUID does not match deployment"
        );
        ensure!(
            config["funding"]["confidentiality"] == "public",
            "initial runner supports public swaps only"
        );
    }
    let tor = config["network"]["mode"] == "tor";
    ensure!(
        tor == (m.network.tor_mode != TorMode::Direct),
        "manifest/deployment network modes differ"
    );
    let cases: BTreeMap<_, _> = m.cases.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut bindings = BTreeMap::new();
    for c in &m.cases {
        let wallet = config["wallet_bindings"][&c.server][&c.source]["wallet"]
            .as_str()
            .with_context(|| format!("case {}: unknown source/listener binding", c.id))?;
        let profile = &wallets[wallet];
        let reserve = usdc(&c.reserve_usdc)?;
        if !c.unsigned {
            ensure!(
                profile["mode"] == "zcash_rotation" && m.start.pools.iter().any(|x| x == wallet),
                "case {} must use a selected managed pool",
                c.id
            );
            let cap = usdc(
                profile["max_price_usd"]
                    .as_str()
                    .context("wallet cap missing")?,
            )?;
            ensure!(
                reserve >= cap,
                "case {} reservation is below effective wallet cap",
                c.id
            );
        }
        bindings.insert(c.id.clone(), json!({"server":c.server,"source":c.source,"tool":c.tool,"wallet":wallet,"unsigned":c.unsigned,"reservation_atomic":reserve,"catalog_validation":"pending_prepare"}));
    }
    let total = checked_sum(
        m.cases
            .iter()
            .map(|c| usdc(&c.reserve_usdc).expect("validated")),
    )?;
    let ceiling = usdc(&m.limits.api_reservation_usdc)?;
    ensure!(
        total <= ceiling,
        "expanded API reservations exceed run ceiling"
    );
    let mut seen = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut expanded = Vec::new();
    let mut jobs = if m.start.mode == StartMode::TreasuryOnly {
        u32::try_from(m.start.pools.len())?
            .checked_mul(2)
            .context("bootstrap slots overflow")?
    } else {
        0
    };
    for p in &m.phases {
        unique(p.depends_on.iter().map(String::as_str), "dependency")?;
        for dependency in &p.depends_on {
            ensure!(
                seen.contains(dependency),
                "phase {} dependency {dependency} must precede it (no cycles/forward dependencies)",
                p.id
            );
        }
        unique(p.pools.iter().map(String::as_str), "phase pool")?;
        ensure!(!p.cases.is_empty(), "phase {} has no cases", p.id);
        for pool in &p.pools {
            ensure!(
                m.start.pools.contains(pool),
                "phase {} references unselected pool {pool}",
                p.id
            );
        }
        let window = m
            .windows
            .iter()
            .find(|w| w.id == p.window)
            .context("phase has unknown window")?;
        let mut phase_unsigned = true;
        for id in &p.cases {
            let c = cases
                .get(id.as_str())
                .with_context(|| format!("unknown phase case {id}"))?;
            ensure!(
                used.insert(id.clone()),
                "case {id} appears more than once; expand distinct case IDs explicitly"
            );
            phase_unsigned &= c.unsigned;
            if !c.unsigned {
                ensure!(
                    p.pools
                        .iter()
                        .any(|v| Some(v.as_str()) == bindings[id]["wallet"].as_str()),
                    "case {id} wallet not in phase pools"
                );
            }
        }
        match &p.scenario {
            Scenario::Smoke {} => ensure!(
                p.cases.len() == 1 && p.pools.len() == 1 && !phase_unsigned,
                "smoke requires one paid case and pool"
            ),
            Scenario::Unsigned {} => ensure!(
                phase_unsigned && p.pools.is_empty(),
                "unsigned phase cannot contain paid cases or funding pools"
            ),
            Scenario::Concurrency { batches } => validate_batches(m, p, batches)?,
            Scenario::TorOutage {
                warm_case,
                cached_case,
                uncached_case,
            } => {
                ensure!(
                    m.start.pools.is_empty()
                        && declared.is_empty()
                        && config["funding"]["auto_fund"] != true,
                    "Tor outage requires a separate keyless deployment with no managed wallets or automatic funding"
                );
                ensure!(
                    m.network.tor_mode == TorMode::Owned && phase_unsigned && p.pools.is_empty(),
                    "Tor outage requires owned Tor and unsigned cases"
                );
                ensure!(
                    m.phases.last().is_some_and(|last| last.id == p.id),
                    "Tor outage must be the final phase"
                );
                ensure!(
                    cached_case != uncached_case
                        && p.cases == [cached_case.clone(), uncached_case.clone()],
                    "Tor outage requires ordered cached/uncached cases exactly once"
                );
                let warm = m
                    .phases
                    .iter()
                    .find(|phase| phase.cases.contains(warm_case))
                    .context("outage warm case missing")?;
                ensure!(
                    seen.contains(&warm.id) && p.depends_on.contains(&warm.id),
                    "outage requires an earlier warm phase as an explicit dependency"
                );
            }
            Scenario::Rotation {
                refill_slots,
                rounds,
            }
            | Scenario::RefillService {
                refill_slots,
                rounds,
            } => {
                ensure!(
                    !phase_unsigned && !p.pools.is_empty() && *refill_slots == 1,
                    "rotation/refill requires paid cases, pools and exactly one refill slot"
                );
                validate_rounds(m, p, rounds, 1, &bindings, config)?;
            }
            Scenario::Lifecycle {
                refill_slots,
                rounds,
                ..
            } => {
                ensure!(
                    !phase_unsigned && !p.pools.is_empty() && *refill_slots == 2,
                    "lifecycle requires paid cases, pools and exactly two refill slots"
                );
                validate_rounds(m, p, rounds, 2, &bindings, config)?;
                ensure!(
                    rounds[0].pool == rounds[1].pool,
                    "lifecycle must rotate the same selected pool twice"
                );
            }
            Scenario::Reliability { .. } => {}
            Scenario::ProviderSweep {} => {}
        }
        jobs = jobs
            .checked_add(p.scenario.refill_slots())
            .context("funding slots overflow")?;
        expanded.push(json!({"id":p.id,"scenario":p.scenario,"depends_on":p.depends_on,"window":window,"cases":p.cases,"pools":p.pools,"required":p.required,"call_count":p.cases.len()}));
        seen.insert(p.id.clone());
    }
    crate::reliability::validate(m)?;
    ensure!(
        used.len() == cases.len(),
        "unused cases: every reviewed case must appear in exactly one phase"
    );
    ensure!(
        jobs <= m.limits.new_funding_jobs,
        "insufficient funding slots for bootstrap/refills"
    );
    ensure!(
        jobs > 0 || m.limits.new_funding_jobs == 0,
        "funding allowance without a planned allocation is forbidden"
    );
    let only_unsigned = m.cases.iter().all(|c| c.unsigned);
    ensure!(
        !only_unsigned
            || (m.start.mode == StartMode::FundedPools && m.start.pools.is_empty() && jobs == 0),
        "unsigned suite must not initialize or select managed pools"
    );
    Ok(Plan {
        version: 1,
        run_id: m.run_id.clone(),
        offline: true,
        execution_authorized: false,
        api_reservation_atomic: total,
        api_headroom_atomic: ceiling - total,
        planned_new_funding_jobs: jobs,
        funding_job_headroom: m.limits.new_funding_jobs - jobs,
        source_exposure_limit_zatoshis: zec(&m.limits.source_exposure_zec)?,
        funding_feasibility: "unverified: no live balance, quote, fee or baseline reads",
        prerequisites: vec![
            "prepare must pin binaries/config/catalogs and validate selected tool schemas and arguments",
            "registry authorization and immutable treasury baseline required before execution",
            "ready active/standby balances and no unresolved liabilities required for paid calls",
            "source fees and floating swap minimum must fit funding permits; offline targets are not quotes",
            "all declared pools must be preserved; unused pools receive no new allocation authority",
        ],
        case_bindings: bindings,
        expanded_phases: expanded,
        declared_managed_pools: declared,
        network_disclosures: json!({"policy":config["network"],"base_rpc_policy":config["base_rpc_policy"],"tor":m.network,"external_requests_performed":0,"provider_origins":"resolve during prepare; no catalog downloads during plan"}),
        manifest: m.clone(),
    })
}
fn validate_rounds(
    m: &Manifest,
    phase: &Phase,
    rounds: &[RotationRound],
    count: usize,
    bindings: &BTreeMap<String, Value>,
    config: &Value,
) -> Result<()> {
    ensure!(
        rounds.len() == count,
        "scenario requires exactly {count} rotation rounds"
    );
    let mut ordered = Vec::new();
    for round in rounds {
        ensure!(
            phase.pools.contains(&round.pool),
            "rotation pool is not selected by phase"
        );
        ensure!(
            !round.depletion_cases.is_empty() && !round.service_cases.is_empty(),
            "rotation requires separate depletion and post-promotion service cases"
        );
        let price = usdc(&round.expected_price_usdc)?;
        ensure!(price > 0, "rotation expected price must be positive");
        let cap = usdc(
            config["resolved_wallets"][&round.pool]["max_price_usd"]
                .as_str()
                .context("rotation pool cap missing")?,
        )?;
        ensure!(
            price <= cap,
            "estimated depletion price exceeds effective wallet cap"
        );
        let mut first = None;
        for id in round.depletion_cases.iter().chain(&round.service_cases) {
            let case = m
                .cases
                .iter()
                .find(|c| &c.id == id)
                .context("unknown rotation case")?;
            ensure!(!case.unsigned, "rotation cases must be paid");
            ensure!(
                phase.cases.contains(id),
                "rotation case belongs to a different phase"
            );
            ordered.push(id.clone());
        }
        for id in &round.depletion_cases {
            let case = m.cases.iter().find(|c| &c.id == id).expect("checked case");
            ensure!(
                bindings[id]["wallet"] == round.pool,
                "depletion case must use its selected rotation pool"
            );
            ensure!(
                price <= usdc(&case.reserve_usdc)?,
                "estimated depletion price exceeds case reservation"
            );
            let request = (&case.server, &case.source, &case.tool, &case.arguments);
            ensure!(
                first.is_none_or(|previous| previous == request),
                "depletion estimate requires identical reviewed requests within each round"
            );
            first = Some(request);
        }
        ensure!(
            bindings[&round.service_cases[0]]["wallet"] == round.pool,
            "first post-promotion service case must exercise the rotated pool"
        );
    }
    ensure!(
        ordered == phase.cases,
        "rotation rounds must cover phase cases exactly once in depletion/service order"
    );
    Ok(())
}
fn validate_batches(m: &Manifest, p: &Phase, batches: &[Vec<String>]) -> Result<()> {
    ensure!(!batches.is_empty(), "concurrency requires explicit batches");
    let mut members = BTreeSet::new();
    for batch in batches {
        ensure!(
            (2..=m.limits.max_in_flight).contains(&batch.len()),
            "batch size exceeds concurrency or is below two"
        );
        for id in batch {
            ensure!(
                p.cases.contains(id) && members.insert(id),
                "batch contains unknown or repeated case {id}"
            );
        }
    }
    ensure!(
        members.len() == p.cases.len(),
        "batches must cover phase cases exactly once"
    );
    Ok(())
}
