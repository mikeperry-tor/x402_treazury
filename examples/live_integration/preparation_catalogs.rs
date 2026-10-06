//! Catalog preparation through the same pinned executable used for serving.
use super::{Manifest, Pins, Plan, collect, files};
use crate::{
    manifest::{CatalogMode, Confinement, TorMode},
    process::Process,
    schema,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use x402_treazury::{
    build_identity::{self, BuildIdentity},
    catalog::Config,
    deployment::MetaConfig,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPins {
    pub build: BuildIdentity,
    pub directory: PathBuf,
    pub files: BTreeMap<String, String>,
}
impl CatalogPins {
    pub fn verify(&self, m: &Manifest, pins: &Pins) -> Result<()> {
        execution_contract(
            m,
            &pins.resolved_config,
            self.files.contains_key("client.sb"),
        )?;
        self.verify_archive(m)?;
        ensure!(
            files::hash_file(&m.binary)? == pins.binary_sha256,
            "prepared executable changed"
        );
        verify_build(&self.build, pins)?;
        Ok(())
    }
    /// Validate immutable retained evidence without granting permission to run a
    /// changed executable. Post-run observers may use a newer implementation.
    pub fn verify_archive(&self, m: &Manifest) -> Result<Value> {
        files::directory(&self.directory)?;
        ensure!(
            self.directory == m.evidence_dir.join(&m.run_id),
            "catalog evidence directory mismatch"
        );
        for required in [
            "snapshot.json",
            "deployment.toml",
            "build.json",
            "frozen-snapshot.json",
        ] {
            ensure!(
                self.files.contains_key(required),
                "missing prepared artifact {required}"
            );
        }
        for (name, hash) in &self.files {
            ensure!(
                !name.contains(['/', '\\']) && name != "." && name != "..",
                "invalid artifact name"
            );
            let path = self.directory.join(name);
            files::regular(&path)?;
            ensure!(
                files::hash_file(&path)? == *hash,
                "prepared artifact changed: {name}"
            );
        }
        let snapshot: Value =
            serde_json::from_slice(&files::read_catalog(&self.directory.join("snapshot.json"))?)?;
        validate_cases(m, &snapshot)?;
        let frozen: Value = serde_json::from_slice(&files::read_catalog(
            &self.directory.join("frozen-snapshot.json"),
        )?)?;
        validate_cases(m, &frozen)?;
        ensure!(
            snapshot["inventory"] == frozen["inventory"],
            "frozen catalog inventory changed"
        );
        Ok(snapshot)
    }
}
fn execution_contract(m: &Manifest, config: &Value, confined: bool) -> Result<()> {
    ensure!(
        matches!(m.catalog.execution, CatalogMode::Frozen),
        "catalog runner requires frozen execution"
    );
    ensure!(
        (confined
            && m.network.tor_mode == TorMode::Owned
            && m.network.confinement == Confinement::MacosSandbox)
            || (!confined
                && m.network.tor_mode != TorMode::Owned
                && m.network.confinement == Confinement::None
                && !m.network.require_isolation_evidence),
        "owned Tor qualification requires the supervised confined command; cannot downgrade to proxy-only"
    );
    let managed = !m.start.pools.is_empty();
    ensure!(
        config["resolved_wallets"]
            .as_object()
            .context("missing wallets")?
            .values()
            .all(|w| w["mode"] == if managed { "zcash_rotation" } else { "static" }),
        "qualification cannot mix static and managed profiles"
    );
    ensure!(
        managed || (m.cases.iter().all(|c| c.unsigned) && config["funding"]["auto_fund"] != true),
        "unsigned preparation refuses payments/automatic funding"
    );
    ensure!(
        config["source_management"].is_null(),
        "qualification refuses source management"
    );
    if managed {
        ensure!(
            m.limits.new_funding_jobs == 0 || config["funding"]["auto_fund"] == true,
            "positive funding plan requires auto_fund=true"
        );
        ensure!(
            config["treasury"]["id"] == m.treasury_id,
            "managed qualification treasury differs"
        );
        ensure!(
            config["funding"]["confidentiality"] == "public",
            "managed qualification requires public swaps"
        );
    }
    Ok(())
}
fn verify_build(build: &BuildIdentity, pins: &Pins) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = build_identity::current();
    ensure!(
        build.version == 1 && build.qualification_protocol == 1,
        "unsupported executable qualification contract"
    );
    ensure!(
        build.source_sha256 == build_identity::inputs::source_hash(root)?
            && build.source_sha256 == runner.source_sha256,
        "executable or runner is stale against current compilation inputs; rebuild both"
    );
    ensure!(
        build.cargo_lock_sha256 == pins.cargo_lock_sha256
            && build.cargo_lock_sha256 == runner.cargo_lock_sha256,
        "executable/runner Cargo.lock mismatch"
    );
    ensure!(
        build.features == runner.features
            && build.target == runner.target
            && build.compiler == runner.compiler,
        "executable and runner features/target/compiler differ; rebuild together"
    );
    Ok(())
}
fn validate_cases(m: &Manifest, snapshot: &Value) -> Result<()> {
    if snapshot.get("catalog_stages").is_some() {
        let sources = snapshot["sources"].as_object().context("missing catalog sources")?.keys().cloned().collect();
        crate::catalog_stages::project(snapshot, &sources, "inspection")?;
    }
    crate::tor::outage::validate(m, snapshot)?;
    ensure!(
        snapshot["version"] == 1,
        "unsupported catalog snapshot contract"
    );
    let inventory = snapshot["inventory"]
        .as_array()
        .context("missing inventory")?;
    for case in &m.cases {
        let listener = inventory
            .iter()
            .find(|s| s["server"] == case.server)
            .context("case listener missing from catalog")?;
        let tool = listener["tools"]
            .as_array()
            .context("missing tools")?
            .iter()
            .find(|t| t["source"] == case.source && t["name"] == case.tool)
            .with_context(|| {
                format!(
                    "case {} is not in its selected source/listener inventory",
                    case.id
                )
            })?;
        schema::validate(
            &tool["input_schema"],
            &Value::Object(case.arguments.clone()),
        )
        .with_context(|| format!("case {} arguments cannot be certified", case.id))?;
        if case.help_cache.is_some() {
            ensure!(
                case.unsigned && tool["help_url"].as_str().is_some(),
                "help_cache requires an unsigned help-tool case"
            );
        }
    }
    Ok(())
}
async fn inspect(
    m: &Manifest,
    directory: &Path,
    label: &str,
    args: Vec<String>,
    deadline: Instant,
    profile: Option<&Path>,
    stop: CancellationToken,
) -> Result<Vec<u8>> {
    ensure!(
        !stop.is_cancelled(),
        "catalog preparation cancelled before {label}"
    );
    eprintln!("qualification inspecting {label} through the pinned executable");
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .context("catalog preparation deadline exceeded")?;
    let child = Process::launch_confined(
        &m.binary,
        &args,
        &BTreeMap::new(),
        directory,
        files::CATALOG_BYTES,
        profile,
    )?;
    let evidence = child
        .wait(
            remaining,
            Duration::from_secs(m.limits.cleanup_seconds),
            stop.clone(),
        )
        .await?;
    files::publish_catalog(
        &directory.join(format!("{label}.stdout")),
        &evidence.stdout.bytes,
    )?;
    files::publish_catalog(
        &directory.join(format!("{label}.stderr")),
        &evidence.stderr.bytes,
    )?;
    let record = serde_json::json!({"pid":evidence.pid,"exit_code":evidence.exit_code,"success":evidence.success,
        "forced_kill":evidence.forced_kill,"reason":evidence.reason,"valid_output":evidence.valid_output(),
        "stdout_bytes":evidence.stdout.bytes.len(),"stderr_bytes":evidence.stderr.bytes.len(),
        "stdout_limit_exceeded":evidence.stdout.limit_exceeded,"stderr_limit_exceeded":evidence.stderr.limit_exceeded,
        "stdout_read_error":evidence.stdout.read_error,"stderr_read_error":evidence.stderr.read_error,
        "stdout_pipe_incomplete":evidence.stdout.pipe_incomplete,"stderr_pipe_incomplete":evidence.stderr.pipe_incomplete});
    files::publish(
        &directory.join(format!("{label}.process.json")),
        &serde_json::to_vec_pretty(&record)?,
    )?;
    ensure!(
        evidence.success
            && evidence.valid_output()
            && !evidence.forced_kill
            && evidence.reason == "exited",
        "{label} inspection failed; private process evidence retained"
    );
    Ok(evidence.stdout.bytes)
}
pub async fn collect_catalogs(m: &Manifest, state: &Path) -> Result<(Plan, Pins)> {
    collect_catalogs_confined(m, state, None, CancellationToken::new()).await
}
pub async fn collect_catalogs_confined(
    m: &Manifest,
    state: &Path,
    profile: Option<&Path>,
    stop: CancellationToken,
) -> Result<(Plan, Pins)> {
    eprintln!("qualification preparing binary/configuration provenance and frozen catalogs");
    let (plan, mut pins) = collect(m, state).await?;
    execution_contract(m, &pins.resolved_config, profile.is_some())?;
    files::directory(&m.evidence_dir)?;
    let directory = m.evidence_dir.join(&m.run_id);
    files::create_dir(&directory)?; // Failed preparation evidence is never overwritten.
    let deadline = Instant::now() + Duration::from_secs(m.limits.run_seconds);
    let build: BuildIdentity = serde_json::from_slice(
        &inspect(
            m,
            &directory,
            "build",
            vec!["build-info".into()],
            deadline,
            profile,
            stop.clone(),
        )
        .await?,
    )?;
    verify_build(&build, &pins)?;
    let bytes = inspect(
        m,
        &directory,
        "catalog",
        vec![
            "--config".into(),
            m.deployment.to_string_lossy().into_owned(),
            "--qualification-snapshot".into(),
        ],
        deadline,
        profile,
        stop.clone(),
    )
    .await?;
    let snapshot: Value = serde_json::from_slice(&bytes)?;
    validate_cases(m, &snapshot)?;
    let mut config: MetaConfig = serde_json::from_value(snapshot["deployment"].clone())?;
    ensure!(
        config.source_management.is_none()
            && config
                .servers
                .values()
                .all(|s| s.source_management.is_none()),
        "qualification refuses source management"
    );
    for listener in config.servers.values() {
        ensure!(
            listener.listen.ip().is_loopback() && listener.listen.port() != 0,
            "prepared listeners require fixed literal loopback addresses"
        );
    }
    let mut artifacts = BTreeMap::new();
    let mut save = |name: String, bytes: &[u8]| -> Result<()> {
        files::publish_catalog(&directory.join(&name), bytes)?;
        artifacts.insert(name, files::hash(bytes));
        Ok(())
    };
    if let Some(profile) = profile {
        files::regular(profile)?;
        save("client.sb".into(), &files::read(profile)?)?;
    }
    save("build.json".into(), &serde_json::to_vec(&build)?)?;
    save("snapshot.json".into(), &bytes)?;
    for (name, source) in &mut config.sources {
        let recorded = &snapshot["sources"][name];
        let mut settings: Config = serde_json::from_value(recorded["settings"].clone())?;
        let filename = format!("source-{name}.json");
        save(
            filename.clone(),
            &serde_json::to_vec(&recorded["document"])?,
        )?;
        settings.spec = directory.join(filename).to_string_lossy().into_owned();
        settings.base_url = Some(
            recorded["base_url"]
                .as_str()
                .context("missing resolved base URL")?
                .into(),
        );
        // Inspection does not probe. Preserve serving policy; recorded startup
        // price lines reconstruct expected descriptions before inventory checks.
        source.provider = toml::Value::try_from(settings)?
            .as_table()
            .context("source settings must be table")?
            .clone();
    }
    save(
        "deployment.toml".into(),
        toml::to_string(&config)?.as_bytes(),
    )?;
    let frozen = inspect(
        m,
        &directory,
        "frozen-catalog",
        vec![
            "--config".into(),
            directory
                .join("deployment.toml")
                .to_string_lossy()
                .into_owned(),
            "--qualification-snapshot".into(),
        ],
        deadline,
        profile,
        stop.clone(),
    )
    .await?;
    let frozen_value: Value = serde_json::from_slice(&frozen)?;
    ensure!(
        snapshot["inventory"] == frozen_value["inventory"],
        "frozen catalog changed tool inventory"
    );
    save("frozen-snapshot.json".into(), &frozen)?;
    ensure!(
        x402_treazury::deployment::Deployment::show_config(&m.deployment).await?
            == pins.resolved_config,
        "deployment configuration changed during catalog preparation"
    );
    pins.catalogs = Some(CatalogPins {
        build,
        directory,
        files: artifacts,
    });
    pins.qualification = "catalogs_prepared".into();
    pins.catalogs.as_ref().expect("just set").verify(m, &pins)?;
    Ok((plan, pins))
}

#[cfg(test)]
mod help_contract_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cache_assertions_require_a_reviewed_unsigned_help_tool() {
        let mut m = crate::tests::manifest();
        m.cases[0].help_cache = Some(crate::help::ExpectedCache::Hit);
        let case = &m.cases[0];
        let mut snapshot = json!({"version":1,"inventory":[{"server":case.server,"tools":[{"source":case.source,"name":case.tool,"input_schema":{"type":"object"}}]}]});
        assert!(validate_cases(&m, &snapshot).is_err());
        snapshot["inventory"][0]["tools"][0]["help_url"] =
            json!("https://example.invalid/llms.txt");
        assert!(validate_cases(&m, &snapshot).is_err());
        m.cases[0].unsigned = true;
        validate_cases(&m, &snapshot).unwrap();
    }
}
