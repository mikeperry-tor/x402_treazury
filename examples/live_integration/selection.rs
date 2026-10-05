//! Offline filtering of reviewed cases. Never invent arguments, prices or authority.
use crate::{
    files,
    manifest::{Manifest, Scenario, identifier},
    planner,
};
use anyhow::{Result, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Args, Clone)]
pub struct Options {
    #[arg(long)]
    pub manifest: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long)]
    pub run_id: String,
    /// Include cases from these exact source IDs (union with --case).
    #[arg(long)]
    pub source: Vec<String>,
    #[arg(long = "case")]
    pub cases: Vec<String>,
    #[arg(long)]
    pub exclude_source: Vec<String>,
    #[arg(long = "exclude-case")]
    pub exclude_cases: Vec<String>,
    #[arg(long)]
    pub exclude_reliability_tag: Vec<String>,
    /// Omit cases whose source has any configured reliability tag.
    #[arg(long)]
    pub exclude_flagged: bool,
}

pub async fn write(options: Options) -> Result<Value> {
    let input = Manifest::load(&options.manifest).await?;
    let config = x402_treazury::deployment::Deployment::show_config(&input.deployment).await?;
    planner::build(&input, &config)?;
    let input_hash = files::hash(toml::to_string(&input)?);
    let (selected, omitted) = select(&input, &config, &options)?;
    let plan = planner::build(&selected, &config)?;
    let bytes = toml::to_string_pretty(&selected)?;
    if bytes.len() > 4 * 1024 * 1024 {
        eprintln!("selected manifest exceeds 4194304 byte limit; no output published");
        anyhow::bail!("selected manifest exceeds 4194304 byte limit; split the reviewed input");
    }
    files::publish(&options.output, bytes.as_bytes())?;
    Ok(
        json!({"version":1,"execution_authorized":false,"output":options.output,
        "reviewed_input_sha256":input_hash,"manifest_sha256":files::hash(&bytes),
        "selected_cases":selected.cases.iter().map(|c|&c.id).collect::<Vec<_>>(),"omitted_cases":omitted,
        "omitted_phases":input.phases.iter().filter(|p|!selected.phases.iter().any(|s|s.id==p.id)).map(|p|&p.id).collect::<Vec<_>>(),
        "api_reservation_atomic":plan.api_reservation_atomic,"planned_new_funding_jobs":plan.planned_new_funding_jobs,
        "unchanged_deployment":true,"note":"Only cases were selected. Declared sources still load and declared managed pools remain configured; review plan before preparing or authorizing this new run."}),
    )
}

