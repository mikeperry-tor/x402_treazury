//! Destination constraints for managed runs. Rebuild after execution to include
//! newly allocated wallets, using durable addresses rather than observed tokens.
use super::*;
use x402_treazury::{deployment::MetaConfig, rotation::assignment};
fn target(url: &str) -> Result<String> {
    let url = reqwest::Url::parse(url)?;
    ensure!(
        matches!(url.scheme(), "https" | "http"),
        "unsupported managed network URL"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "managed endpoint has embedded credentials"
    );
    let host = url
        .domain()
        .context("Tor identity target requires a remote hostname")?;
    Ok(format!(
        "{}:{}",
        host,
        url.port_or_known_default()
            .context("endpoint port missing")?
    )
    .to_ascii_lowercase())
}
fn insert(
    map: &mut Identities,
    context: &NetworkContext,
    label: String,
    id: IsolationId,
    kind: &str,
    targets: BTreeSet<String>,
    permitted: bool,
) -> Result<()> {
    let (user, password) = context.credentials(&id);
    if let Some(existing) = map.get_mut(&label) {
        ensure!(
            existing.user == user && existing.password == password && existing.kind == kind,
            "identity label changed"
        );
        existing.targets.extend(targets);
        existing.permitted |= permitted;
    } else {
        ensure!(
            !map.values()
                .any(|v| v.user == user && v.password == password),
            "credential alias across identity labels"
        );
        map.insert(
            label,
            Identity {
                user,
                password,
                kind: kind.into(),
                targets,
                permitted,
                required: false,
                required_targets: BTreeSet::new(),
            },
        );
    }
    Ok(())
}
fn infrastructure_discovery(
    map: &mut Identities,
    context: &NetworkContext,
    url: &str,
) -> Result<()> {
    let origin = reqwest::Url::parse(url)?.origin().ascii_serialization();
    insert(
        map,
        context,
        format!("discovery_{}", files::hash(&origin)),
        IsolationId::discovery(&origin)?,
        "discovery",
        BTreeSet::from([target(url)?]),
        true,
    )
}
pub fn managed_map(
    m: &Manifest,
    snapshot: &Value,
    state: &std::path::Path,
    lookup: impl Fn(&str) -> Option<String>,
    payments: &[Value],
) -> Result<Identities> {
    managed_map_inner(m, snapshot, state, lookup, payments, true)
}
pub fn managed_map_frozen(
    m: &Manifest,
    snapshot: &Value,
    state: &std::path::Path,
    lookup: impl Fn(&str) -> Option<String>,
    payments: &[Value],
) -> Result<Identities> {
    managed_map_inner(m, snapshot, state, lookup, payments, false)
}
fn managed_map_inner(
    m: &Manifest,
    snapshot: &Value,
    state: &std::path::Path,
    lookup: impl Fn(&str) -> Option<String>,
    payments: &[Value],
    fetched: bool,
) -> Result<Identities> {
    let mut map = discovery_catalogs(m, snapshot, fetched)?;
    let status = x402_treazury::rotation::store::status(state)?;
    ensure!(
        status.treasury_id == m.treasury_id,
        "identity export treasury mismatch"
    );
    // This is the actual execution config, not `--show-config` inspection JSON.
    // Resolve named/generated wallets through production assignment and check
    // the stored inventory against that resolution before granting any identity.
    let config: MetaConfig = serde_json::from_value(snapshot["deployment"].clone())?;
    let resolution = assignment::resolve(&config)?;
    let context = NetworkContext::new(config.network.clone())?;
    let treasury = config
        .treasury
        .as_ref()
        .context("treasury config missing")?;
    let inventory = snapshot["inventory"]
        .as_array()
        .context("inventory missing")?;
    for listener in inventory {
        let server = listener["server"]
            .as_str()
            .context("listener name missing")?;
        let bindings = resolution
            .bindings
            .get(server)
            .context("listener has no resolved bindings")?;
        ensure!(
            serde_json::to_value(bindings)? == listener["wallet_bindings"],
            "snapshot inventory wallet bindings differ from production resolution"
        );
    }
    for pool in &m.start.pools {
        ensure!(
            resolution.wallets.get(pool).is_some_and(|w| w.managed()),
            "selected managed pool missing from resolved execution config"
        );
    }
    ensure!(
        treasury.id == status.treasury_id,
        "configured treasury differs from durable identity"
    );
    let funding = config.funding.as_ref().context("funding config missing")?;
    let indexer = target(
        &lookup(&treasury.indexer_url_env).context("indexer endpoint environment missing")?,
    )?;
    let submission = target(
        &lookup(&treasury.submission_url_env).context("submission endpoint environment missing")?,
    )?;
    let mut wallet_targets = BTreeSet::new();
    for url in funding.base_rpc_urls(&lookup)? {
        let destination = target(&url)?;
        wallet_targets.insert(destination.clone());
        infrastructure_discovery(&mut map, &context, &url)?;
    }
    // The public token catalog is fetched before a wallet-bound quote. It uses
    // the origin discovery identity, independently of destination EVM wallets.
    infrastructure_discovery(&mut map, &context, x402_treazury::rotation::near::ORIGIN)?;
    // NEAR quotes and status, ZEC submission and lookup all bind to the
    // destination EVM identity; historical pending operations may reconcile too.
    wallet_targets.extend([
        target(x402_treazury::rotation::near::ORIGIN)?,
        indexer.clone(),
        submission.clone(),
    ]);
    insert(
        &mut map,
        &context,
        "treasury".into(),
        IsolationId::treasury(&status.treasury_id),
        "treasury",
        BTreeSet::from([indexer, submission]),
        true,
    )?;
    map.get_mut("treasury").unwrap().required = true;
    for pool in status.pools {
        let declared = resolution
            .wallets
            .get(&pool.name)
            .is_some_and(|w| w.managed());
        let mut destinations = if declared {
            wallet_targets.clone()
        } else {
            BTreeSet::new()
        };
        if declared {
            for listener in inventory {
                let server = listener["server"]
                    .as_str()
                    .context("listener name missing")?;
                for tool in listener["tools"].as_array().context("tools missing")? {
                    let source = tool["source"].as_str().context("tool source missing")?;
                    let binding = resolution
                        .bindings
                        .get(server)
                        .and_then(|b| b.get(source))
                        .context("tool source has no resolved wallet binding")?;
                    if binding.wallet != pool.name {
                        continue;
                    }
                    if tool["help_url"].is_string() {
                        continue;
                    }
                    let path = tool["path"].as_str().context("tool path missing")?;
                    let url = if path.starts_with("https://") || path.starts_with("http://") {
                        path
                    } else {
                        snapshot["sources"][source]["base_url"]
                            .as_str()
                            .context("source base URL missing")?
                    };
                    destinations.insert(target(url)?);
                }
            }
        }
        for wallet in pool.addresses {
            let label = format!("evm_{}_{}", pool.name, wallet.id);
            insert(
                &mut map,
                &context,
                label.clone(),
                IsolationId::evm(&wallet.address)?,
                "evm",
                destinations.clone(),
                declared,
            )?;
            for payment in payments.iter().filter(|p| p["wallet"] == wallet.id) {
                ensure!(
                    declared
                        && payment["pool"] == pool.id
                        && payment["address"]
                            .as_str()
                            .is_some_and(|a| a.eq_ignore_ascii_case(&wallet.address)),
                    "payment identity differs from durable wallet"
                );
                let case = m
                    .cases
                    .iter()
                    .find(|c| payment["case"] == c.id)
                    .context("payment case missing")?;
                ensure!(
                    !case.unsigned
                        && resolution
                            .bindings
                            .get(&case.server)
                            .and_then(|b| b.get(&case.source))
                            .is_some_and(|b| b.wallet == pool.name),
                    "payment differs from reviewed wallet assignment"
                );
                let listener = snapshot["inventory"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|l| l["server"] == case.server)
                    .context("case listener missing")?;
                let tool = listener["tools"]
                    .as_array()
                    .context("case tools missing")?
                    .iter()
                    .find(|t| t["name"] == case.tool && t["source"] == case.source)
                    .context("case tool missing")?;
                let path = tool["path"].as_str().context("paid tool path missing")?;
                let url = if path.starts_with("https://") || path.starts_with("http://") {
                    path
                } else {
                    snapshot["sources"][&case.source]["base_url"]
                        .as_str()
                        .context("paid origin missing")?
                };
                let entry = map.get_mut(&label).unwrap();
                entry.required = true;
                entry.required_targets.insert(target(url)?);
            }
        }
    }
    ensure!(
        payments.iter().all(|p| map.keys().any(|label| p["wallet"]
            .as_str()
            .is_some_and(|id| label.ends_with(&format!("_{id}"))))),
        "payment wallet absent from final identity export"
    );
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_catalog_discovery_is_distinct_and_destination_constrained() {
        let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
        let mut map = Identities::new();
        let origin = x402_treazury::rotation::near::ORIGIN;
        infrastructure_discovery(&mut map, &context, origin).unwrap();
        infrastructure_discovery(&mut map, &context, &format!("{origin}/v0/tokens")).unwrap();
        assert_eq!(map.len(), 1);
        let identity = map.values().next().unwrap();
        let credentials = context.credentials(&IsolationId::discovery(origin).unwrap());
        assert_eq!(
            (&identity.user, &identity.password),
            (&credentials.0, &credentials.1)
        );
        assert_eq!(identity.kind, "discovery");
        assert!(identity.permitted);
        assert!(!identity.required);
        assert_eq!(identity.targets, BTreeSet::from([target(origin).unwrap()]));
        let wallet = context
            .credentials(&IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap());
        assert_ne!(credentials, wallet);
        assert_ne!(
            credentials,
            context.credentials(&IsolationId::treasury("test"))
        );
        assert!(infrastructure_discovery(&mut map, &context, "https://127.0.0.1").is_err());
        assert!(infrastructure_discovery(&mut map, &context, "https://user@near.invalid").is_err());
    }
}
