//! Freeze qualification catalogs independently; failures retain explicit snapshot provenance.
use super::*;
use std::collections::BTreeMap;
use x402_treazury::{catalog, config};

pub async fn freeze(manifest: &Manifest, output: &Path, evidence: &Path) -> Result<()> {
    let input = manifest.deployment.canonicalize()?;
    ensure!(
        output
            .parent()
            .context("missing output parent")?
            .canonicalize()?
            == input.parent().unwrap(),
        "frozen deployment must remain beside its input so relative paths retain meaning"
    );
    ensure!(!output.exists(), "frozen deployment already exists");
    let mut table = config::read_table(&input).await?;
    let policy: NetworkPolicy = table
        .get("network")
        .context("missing network")?
        .clone()
        .try_into()?;
    ensure!(
        policy.mode == x402_treazury::network::Mode::Tor,
        "catalog capture requires Tor"
    );
    let context = NetworkContext::new(policy)?;
    let network = &context;
    std::fs::create_dir(evidence).context("catalog evidence directory already exists")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(evidence, std::fs::Permissions::from_mode(0o700))?;
    }
    let evidence = evidence.canonicalize()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/catalogs/cases.json"))?;
    let mut fallbacks = BTreeMap::new();
    for fixture in fixtures {
        if let Some(spec) = fixture["spec"].as_str() {
            fallbacks.insert(
                root.join(fixture["provider"].as_str().unwrap())
                    .canonicalize()?,
                root.join(spec),
            );
        }
    }
    for name in ["arkham", "botsmith", "google-trends", "x402stock"] {
        fallbacks.insert(
            root.join(format!("providers/{name}.toml")).canonicalize()?,
            root.join(format!("tests/live/fixtures/{name}.json")),
        );
    }
    let mut sources = Vec::new();
    let mut requests = BTreeMap::new();
    for (name, raw) in table["sources"].as_table().context("missing sources")? {
        let mut local = raw.as_table().context("invalid source")?.clone();
        local.remove("wallet");
        let provider = local
            .get("extends")
            .and_then(|v| v.as_str())
            .map(|p| input.parent().unwrap().join(p).canonicalize())
            .transpose()?;
        let cfg = config::resolve(local, &input).await?.settings;
        let key = format!("{}:{}:{}", cfg.spec, cfg.max_spec_bytes, cfg.timeout);
        sources.push((name.clone(), key.clone()));
        requests
            .entry(key)
            .or_insert((cfg, provider.and_then(|p| fallbacks.get(&p).cloned())));
    }
    use futures_util::{StreamExt, stream};
    let jobs = stream::iter(requests)
        .map(|(key, (cfg, fallback))| async move {
            let started = now()?;
            let origin = if cfg.spec.starts_with("http") {
                cfg.spec.as_str()
            } else {
                "https://local.invalid"
            };
            let http = network.discovery(origin, Duration::from_secs_f64(cfg.timeout))?;
            let loaded = catalog::load_json_with_limit(&cfg.spec, &http, cfg.max_spec_bytes).await;
            Ok::<_, anyhow::Error>((key, cfg.spec.clone(), fallback, loaded, started, now()?))
        })
        .buffered(3);
    futures_util::pin_mut!(jobs);
    let mut files = BTreeMap::new();
    let mut report = ledger::private_file(&evidence.join("catalogs.jsonl"))?;
    while let Some(job) = jobs.next().await {
        let (key, spec, fallback, loaded, started, finished) = job?;
        let (document, status) = select(loaded, fallback.as_deref())?;
        let bytes = serde_json::to_vec(&document)?;
        let digest = hash(&bytes);
        let path = evidence.join(format!("{}.json", hash(&key)));
        let mut file = ledger::private_file(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        writeln!(
            report,
            "{}",
            json!({"source_spec":spec,"local":!spec.starts_with("http"),"started_at":started,"finished_at":finished,"sha256":digest,"frozen_spec":path,"result":status})
        )?;
        report.sync_all()?;
        eprintln!("Catalog captured: {} ({})", spec, status["catalog_source"]);
        files.insert(key, path);
    }
    for (name, key) in sources {
        table["sources"][&name]
            .as_table_mut()
            .context("invalid source")?
            .insert(
                "spec".into(),
                toml::Value::String(files[&key].to_string_lossy().into()),
            );
    }
    let mut file = ledger::private_file(output)?;
    file.write_all(toml::to_string_pretty(&table)?.as_bytes())?;
    file.sync_all()?;
    eprintln!(
        "Frozen qualification deployment written; live failures remain recorded. No keys, funding or payments accessed."
    );
    Ok(())
}

