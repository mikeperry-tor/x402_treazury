//! Unsigned, opt-in Tor diagnostics. No wallet, signer, paid retry or automatic retry.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Parser;
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use x402_treazury::{catalog, config, network};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    network_config: PathBuf,
    #[arg(long, default_value = "tests/live/preflight.json")]
    manifest: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    provider: String,
    config: PathBuf,
    method: String,
    url: String,
    body: Option<Value>,
}
async fn bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        ensure!(
            chunk.len() <= limit.saturating_sub(data.len()),
            "response exceeds {limit}-byte limit; rejected without truncation"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
fn prices(challenge: &Value) -> Value {
    let offers: Vec<_> = challenge["accepts"].as_array().into_iter().flatten().map(|a| {
        let base_usdc = a["network"] == "eip155:8453" && a["asset"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"));
        let atomic = a["amount"].as_str().and_then(|s| s.parse::<u128>().ok());
        json!({"scheme":a["scheme"],"network":a["network"],"asset":a["asset"],"amount":a["amount"],
            "base_usdc":base_usdc,"above_one_cent":if base_usdc {atomic.map(|n|n>10000)} else {None},
            "usdc":if base_usdc {atomic.map(|n|format!("{}.{:06}",n/1000000,n%1000000))} else {None},
            "extra_keys":a["extra"].as_object().map(|m|m.keys().collect::<Vec<_>>()),
            "domain":a["extra"]["name"],"transfer_method":a["extra"]["assetTransferMethod"],
            "note":"observed offer, not spent; managed compatibility not established by this preflight"})
    }).collect();
    json!(offers)
}
async fn inspect(case: Case) -> Value {
    let start = Instant::now();
    let mut report = json!({"provider":case.provider,"method":case.method,"url":case.url,"paid_submission":"not_attempted","settlement":"not_attempted","classification":"unsigned Tor diagnostic; not MCP or managed-payment qualification"});
    let cfg = match config::load(&case.config).await {
        Ok(c) => c.settings,
        Err(e) => {
            report["config"] = json!({"error":format!("{e:#}")});
            return report;
        }
    };
    let spec_result = async {
        let origin=if cfg.spec.starts_with("https://") {cfg.spec.as_str()} else {case.url.as_str()};
        let http=network::discovery(origin,Duration::from_secs(60))?;
        let root=catalog::load_json_with_limit(&cfg.spec,&http,cfg.max_spec_bytes).await?;
        let tools=catalog::build_tools(&cfg,&root,cfg.prefix.as_deref().unwrap_or("api"))?;
        Ok::<_,anyhow::Error>(json!({"status":"pass","local":!cfg.spec.starts_with("https://"),"tools":tools.len(),
            "pricing_key":cfg.pricing_key,"pricing_probe":"not_attempted; production startup eligibility/cache tested separately"}))
    }.await;
    report["spec_catalog"] = match spec_result {
        Ok(v) => v,
        Err(e) => json!({"status":"fail","error":format!("{e:#}")}),
    };
    report["help"] = if let Some(url) = &cfg.help_url {
        let result=async {
            let response=network::discovery(url,Duration::from_secs(60))?.get(url).send().await.map_err(reqwest::Error::without_url)?;
            let status=response.status().as_u16();
            let data=bytes(response,cfg.max_help_bytes).await?;
            Ok::<_,anyhow::Error>(json!({"status":if (200..300).contains(&status){"pass"}else{"fail"},"http_status":status,"bytes":data.len(),"note":"retrieval only; semantic review and MCP lazy-cache qualification pending"}))
        }.await;
        match result {
            Ok(v) => v,
            Err(e) => json!({"status":"fail","error":format!("{e:#}")}),
        }
    } else {
        json!({"status":"not_applicable"})
    };
    let result=async {
        let identity=network::IsolationId::evm("0x0000000000000000000000000000000000000001")?;
        let http=network::global().http(&identity,&case.url,Duration::from_secs(60))?;
        let mut request=http.request(case.method.parse()?,&case.url);
        if let Some(body)=&case.body { request=request.json(body); }
        let response=request.send().await.map_err(reqwest::Error::without_url)?;
        let status=response.status().as_u16();
        let encoded=response.headers().get("payment-required").map(|v|v.to_str().map(str::to_owned)).transpose()?;
        let body=bytes(response,2_000_000).await?;
        let challenge:Value=if let Some(encoded)=encoded {serde_json::from_slice(&STANDARD.decode(encoded)?)?}else{serde_json::from_slice(&body).unwrap_or(Value::Null)};
        Ok::<_,anyhow::Error>(json!({"status":if status==402 || (200..300).contains(&status){"pass"}else{"fail"},"http_status":status,"bytes":body.len(),"offers":prices(&challenge),
            "extensions":challenge["extensions"].as_object().map(|m|m.keys().collect::<Vec<_>>()),"free_response":(200..300).contains(&status)}))
    }.await;
    report["unsigned_request"] = match result {
        Ok(v) => v,
        Err(e) => json!({"status":"fail","error":format!("{e:#}")}),
    };
    report["elapsed_ms"] = json!(start.elapsed().as_millis());
    report
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let policy = network::NetworkPolicy::load(&args.network_config)?;
    ensure!(
        serde_json::to_value(&policy)?["mode"] == "tor",
        "preflight requires Tor mode"
    );
    network::install(policy)?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&args.manifest)?)?;
    ensure!(!cases.is_empty(), "empty manifest");
    for c in &cases {
        ensure!(
            matches!(c.method.as_str(), "GET" | "POST")
                && c.url.starts_with("https://")
                && !c.url.contains('@'),
            "invalid unsigned case"
        );
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&args.output)
        .context("create new private report")?;
    // Sequential by design: shared hosts are not hammered and every completed case is durable.
    let mut results = stream::iter(cases).map(inspect).buffered(1);
    while let Some(result) = results.next().await {
        writeln!(file, "{}", result)?;
        file.sync_data()?;
        eprintln!("preflight complete: {}", result["provider"]);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_covers_every_provider() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/live/preflight.json")).unwrap();
        fn collect(path: &std::path::Path, paths: &mut std::collections::BTreeSet<PathBuf>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    collect(&p, paths);
                } else if p.extension().is_some_and(|e| e == "toml") {
                    paths.insert(p);
                }
            }
        }
        let mut expected = std::collections::BTreeSet::new();
        collect(std::path::Path::new("providers"), &mut expected);
        assert_eq!(
            cases
                .iter()
                .map(|c| c.config.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            cases
                .iter()
                .map(|c| &c.provider)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            cases.len()
        );
    }
    #[test]
    fn price_threshold_is_exact_and_unknown_is_not_free() {
        for (amount, expected) in [("9999", false), ("10000", false), ("10001", true)] {
            let p = prices(
                &json!({"accepts":[{"network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":amount}]}),
            );
            assert_eq!(p[0]["above_one_cent"], expected);
        }
        assert!(prices(&json!({"accepts":[{"amount":"10001"}]}))[0]["above_one_cent"].is_null());
    }
}
