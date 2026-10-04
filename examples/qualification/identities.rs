use super::*;
use std::collections::BTreeMap;
use x402_treazury::network::IsolationId;

pub async fn export(manifest: &Manifest, output: &Path) -> Result<()> {
    let shown = settings(manifest).await?;
    let table = x402_treazury::config::read_table(&manifest.deployment).await?;
    let policy: NetworkPolicy = table
        .get("network")
        .context("missing network")?
        .clone()
        .try_into()?;
    let context = NetworkContext::new(policy)?;
    let state = store::status(Path::new(
        shown["treasury"]["state_dir"]
            .as_str()
            .context("missing state path")?,
    ))?;
    ensure!(
        Some(state.treasury_id.as_str()) == shown["treasury"]["id"].as_str(),
        "treasury identity mismatch"
    );
    let mut entries = BTreeMap::new();
    let mut add = |label: String, identity: IsolationId, kind: &str, target: Option<String>| {
        let (user, password) = context.credentials(&identity);
        entries.insert(
            label,
            json!({"user":user,"password":password,"kind":kind,"target":target}),
        );
    };
    add(
        "treasury".into(),
        IsolationId::treasury(&state.treasury_id),
        "treasury",
        None,
    );
    for pool in state.pools {
        for address in pool.addresses {
            add(
                format!("evm_{}_{}", pool.name, address.id),
                IsolationId::evm(&address.address)?,
                "evm",
                None,
            );
        }
    }
    let mut origins = BTreeSet::from(["https://1click.chaindefuser.com".to_owned()]);
    if let Some(name) = shown["funding"]["base_rpc_url_env"].as_str() {
        if let Ok(url) = std::env::var(name) {
            origins.insert(reqwest::Url::parse(&url)?.origin().ascii_serialization());
        } else {
            eprintln!(
                "Base RPC discovery identity omitted: set the configured {name} environment variable when exporting the final map. Unknown streams will fail verification."
            );
        }
    }
    for source in shown["sources"]
        .as_object()
        .context("missing sources")?
        .values()
    {
        for name in ["spec", "help_url", "base_url"] {
            if let Some(url) = source["settings"][name].as_str()
                && (url.starts_with("https://") || url.starts_with("http://"))
            {
                origins.insert(reqwest::Url::parse(url)?.origin().ascii_serialization());
            }
        }
    }
    for origin in origins {
        let url = reqwest::Url::parse(&origin)?;
        let target = format!(
            "{}:{}",
            url.host_str().context("missing host")?,
            url.port_or_known_default().context("missing port")?
        );
        add(
            format!("discovery_{}", hash(&origin)),
            IsolationId::discovery(&origin)?,
            "discovery",
            Some(target),
        );
    }
    let mut file = ledger::private_file(output)?;
    serde_json::to_writer_pretty(&mut file, &entries)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!(
        "Wrote private Tor identity map; retain it privately with control events. No network requests or payments performed."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn private_identity_map_binds_existing_wallets_without_keys_or_network() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let key = temp.path().join("key");
        let mut store = store::Store::create(&state, &key, 1, b"offline").unwrap();
        let id = store.id().to_owned();
        store.ensure_pool("coverage_a", "1.717124").unwrap();
        drop(store);
        let original =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/live-qualification.toml");
        let mut table = x402_treazury::config::read_table(&original).await.unwrap();
        table["treasury"]["id"] = toml::Value::String(id.clone());
        table["treasury"]["state_dir"] = toml::Value::String(state.to_str().unwrap().into());
        table["treasury"]["key_file"] = toml::Value::String("does-not-exist".into());
        for (_, source) in table["sources"].as_table_mut().unwrap().iter_mut() {
            source["extends"] = toml::Value::String(
                original
                    .parent()
                    .unwrap()
                    .join("../providers/socialfetch.toml")
                    .to_str()
                    .unwrap()
                    .into(),
            );
        }
        let deployment = temp.path().join("deployment.toml");
        std::fs::write(&deployment, toml::to_string(&table).unwrap()).unwrap();
        let mut manifest: Manifest =
            toml::from_str(include_str!("../../tests/live/driver.example.toml")).unwrap();
        manifest.deployment = deployment;
        let output = temp.path().join("identities.json");
        export(&manifest, &output).await.unwrap();
        assert!(
            export(&manifest, &output).await.is_err(),
            "cannot overwrite private evidence"
        );
        let entries: BTreeMap<String, Value> =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        let evms: Vec<_> = entries.values().filter(|v| v["kind"] == "evm").collect();
        assert_eq!(evms.len(), 2);
        assert_ne!(evms[0]["password"], evms[1]["password"]);
        assert!(
            entries
                .values()
                .any(|v| v["target"] == "api.socialfetch.dev:443")
        );
        assert!(
            entries
                .values()
                .any(|v| v["target"] == "1click.chaindefuser.com:443")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
