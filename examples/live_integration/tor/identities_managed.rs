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
    // This is the actual execution config, not `config show` inspection JSON.
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
        treasury.id.is_empty() || treasury.id == status.treasury_id,
        "configured treasury differs from durable identity"
    );
    let funding = config.funding.as_ref().context("funding config missing")?;
    let indexer = target(&treasury.indexer_endpoint(&lookup)?)?;
    let submission = target(&treasury.submission_endpoint(&lookup)?)?;
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

    struct Fixture {
        _dir: tempfile::TempDir,
        state: std::path::PathBuf,
        manifest: Manifest,
        snapshot: Value,
        payment: Value,
        wallets: Vec<(String, String, String)>,
    }
    impl Fixture {
        fn new() -> Self {
            use serde_json::json;
            use x402_treazury::rotation::{
                config::{ReserveConfig, ReservesConfig},
                store::{AllocationKind, Store},
            };
            let dir = tempfile::tempdir().unwrap();
            let state = dir.path().join("state");
            let mut store = Store::create(
                &state,
                &dir.path().join("key"),
                1,
                b"unfunded-identity-fixture",
            )
            .unwrap();
            let reserves = ReservesConfig {
                small: ReserveConfig {
                    funding_amount_usdc: "2".into(),
                    max_funding_amount_usdc: None,
                },
                large: None,
            };
            for name in ["pool", "historical"] {
                let pool = store
                    .ensure_capacity_group(
                        name,
                        &reserves,
                        Some(alloy_primitives::U256::from(20000)),
                        0,
                    )
                    .unwrap();
                store
                    .allocate_capacity_kind(&pool, AllocationKind::Small, true)
                    .unwrap();
            }
            let status = store.status().unwrap();
            let mut manifest = crate::tests::manifest();
            manifest.treasury_id = status.treasury_id.clone();
            let wallets = status
                .pools
                .iter()
                .flat_map(|p| {
                    p.addresses
                        .iter()
                        .map(|a| (p.name.clone(), a.id.clone(), a.address.clone()))
                })
                .collect();
            let pool = status.pools.iter().find(|p| p.name == "pool").unwrap();
            let payment = json!({"case":"call","pool":pool.id,"wallet":pool.addresses[0].id,"address":pool.addresses[0].address});
            drop(store);
            let config: MetaConfig = serde_json::from_value(json!({
                "version":1,"network":NetworkPolicy::default(),
                "treasury":{"id":manifest.treasury_id,"state_dir":state,
                    "indexer_url_env":"INDEXER","submission_url_env":"SUBMIT"},
                "funding":{"base_rpc_url_env":"BASE","base_rpc_fallback_url_envs":[]},
                "wallets":{"pool":{"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"2"}},
                    "max_api_payment_usdc":"0.02","max_funding_spend_zec":"0.02","max_conversion_overhead_percent":5}},
                "sources":{"api":{"spec":"https://catalog.invalid/spec"},"other":{"spec":"local.json"}},
                "servers":{"main":{"listen":"127.0.0.1:0","bearer_token_env":"TOKEN","wallet":"pool","sources":["api","other"]}}
            })).unwrap();
            let resolution = assignment::resolve(&config).unwrap();
            let snapshot = json!({"deployment":config,
                "sources":{"api":{"settings":{"spec":"https://catalog.invalid/spec"},"base_url":"https://api.invalid"},
                    "other":{"settings":{"spec":"local.json"},"base_url":"https://other.invalid"}},
                "inventory":[{"server":"main","wallet_bindings":resolution.bindings["main"],"tools":[
                    {"source":"api","name":"api_read","path":"/read"},
                    {"source":"other","name":"other_read","path":"https://absolute.invalid:8443/read"},
                    {"source":"api","name":"api_help","path":"https://help.invalid/read","help_url":"https://help.invalid/read"}]}]});
            Self {
                _dir: dir,
                state,
                manifest,
                snapshot,
                payment,
                wallets,
            }
        }
        fn lookup(name: &str) -> Option<String> {
            match name {
                "INDEXER" => Some("https://indexer.invalid".into()),
                "SUBMIT" => Some("https://submit.invalid".into()),
                "BASE" => Some("https://base.invalid".into()),
                _ => None,
            }
        }
        fn map(&self, fetched: bool, payments: &[Value]) -> Result<Identities> {
            managed_map_inner(
                &self.manifest,
                &self.snapshot,
                &self.state,
                Self::lookup,
                payments,
                fetched,
            )
        }
    }

    #[test]
    fn managed_mapping_preserves_exact_destinations_and_historical_prohibition() {
        let f = Fixture::new();
        let before =
            serde_json::to_value(x402_treazury::rotation::store::status(&f.state).unwrap())
                .unwrap();
        let context = NetworkContext::new(NetworkPolicy::default()).unwrap();
        for fetched in [false, true] {
            let map = f
                .map(fetched, &[f.payment.clone(), f.payment.clone()])
                .unwrap();
            let catalog = &map[&format!("discovery_{}", files::hash("https://catalog.invalid"))];
            assert_eq!(catalog.required, fetched);
            assert_eq!(
                catalog.targets,
                BTreeSet::from(["catalog.invalid:443".into()])
            );
            assert!(map["treasury"].required);
            assert_eq!(
                map["treasury"].targets,
                BTreeSet::from(["indexer.invalid:443".into(), "submit.invalid:443".into()])
            );
            for (pool, id, address) in &f.wallets {
                let entry = &map[&format!("evm_{pool}_{id}")];
                let credentials = context.credentials(&IsolationId::evm(address).unwrap());
                assert!(entry.user == credentials.0 && entry.password == credentials.1);
                if pool == "historical" {
                    assert!(!entry.permitted && !entry.required);
                    assert!(entry.targets.is_empty() && entry.required_targets.is_empty());
                } else {
                    assert!(entry.permitted && entry.required);
                    assert_eq!(
                        entry.targets,
                        BTreeSet::from([
                            "api.invalid:443".into(),
                            "absolute.invalid:8443".into(),
                            "base.invalid:443".into(),
                            "indexer.invalid:443".into(),
                            "submit.invalid:443".into(),
                            target(x402_treazury::rotation::near::ORIGIN).unwrap()
                        ])
                    );
                    assert_eq!(
                        entry.required_targets,
                        BTreeSet::from(["api.invalid:443".into()])
                    );
                }
            }
            assert_eq!(map.len(), 7);
        }
        let no_payments = f.map(false, &[]).unwrap();
        for (pool, id, _) in &f.wallets {
            let entry = &no_payments[&format!("evm_{pool}_{id}")];
            assert!(!entry.required && entry.required_targets.is_empty());
        }
        assert_eq!(
            serde_json::to_value(x402_treazury::rotation::store::status(&f.state).unwrap())
                .unwrap(),
            before
        );
    }

    #[test]
    fn managed_mapping_rejects_changed_authority_and_payment_bindings() {
        use serde_json::json;
        let f = Fixture::new();
        for (field, value, expected) in [
            (
                "pool",
                json!("wrong"),
                "payment identity differs from durable wallet",
            ),
            (
                "address",
                json!("0x0000000000000000000000000000000000000001"),
                "payment identity differs from durable wallet",
            ),
            ("case", json!("missing"), "payment case missing"),
            (
                "wallet",
                json!("missing"),
                "payment wallet absent from final identity export",
            ),
        ] {
            let mut payment = f.payment.clone();
            payment[field] = value;
            assert_eq!(
                f.map(false, &[payment]).err().unwrap().to_string(),
                expected
            );
        }
        let mut upper = f.payment.clone();
        upper["address"] = json!(upper["address"].as_str().unwrap().to_ascii_uppercase());
        assert!(f.map(false, &[upper]).is_ok());
        let check = |m: &Manifest, snapshot: &Value, expected: &str| {
            let error = managed_map_inner(
                m,
                snapshot,
                &f.state,
                Fixture::lookup,
                std::slice::from_ref(&f.payment),
                false,
            )
            .err()
            .unwrap();
            assert_eq!(error.to_string(), expected);
        };
        let mut m = f.manifest.clone();
        m.treasury_id = "different".into();
        check(&m, &f.snapshot, "identity export treasury mismatch");
        let mut snapshot = f.snapshot.clone();
        snapshot["deployment"]["treasury"]["id"] = json!("different");
        check(
            &f.manifest,
            &snapshot,
            "configured treasury differs from durable identity",
        );
        let mut snapshot = f.snapshot.clone();
        snapshot["inventory"][0]["wallet_bindings"]["api"]["wallet"] = json!("different");
        check(
            &f.manifest,
            &snapshot,
            "snapshot inventory wallet bindings differ from production resolution",
        );
        let mut m = f.manifest.clone();
        m.start.pools = vec!["missing".into()];
        check(
            &m,
            &f.snapshot,
            "selected managed pool missing from resolved execution config",
        );
        let mut m = f.manifest.clone();
        m.cases[0].unsigned = true;
        check(
            &m,
            &f.snapshot,
            "payment differs from reviewed wallet assignment",
        );
        let mut snapshot = f.snapshot.clone();
        snapshot["deployment"]["wallets"]["static_payer"] = json!({
            "mode":"static","private_key_env":"UNUSED","max_api_payment_usdc":"0.02"});
        snapshot["deployment"]["sources"]["api"]["wallet"] = json!("static_payer");
        let config: MetaConfig = serde_json::from_value(snapshot["deployment"].clone()).unwrap();
        snapshot["inventory"][0]["wallet_bindings"] =
            serde_json::to_value(assignment::resolve(&config).unwrap().bindings["main"].clone())
                .unwrap();
        check(
            &f.manifest,
            &snapshot,
            "payment differs from reviewed wallet assignment",
        );
        let mut m = f.manifest.clone();
        m.start.pools = vec!["static_payer".into()];
        check(
            &m,
            &snapshot,
            "selected managed pool missing from resolved execution config",
        );
        let status = x402_treazury::rotation::store::status(&f.state).unwrap();
        let historical = status
            .pools
            .iter()
            .find(|p| p.name == "historical")
            .unwrap();
        let payment = json!({"case":"call","pool":historical.id,
            "wallet":historical.addresses[0].id,"address":historical.addresses[0].address});
        assert_eq!(
            f.map(false, &[payment]).err().unwrap().to_string(),
            "payment identity differs from durable wallet"
        );
        let mut snapshot = f.snapshot.clone();
        snapshot["inventory"][0]["tools"] = json!([]);
        check(&f.manifest, &snapshot, "tool missing");
    }

    #[test]
    fn duplicate_lookup_entries_keep_first_payment_case_and_tool() {
        use serde_json::json;
        let mut f = Fixture::new();
        let mut later_case = f.manifest.cases[0].clone();
        later_case.unsigned = true;
        f.manifest.cases.push(later_case);
        f.snapshot["inventory"][0]["tools"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "source":"api","name":"api_read","path":"https://later.invalid/other"}));
        let mut later_listener = f.snapshot["inventory"][0].clone();
        later_listener["tools"][0]["path"] = json!("https://later-listener.invalid/read");
        f.snapshot["inventory"]
            .as_array_mut()
            .unwrap()
            .push(later_listener);
        let map = f.map(false, std::slice::from_ref(&f.payment)).unwrap();
        let id = f.payment["wallet"].as_str().unwrap();
        let payer = &map[&format!("evm_pool_{id}")];
        assert_eq!(
            payer.required_targets,
            BTreeSet::from(["api.invalid:443".into()])
        );
        assert!(payer.targets.contains("later.invalid:443"));
        assert!(payer.targets.contains("later-listener.invalid:443"));
    }
}