fn select(
    input: &Manifest,
    config: &Value,
    options: &Options,
) -> Result<(Manifest, BTreeMap<String, Vec<&'static str>>)> {
    ensure!(
        identifier(&options.run_id) && options.run_id != input.run_id,
        "selection requires a different valid run ID"
    );
    ensure!(
        input.phases.iter().all(|p| matches!(
            p.scenario,
            Scenario::ProviderSweep {} | Scenario::Unsigned {} | Scenario::Reliability { .. }
        )),
        "case selection supports only provider_sweep, unsigned and reliability phases; lifecycle/concurrency manifests must be authored explicitly"
    );
    ensure!(
        !options.source.is_empty()
            || !options.cases.is_empty()
            || !options.exclude_source.is_empty()
            || !options.exclude_cases.is_empty()
            || !options.exclude_reliability_tag.is_empty()
            || options.exclude_flagged,
        "at least one case/source/reliability selector is required"
    );
    let sources: BTreeSet<_> = input.cases.iter().map(|c| c.source.as_str()).collect();
    let cases: BTreeSet<_> = input.cases.iter().map(|c| c.id.as_str()).collect();
    for source in options.source.iter().chain(&options.exclude_source) {
        ensure!(
            sources.contains(source.as_str()),
            "unknown source selector {source}"
        );
    }
    for case in options.cases.iter().chain(&options.exclude_cases) {
        ensure!(
            cases.contains(case.as_str()),
            "unknown case selector {case}"
        );
    }
    let mut tags: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for source in sources {
        let value = &config["sources"][source]["settings"]["reliability_tags"];
        tags.insert(
            source,
            if value.is_null() {
                Vec::new()
            } else {
                serde_json::from_value(value.clone())?
            },
        );
    }
    for tag in &options.exclude_reliability_tag {
        ensure!(
            tags.values().any(|v| v.contains(tag)),
            "unknown reliability tag selector {tag}"
        );
    }
    let mut selected = input.clone();
    selected.run_id = options.run_id.clone();
    let mut omitted = BTreeMap::new();
    selected.cases.retain(|case| {
        let mut reasons = Vec::new();
        if !(options.source.is_empty() && options.cases.is_empty()
            || options.source.contains(&case.source)
            || options.cases.contains(&case.id))
        {
            reasons.push("not_in_include_set");
        }
        if options.exclude_source.contains(&case.source) {
            reasons.push("excluded_source");
        }
        if options.exclude_cases.contains(&case.id) {
            reasons.push("excluded_case");
        }
        let source_tags = &tags[case.source.as_str()];
        if options.exclude_flagged && !source_tags.is_empty() {
            reasons.push("flagged_source");
        }
        if source_tags
            .iter()
            .any(|t| options.exclude_reliability_tag.contains(t))
        {
            reasons.push("excluded_reliability_tag");
        }
        if reasons.is_empty() {
            true
        } else {
            omitted.insert(case.id.clone(), reasons);
            false
        }
    });
    ensure!(
        !selected.cases.is_empty(),
        "selectors removed every case; no manifest published"
    );
    let kept: BTreeSet<_> = selected.cases.iter().map(|c| c.id.clone()).collect();
    for phase in &mut selected.phases {
        phase.cases.retain(|c| kept.contains(c));
    }
    selected.phases.retain(|p| !p.cases.is_empty());
    let phases: BTreeSet<_> = selected.phases.iter().map(|p| p.id.as_str()).collect();
    ensure!(
        selected
            .phases
            .iter()
            .all(|p| p.depends_on.iter().all(|d| phases.contains(d.as_str()))),
        "selection removes a required dependency; author a new reviewed phase graph explicitly"
    );
    selected.validate()?;
    crate::reliability::validate(&selected)?;
    Ok((selected, omitted))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Value, Options) {
        let mut m = crate::tests::manifest();
        m.phases[0].scenario = Scenario::ProviderSweep {};
        m.cases[0].checks.push(crate::semantics::Check::JsonExists {
            pointer: "/data".into(),
        });
        let mut second = m.cases[0].clone();
        second.id = "second".into();
        second.source = "other".into();
        m.cases.push(second);
        m.phases[0].cases.push("second".into());
        let config = json!({"sources":{"api":{"settings":{"reliability_tags":[]}},"other":{"settings":{"reliability_tags":["slow_pricing"]}}}});
        let options = Options {
            manifest: "input.toml".into(),
            output: "output.toml".into(),
            run_id: "selected_run".into(),
            source: vec![],
            cases: vec![],
            exclude_source: vec![],
            exclude_cases: vec![],
            exclude_reliability_tag: vec![],
            exclude_flagged: true,
        };
        (m, config, options)
    }
    #[test]
    fn selection_preserves_reviewed_requests_windows_authority_and_pool_scope() {
        let (m, c, o) = fixture();
        let (selected, omitted) = select(&m, &c, &o).unwrap();
        assert_eq!(selected.cases.len(), 1);
        assert_eq!(
            serde_json::to_value(&selected.cases[0]).unwrap(),
            serde_json::to_value(&m.cases[0]).unwrap()
        );
        assert_eq!(omitted["second"], ["flagged_source"]);
        let mut expected = serde_json::to_value(&m).unwrap();
        expected["run_id"] = json!("selected_run");
        expected["cases"] = json!([m.cases[0]]);
        expected["phases"][0]["cases"] = json!(["call"]);
        assert_eq!(serde_json::to_value(&selected).unwrap(), expected);
    }
    #[test]
    fn includes_union_and_exclusions_win_without_guessing_source_from_tool_name() {
        let (m, c, mut o) = fixture();
        o.source = vec!["other".into()];
        o.cases = vec!["call".into()];
        o.exclude_flagged = false;
        o.exclude_cases = vec!["second".into()];
        let (selected, omitted) = select(&m, &c, &o).unwrap();
        assert_eq!(selected.cases[0].id, "call");
        assert_eq!(omitted["second"], ["excluded_case"]);
        o.exclude_cases.clear();
        o.exclude_reliability_tag = vec!["slow_pricing".into()];
        assert_eq!(
            select(&m, &c, &o).unwrap().1["second"],
            ["excluded_reliability_tag"]
        );
    }
    #[test]
    fn typos_empty_selection_lifecycle_and_broken_dependencies_are_rejected() {
        let (m, c, o) = fixture();
        for mode in 0..6 {
            let (mut m, mut o) = (
                m.clone(),
                Options {
                    manifest: o.manifest.clone(),
                    output: o.output.clone(),
                    run_id: o.run_id.clone(),
                    source: vec![],
                    cases: vec![],
                    exclude_source: vec![],
                    exclude_cases: vec![],
                    exclude_reliability_tag: vec![],
                    exclude_flagged: true,
                },
            );
            match mode {
                0 => o.source.push("typo".into()),
                1 => o.cases.push("typo".into()),
                2 => o.exclude_reliability_tag.push("typo".into()),
                3 => o.exclude_source.push("api".into()),
                4 => m.phases[0].scenario = Scenario::Smoke {},
                _ => {
                    let mut prerequisite = m.phases[0].clone();
                    prerequisite.id = "prerequisite".into();
                    prerequisite.cases = vec!["second".into()];
                    m.phases[0].cases = vec!["call".into()];
                    m.phases[0].depends_on = vec!["prerequisite".into()];
                    m.phases.insert(0, prerequisite);
                }
            }
            assert!(select(&m, &c, &o).is_err(), "mode {mode}");
        }
    }
    #[test]
    fn reliability_selection_cannot_silently_discard_required_repetitions() {
        let (mut m, c, mut o) = fixture();
        m.cases[1].source = m.cases[0].source.clone();
        m.windows[0].not_after = 10;
        m.windows.push(crate::manifest::Window {
            id: "later".into(),
            not_before: 11,
            not_after: 20,
        });
        m.phases[0].cases = vec!["call".into()];
        m.phases[0].scenario = Scenario::Reliability {
            min_spacing_seconds: 1,
            later_utc_day: false,
        };
        let mut later = m.phases[0].clone();
        later.id = "later".into();
        later.window = "later".into();
        later.cases = vec!["second".into()];
        m.phases.push(later);
        o.exclude_flagged = false;
        o.source = vec!["api".into()];
        assert_eq!(select(&m, &c, &o).unwrap().0.cases.len(), 2);
        o.exclude_cases = vec!["second".into()];
        assert!(
            select(&m, &c, &o)
                .unwrap_err()
                .to_string()
                .contains("at least two identical")
        );
    }
    #[tokio::test]
    async fn writes_new_private_manifest_without_catalog_wallet_or_registry_access() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private");
        files::create_dir(&private).unwrap();
        let (mut m, _, mut o) = fixture();
        m.start.pools.clear();
        m.phases[0].pools.clear();
        m.phases[0].scenario = Scenario::Unsigned {};
        for case in &mut m.cases {
            case.unsigned = true;
            case.reserve_usdc = "0".into();
        }
        std::fs::write(
            private.join("deploy.toml"),
            r#"version=1
[wallets.static_wallet]
mode="static"
private_key_env="NEVER_READ_ME"
[sources.api]
spec="https://must-not-fetch.invalid/openapi.json"
[sources.other]
spec="https://must-not-fetch.invalid/other.json"
reliability_tags=["slow_pricing"]
[servers.main]
listen="127.0.0.1:0"
sources=["api","other"]
wallet="static_wallet"
bearer_token_env="NEVER_READ_ME_EITHER"
"#,
        )
        .unwrap();
        o.manifest = private.join("input.toml");
        o.output = private.join("selected.toml");
        std::fs::write(&o.manifest, toml::to_string(&m).unwrap()).unwrap();
        let output = o.output.clone();
        let report = write(o).await.unwrap();
        assert_eq!(report["execution_authorized"], false);
        assert_eq!(report["omitted_cases"]["second"], json!(["flagged_source"]));
        files::regular(&output).unwrap();
        let loaded = Manifest::load(&output).await.unwrap();
        assert!(loaded.deployment.is_absolute());
        assert_eq!(loaded.run_id, "selected_run");
        assert_eq!(loaded.cases.len(), 1);
        assert!(!private.join("evidence").exists());
        let mut o = fixture().2;
        o.manifest = private.join("input.toml");
        o.output = output.clone();
        let saved = files::read(&output).unwrap();
        assert!(
            write(o)
                .await
                .unwrap_err()
                .to_string()
                .contains("overwrite")
        );
        assert_eq!(files::read(&output).unwrap(), saved);
    }
}
