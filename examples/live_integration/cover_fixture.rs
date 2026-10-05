//! Offline materialization of reviewed, zero-spend cover experiments.
use crate::{files, manifest, registry};
use anyhow::{Result, ensure};
use clap::{Args, ValueEnum};
use serde_json::{Value, json};
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Profile {
    None,
    Ranges,
    Padding,
    Combined,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Distribution {
    Uniform,
    Exponential,
    Weibull,
    LogNormal,
    WeightedDiscrete,
}
#[derive(Args)]
pub struct Options {
    /// New private directory. This creates synthetic state that must never be funded.
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub binary: PathBuf,
    #[arg(long)]
    pub tor_binary: PathBuf,
    #[arg(long, value_enum, default_value = "ranges")]
    pub profile: Profile,
    #[arg(long, value_enum, default_value = "log-normal")]
    pub distribution: Distribution,
    #[arg(long, default_value_t = 2)]
    pub concurrency: usize,
    #[arg(long, default_value_t = 3)]
    pub samples: usize,
    #[arg(long, default_value_t = 19950)]
    pub socks_port: u16,
    #[arg(long, default_value_t = 19877)]
    pub mcp_port: u16,
}
fn distribution(family: Distribution, unit: &str, min: u64, max: u64) -> Value {
    let name = match family {
        Distribution::Uniform => "uniform",
        Distribution::Exponential => "exponential",
        Distribution::Weibull => "weibull",
        Distribution::LogNormal => "log_normal",
        Distribution::WeightedDiscrete => "weighted_discrete",
    };
    let mut v = json!({"distribution":name});
    v[format!("min_{unit}")] = json!(min);
    v[format!("max_{unit}")] = json!(max);
    let middle = (min + max) / 2;
    match family {
        Distribution::Uniform => (),
        Distribution::Exponential => v[format!("mean_{unit}")] = json!(middle as f64),
        Distribution::Weibull => {
            v[format!("scale_{unit}")] = json!(middle as f64);
            v["shape"] = json!(0.8);
        }
        Distribution::LogNormal => {
            v[format!("median_{unit}")] = json!(middle as f64);
            v["sigma"] = json!(0.65);
        }
        Distribution::WeightedDiscrete => {
            v[format!("values_{unit}")] = json!([min, middle, max]);
            v["weights"] = json!([1, 2, 1]);
        }
    }
    v
}
pub fn config(
    profile: Profile,
    family: Distribution,
    concurrency: usize,
) -> Result<Option<x402_treazury::cover::Config>> {
    if matches!(profile, Profile::None) {
        return Ok(None);
    }
    let mut v = json!({"ranges_enabled":!matches!(profile,Profile::Padding),"url":"https://www.rfc-editor.org/rfc/rfc9113.txt","volume_mode":"extra_body","concurrency":concurrency,"max_requests_per_episode":8,"max_cover_body_bytes_per_episode":16384,"max_episode_ms":10000,"qualification_range_bytes":1024,"max_resource_bytes":16777216,"volume":distribution(family,"bytes",4096,16384),"ranges":distribution(family,"bytes",1024,4096),"start_delay":distribution(family,"ms",0,100),"request_gap":distribution(family,"ms",25,250),"tail":distribution(family,"ms",5000,8000)});
    if matches!(profile, Profile::Padding | Profile::Combined) {
        v["padding"] = json!({"header_name":"x-treazury-cover","on_api_requests":true,"on_cover_requests":false,"max_value_bytes_per_request":512,"max_value_bytes_per_episode":8192,"max_total_header_list_bytes":8192,"size":distribution(family,"bytes",32,512)});
    }
    let config: x402_treazury::cover::Config = serde_json::from_value(v)?;
    config.validate()?;
    Ok(Some(config))
}
pub fn write(options: Options) -> Result<Value> {
    ensure!(
        options.directory.is_absolute(),
        "fixture directory must be absolute"
    );
    ensure!(
        (1..=3).contains(&options.concurrency) && (1..=16).contains(&options.samples),
        "cover fixture allows concurrency 1..3 and samples 1..16"
    );
    ensure!(
        options.socks_port != 0 && options.mcp_port != 0 && options.socks_port != options.mcp_port,
        "distinct nonzero fixture ports required"
    );
    let binary = options.binary.canonicalize()?;
    let tor_binary = options.tor_binary.canonicalize()?;
    let cover = config(options.profile, options.distribution, options.concurrency)?;
    files::create_dir(&options.directory)?;
    let root = &options.directory;
    let state = root.join("state");
    let store = x402_treazury::rotation::store::Store::create(
        &state,
        &root.join("synthetic.key"),
        1,
        b"synthetic cover fixture: never fund",
    )?;
    let id = store.id().to_owned();
    drop(store);
    files::create_dir(&root.join("evidence"))?;
    let spec = json!({"openapi":"3.0.3","info":{"title":"Unsigned public-document qualification","version":"1"},"paths":{"/rfc/rfc9113.txt":{"get":{"description":"Read public RFC9113 text, without payment."}}}});
    files::publish(&root.join("spec.json"), &serde_json::to_vec_pretty(&spec)?)?;
    files::publish(
        &root.join("help.json"),
        br#"{"openapi":"3.0.3","paths":{}}"#,
    )?;
    let source = x402_treazury::catalog::Config {
        spec: "spec.json".into(),
        base_url: Some("https://www.rfc-editor.org".into()),
        prefix: Some("api".into()),
        probe_pricing: false,
        cover_traffic: cover.clone(),
        ..Default::default()
    };
    let tool = x402_treazury::catalog::build_tools(&source, &spec, "api")?
        .into_iter()
        .next()
        .unwrap()
        .name;
    let mut deployment = json!({"version":1,"treasury":{"id":id,"state_dir":"state","key_file":"nonexistent-key","indexer_url_env":"ABSENT_INDEXER","submission_url_env":"ABSENT_SUBMISSION","daily_input_zec":"0.1","shield_max_fee_zec":"0.001"},"network":{"mode":"tor","socks_endpoint":format!("127.0.0.1:{}",options.socks_port),"isolation_namespace":format!("cover_{}",uuid::Uuid::new_v4().simple()),"cover_traffic_enabled":cover.is_some()},"wallets":{"test":{"mode":"static","private_key_env":"ABSENT_KEY"}},"sources":{"api":source,"trace":{"spec":"help.json","base_url":"https://www.cloudflare.com","prefix":"trace","help_url":"https://www.cloudflare.com/cdn-cgi/trace","probe_pricing":false},"fresh":{"spec":"help.json","base_url":"https://www.cloudflare.com","prefix":"fresh","help_url":"https://www.cloudflare.com/cdn-cgi/trace?treazury_cover_qualification=uncached","probe_pricing":false}},"servers":{"main":{"listen":format!("127.0.0.1:{}",options.mcp_port),"sources":["api","trace","fresh"],"wallet":"test","bearer_token_env":"TOKEN"}}});
    // JSON optional nulls must not be emitted as TOML values.
    fn strip_null(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.retain(|_, v| !v.is_null());
                for v in m.values_mut() {
                    strip_null(v);
                }
            }
            Value::Array(a) => {
                for v in a {
                    strip_null(v);
                }
            }
            _ => (),
        }
    }
    strip_null(&mut deployment);
    let deployment: toml::Value = serde_json::from_value(deployment)?;
    files::publish(
        &root.join("deployment.toml"),
        toml::to_string_pretty(&deployment)?.as_bytes(),
    )?;
    let mut cases = vec![];
    let mut connected = vec!["warm".to_owned()];
    for n in 0..options.samples {
        let case = format!("api_{n}");
        connected.push(case.clone());
        cases.push(json!({"id":case,"server":"main","source":"api","tool":tool,"arguments":{},"reserve_usdc":"0","reviewed_read_only":true,"unsigned":true}));
    }
    for (case, source) in [
        ("warm", "trace"),
        ("cached", "trace"),
        ("uncached", "fresh"),
    ] {
        cases.push(json!({"id":case,"server":"main","source":source,"tool":format!("{source}_help"),"arguments":{},"reserve_usdc":"0","reviewed_read_only":true,"unsigned":true}));
    }
    let now = x402_treazury::rotation::base::now()?;
    let manifest: manifest::Manifest = serde_json::from_value(
        json!({"version":1,"run_id":"cover_unsigned","treasury_id":id,"deployment":root.join("deployment.toml"),"binary":binary,"evidence_dir":root.join("evidence"),"registry_authorization":"zero_spend","start":{"mode":"funded_pools","pools":[]},"network":{"tor_mode":"owned","tor_binary":tor_binary,"confinement":"macos_sandbox","require_isolation_evidence":true},"limits":{"api_reservation_usdc":"0","source_exposure_zec":"0","new_funding_jobs":0,"max_in_flight":1,"run_seconds":600,"phase_seconds":300,"call_seconds":240,"cleanup_seconds":20,"result_bytes":1048576,"post_batch_wait_ms":10000},"catalog":{"execution":"frozen","record_live_discovery":true},"windows":[{"id":"now","not_before":now,"not_after":now+86400}],"cases":cases,"phases":[{"id":"connected","window":"now","cases":connected,"pools":[],"required":true,"scenario":{"kind":"unsigned"}},{"id":"outage","window":"now","cases":["cached","uncached"],"pools":[],"required":true,"depends_on":["connected"],"scenario":{"kind":"tor_outage","warm_case":"warm","cached_case":"cached","uncached_case":"uncached"}}]}),
    )?;
    manifest.validate()?;
    let auth = registry::Authorization {
        version: 1,
        id: "zero_spend".into(),
        treasury_id: id,
        cumulative_api_usdc: "0".into(),
        cumulative_source_zec: "0".into(),
        cumulative_new_jobs: 0,
    };
    drop(registry::Registry::authorize(&state, &auth, now as i64)?);
    files::publish(
        &root.join("run.toml"),
        toml::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    files::publish(&root.join("SYNTHETIC_DO_NOT_FUND.txt"),b"Synthetic test state, not a Zcash wallet to fund. No live requests have run. Review deployment.toml and run.toml before qualify-tor. Header padding is experimental and independently optional; public resource availability does not establish vendor approval.\n")?;
    Ok(
        json!({"state_dir":state,"manifest":root.join("run.toml"),"profile":format!("{:?}",options.profile),"distribution":format!("{:?}",options.distribution),"samples":options.samples,"live_requests":0,"spending_authority":0}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn writer_creates_loadable_zero_authority_fixture_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fixture");
        let exe = std::env::current_exe().unwrap();
        let result = write(Options {
            directory: root.clone(),
            binary: exe.clone(),
            tor_binary: exe,
            profile: Profile::Combined,
            distribution: Distribution::Weibull,
            concurrency: 3,
            samples: 2,
            socks_port: 19950,
            mcp_port: 19877,
        })
        .unwrap();
        assert_eq!(result["live_requests"], 0);
        assert_eq!(result["spending_authority"], 0);
        let manifest = manifest::Manifest::load(&root.join("run.toml"))
            .await
            .unwrap();
        assert!(manifest.cases.iter().all(|case| case.unsigned));
        let deployment: x402_treazury::deployment::MetaConfig =
            toml::from_str(&std::fs::read_to_string(root.join("deployment.toml")).unwrap())
                .unwrap();
        assert!(deployment.network.cover_traffic_enabled);
        assert!(root.join("SYNTHETIC_DO_NOT_FUND.txt").is_file());
    }
    #[test]
    fn all_profiles_distributions_and_widths_validate() {
        for p in [
            Profile::None,
            Profile::Ranges,
            Profile::Padding,
            Profile::Combined,
        ] {
            for d in [
                Distribution::Uniform,
                Distribution::Exponential,
                Distribution::Weibull,
                Distribution::LogNormal,
                Distribution::WeightedDiscrete,
            ] {
                for c in 1..=3 {
                    config(p, d, c).unwrap();
                }
            }
        }
    }
}