fn select(loaded: Result<Value>, fallback: Option<&Path>) -> Result<(Value, Value)> {
    match loaded {
        Ok(value) => Ok((
            value,
            json!({"load":"pass","catalog_source":"configured_spec"}),
        )),
        Err(error) => {
            let message = format!("{error:#}");
            eprintln!(
                "Catalog load failed: {message}; considering explicitly reviewed qualification snapshot"
            );
            let path =
                fallback.context("catalog failed and no reviewed qualification snapshot exists")?;
            let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
            ensure!(
                value.get("paths").is_some(),
                "reviewed snapshot is not an OpenAPI document"
            );
            Ok((
                value,
                json!({"load":"fail","error":message,"catalog_source":"reviewed_snapshot","snapshot":path}),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn aliases_share_frozen_document_and_keep_independent_settings() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.toml");
        std::fs::write(
            dir.path().join("spec.json"),
            r#"{"paths":{"/read":{"get":{}}}}"#,
        )
        .unwrap();
        std::fs::write(
            &input,
            r#"
[network]
mode = "tor"
socks_endpoint = "127.0.0.1:9050"
[sources.one]
spec = "spec.json"
prefix = "first"
wallet = "a"
[sources.two]
spec = "spec.json"
prefix = "second"
wallet = "b"
"#,
        )
        .unwrap();
        let mut m: Manifest =
            toml::from_str(include_str!("../../tests/live/driver.example.toml")).unwrap();
        m.deployment = input;
        let output = dir.path().join("output.toml");
        let evidence = dir.path().join("evidence");
        freeze(&m, &output, &evidence).await.unwrap();
        let frozen = config::read_table(&output).await.unwrap();
        assert_eq!(
            frozen["sources"]["one"]["spec"],
            frozen["sources"]["two"]["spec"]
        );
        assert_eq!(frozen["sources"]["one"]["wallet"].as_str(), Some("a"));
        assert_eq!(frozen["sources"]["two"]["prefix"].as_str(), Some("second"));
        let report = std::fs::read_to_string(evidence.join("catalogs.jsonl")).unwrap();
        assert_eq!(report.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&report).unwrap()["local"],
            true
        );
        assert!(freeze(&m, &output, &evidence).await.is_err());
    }
    #[test]
    fn capture_preserves_failure_and_rejects_missing_or_invalid_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spec.json");
        std::fs::write(&path, r#"{"paths":{"/read":{"get":{}}}}"#).unwrap();
        let (doc, status) = select(Err(anyhow::anyhow!("timed out")), Some(&path)).unwrap();
        assert!(doc["paths"]["/read"].is_object());
        assert_eq!(status["load"], "fail");
        assert_eq!(status["error"], "timed out");
        assert_eq!(status["catalog_source"], "reviewed_snapshot");
        assert!(select(Err(anyhow::anyhow!("timeout")), None).is_err());
        std::fs::write(&path, "{}").unwrap();
        assert!(select(Err(anyhow::anyhow!("timeout")), Some(&path)).is_err());
        let live = json!({"paths":{"/live":{"get":{}}}});
        let (doc, status) = select(Ok(live.clone()), Some(&path)).unwrap();
        assert_eq!(doc, live);
        assert_eq!(status["load"], "pass");
        assert_eq!(status["catalog_source"], "configured_spec");
    }
}
