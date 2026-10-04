//! Derive discovery identities from actual prepared origins, never copied tokens.
use super::audit::{Identities, Identity};
use crate::{files, manifest::Manifest};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
use x402_treazury::network::{IsolationId, NetworkContext, NetworkPolicy};
pub fn discovery(m: &Manifest, snapshot: &Value) -> Result<Identities> {
    let policy: NetworkPolicy = serde_json::from_value(snapshot["deployment"]["network"].clone())?;
    let context = NetworkContext::new(policy)?;
    let mut result = Identities::new();
    let mut add = |url: &str, required: bool| -> Result<()> {
        let url = reqwest::Url::parse(url)?;
        ensure!(
            ["https", "http"].contains(&url.scheme()),
            "unsupported discovery identity URL"
        );
        let origin = url.origin().ascii_serialization();
        let label = format!("discovery_{}", files::hash(&origin));
        let (user, password) = context.credentials(&IsolationId::discovery(&origin)?);
        let target = format!(
            "{}:{}",
            url.host_str().context("origin host missing")?,
            url.port_or_known_default().context("origin port missing")?
        )
        .to_ascii_lowercase();
        let entry = result.entry(label).or_insert_with(|| Identity {
            user,
            password,
            kind: "discovery".into(),
            targets: BTreeSet::new(),
            required: false,
            permitted: true,
        });
        entry.targets.insert(target);
        entry.required |= required;
        Ok(())
    };
    for source in snapshot["sources"]
        .as_object()
        .context("prepared sources missing")?
        .values()
    {
        for field in ["spec", "help_url", "base_url"] {
            if let Some(url) = source["settings"][field]
                .as_str()
                .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
            {
                add(url, field == "spec")?;
            }
        }
    }
    for case in &m.cases {
        let listener = snapshot["inventory"]
            .as_array()
            .context("inventory missing")?
            .iter()
            .find(|i| i["server"] == case.server)
            .context("listener missing")?;
        let tool = listener["tools"]
            .as_array()
            .context("tools missing")?
            .iter()
            .find(|t| t["source"] == case.source && t["name"] == case.tool)
            .context("tool missing")?;
        let url = if let Some(url) = tool["help_url"].as_str() {
            url.to_owned()
        } else {
            let path = tool["path"].as_str().context("tool path missing")?;
            if path.starts_with("https://") || path.starts_with("http://") {
                path.to_owned()
            } else {
                snapshot["sources"][&case.source]["base_url"]
                    .as_str()
                    .context("source base URL missing")?
                    .to_owned()
            }
        };
        add(&url, !super::outage::uncached(m, &case.id))?;
    }
    Ok(result)
}

/// Export every durable public address, including retired and replacement slots.
/// In a keyless session their presence is explanatory, never permission to use them.
pub fn unsigned_map(m: &Manifest, snapshot: &Value, state: &std::path::Path) -> Result<Identities> {
    let mut result = discovery(m, snapshot)?;
    let status = x402_treazury::rotation::store::status(state)?;
    ensure!(
        status.treasury_id == m.treasury_id,
        "identity export treasury mismatch"
    );
    let policy: NetworkPolicy = serde_json::from_value(snapshot["deployment"]["network"].clone())?;
    let context = NetworkContext::new(policy)?;
    let mut add = |label: String,
                   identity: IsolationId,
                   kind: &str,
                   targets: BTreeSet<String>|
     -> Result<()> {
        let (user, password) = context.credentials(&identity);
        if let Some(existing) = result
            .values()
            .find(|id| id.user == user && id.password == password)
        {
            ensure!(
                existing.kind == kind,
                "credential aliases differ in identity kind"
            );
            return Ok(());
        }
        ensure!(
            result
                .insert(
                    label,
                    Identity {
                        user,
                        password,
                        kind: kind.into(),
                        targets,
                        required: false,
                        permitted: false
                    }
                )
                .is_none(),
            "identity label collision"
        );
        Ok(())
    };
    add(
        "treasury".into(),
        IsolationId::treasury(&status.treasury_id),
        "treasury",
        BTreeSet::new(),
    )?;
    for pool in status.pools {
        for address in pool.addresses {
            add(
                format!("evm_{}_{}", pool.name, address.id),
                IsolationId::evm(&address.address)?,
                "evm",
                BTreeSet::new(),
            )?;
        }
    }
    // The keyless child deliberately inherits no endpoint environment. Resolve
    // configured defaults using the same production resolver; custom references
    // that cannot be resolved must not silently become another destination.
    if !snapshot["deployment"]["funding"].is_null() {
        let funding: x402_treazury::rotation::config::FundingConfig =
            serde_json::from_value(snapshot["deployment"]["funding"].clone())?;
        for url in funding.base_rpc_urls(|_| None)? {
            let url = reqwest::Url::parse(&url)?;
            let origin = url.origin().ascii_serialization();
            let target = format!(
                "{}:{}",
                url.host_str().context("RPC host missing")?,
                url.port_or_known_default().context("RPC port missing")?
            );
            add(
                format!("discovery_{}", files::hash(&origin)),
                IsolationId::discovery(&origin)?,
                "discovery",
                BTreeSet::from([target]),
            )?;
        }
    }
    Ok(result)
}
