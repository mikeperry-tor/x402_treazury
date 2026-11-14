use super::*;
use serde_json::json;

fn source(prefix: &str, remote: bool, includes: &[&str], advice: &[&str], opt_in: bool) -> Source {
    Source::from_document(
        "api",
        Config {
            spec: if remote {
                "https://catalog.example/spec"
            } else {
                "catalog.json"
            }
            .into(),
            prefix: Some(prefix.into()),
            base_url: Some("https://api.example".into()),
            allow_empty_selection: true,
            include_tools: includes.iter().map(|s| (*s).into()).collect(),
            recommended_tools: advice.iter().map(|s| (*s).into()).collect(),
            use_recommended_tools: opt_in,
            ..Default::default()
        },
        std::sync::Arc::new(json!({"paths":{
            "/read":{"get":{}}, "/other":{"get":{}}
        }})),
    )
    .unwrap()
}
fn listener(includes: &[&str], sources: &[&str]) -> ListenerConfig {
    serde_json::from_value(json!({"listen":"127.0.0.1:0", "auth":false,
        "wallet":"unused", "sources":sources, "include_tools":includes}))
    .unwrap()
}

#[test]
fn source_inclusions_and_advice_never_widen_selection() {
    // Document location must not change ordinary selection behavior.
    for remote in [false, true] {
        for (includes, advice, opt_in, expected) in [
            (vec!["api_missing"], vec![], false, vec![]),
            (
                vec!["api_missing", "api_read"],
                vec![],
                false,
                vec!["api_read"],
            ),
            (vec![], vec!["api_missing"], true, vec![]),
            (vec![], vec![], true, vec![]),
            (
                vec![],
                vec!["api_missing", "api_read"],
                true,
                vec!["api_read"],
            ),
            (
                vec![],
                vec!["api_missing"],
                false,
                vec!["api_other", "api_read"],
            ),
        ] {
            let loaded = source("api", remote, &includes, &advice, opt_in);
            assert_eq!(
                loaded
                    .tools
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            if !includes.is_empty() || opt_in {
                assert!(!loaded.config.selection_notices.is_empty());
                let mut strict = loaded.config;
                strict.allow_empty_selection = false;
                assert!(catalog::build_catalog(&strict, &loaded.document, "api").is_err());
            }
        }
    }
}

#[test]
fn empty_source_filters_remain_narrow_and_report_reduced_coverage() {
    let loaded = source("api", false, &[], &[], false);
    for (includes, excludes) in [
        (vec!["api_read"], vec!["api_read"]),
        (vec!["api_*"], vec!["api_*"]),
        (vec![], vec!["api_read", "api_other"]),
    ] {
        let mut cfg = loaded.config.clone();
        cfg.include_tools = includes.into_iter().map(str::to_owned).collect();
        cfg.exclude_tools = excludes.into_iter().map(str::to_owned).collect();
        let built = catalog::build_catalog(&cfg, &loaded.document, "api").unwrap();
        assert!(built.tools.is_empty());
        assert!(!built.selection_notices.is_empty());
        cfg.allow_empty_selection = false;
        assert!(catalog::build_catalog(&cfg, &loaded.document, "api").is_err());
    }
}

#[test]
fn empty_endpoint_filters_do_not_infer_namespace_ownership() {
    let sources = BTreeMap::from([
        ("a".into(), source("api", true, &[], &[], false)),
        ("b".into(), source("api_nested", false, &[], &[], false)),
    ]);
    for selector in [
        "api_missing",
        "api_nested_missing",
        "unknown_missing",
        "unknown_*",
        "*",
    ] {
        let mut cfg = listener(&[selector], &["a", "b"]);
        cfg.exclude_tools = vec!["*".into()];
        let ordinary =
            listener_tool_inventory("one", &cfg, &sources, &BTreeMap::new(), true).unwrap();
        assert!(ordinary.tools.is_empty());
        assert!(listener_tool_inventory("one", &cfg, &sources, &BTreeMap::new(), false).is_err());
    }
    let cfg = listener(&["unknown_missing", "api_read"], &["a", "b"]);
    let inventory = listener_tool_inventory("one", &cfg, &sources, &BTreeMap::new(), true).unwrap();
    assert_eq!(inventory.tools.len(), 1);
    assert_eq!(inventory.tools[0].1.name, "api_read");
}

#[test]
fn missing_distribution_names_preserve_rule_order_and_destination_isolation() {
    let rules = serde_json::from_value(json!([
        {"to":"exclude","tools":["other"]},
        {"to":"one","tools":["missing", "read"]},
        {"to":"two","tools":["*"]},
        {"to":"exclude"}
    ]))
    .unwrap();
    let loaded = source("api", false, &[], &[], false)
        .with_send_tools("api", Some(rules))
        .unwrap();
    assert_eq!(loaded.config.selection_notices[0].setting, "send_tools");
    let sources = BTreeMap::from([("api".into(), loaded)]);
    let cfg = listener(&[], &["api"]);
    let one = listener_tool_inventory("one", &cfg, &sources, &BTreeMap::new(), true).unwrap();
    assert_eq!(
        one.tools
            .iter()
            .map(|(_, t)| t.name.as_str())
            .collect::<Vec<_>>(),
        ["api_read"]
    );
    let two = listener_tool_inventory("two", &cfg, &sources, &BTreeMap::new(), true).unwrap();
    assert!(
        two.tools.is_empty(),
        "later rules cannot restore excluded or assigned tools"
    );
    let mut strict = sources["api"].clone();
    strict.config.allow_empty_selection = false;
    assert!(
        strict
            .clone()
            .with_send_tools("api", strict.send_tools)
            .is_err()
    );
}

#[tokio::test]
async fn accepted_empty_mixed_local_source_keeps_required_endpoint_ready() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("catalog.json"),
        json!({"paths":{"/read":{"get":{}}, "/cookie":{"get":{"parameters":[{"in":"cookie","name":"session"}]}}}}).to_string(),
    )
    .unwrap();
    let path = dir.path().join("deployment.toml");
    for settings in [
        "include_tools=['unknown_name']",
        "exclude_tools=['api_read']",
        "use_recommended_tools=true",
        "tags=['removed']",
    ] {
        std::fs::write(&path, format!("version=1\n[treasury]\nstate_dir='.'\n[wallets.w]\nmode='static'\nprivate_key_env='UNREAD'\n[sources.api]\nspec='catalog.json'\nbase_url='https://api.example'\n{settings}\n[servers.one]\nlisten='127.0.0.1:0'\nauth=false\nwallet='w'\nsources=['api']\ninstructions_text='Custom guidance'\n")).unwrap();
        let deployment = Deployment::load_for_unsigned_serving(&path).await.unwrap();
        assert!(deployment.sources["api"].tools.is_empty());
        assert!(!deployment.unavailable_sources.contains_key("api"));
        if !settings.starts_with("tags") {
            assert_eq!(deployment.sources["api"].issues.len(), 1);
        }
        assert!(deployment.endpoint_readiness("one").unwrap().is_ok());
        assert!(!deployment.config.sources["api"].allow_catalog_failure);
        assert!(
            !deployment.sources["api"]
                .config
                .selection_notices
                .is_empty()
        );
        let servers =
            deployment.build_servers(&BTreeMap::from([("w".into(), PaidClient::unsigned())]));
        let instructions = servers["one"].instructions.as_deref().unwrap();
        assert!(instructions.contains("Custom guidance"));
        assert!(instructions.contains("No static API tools are selected"));
        let warmed = cache_warm::warm_cache(&path, &[], false).await.unwrap();
        assert_eq!(warmed.outcome, "degraded");
        assert_eq!(warmed.sources[0].status, "unavailable");
        assert!(!warmed.sources[0].selection_notices.is_empty());
        if !settings.starts_with("tags") {
            assert_eq!(warmed.sources[0].omissions.len(), 1);
        }
        assert!(Deployment::load(&path).await.is_err());
    }
}

#[tokio::test]
async fn catalog_failure_setting_is_explicit_and_old_key_is_rejected() {
    let source: SourceConfig =
        toml::from_str("spec='https://api.example/spec'\nallow_catalog_failure=true").unwrap();
    assert!(source.allow_catalog_failure);
    let encoded = serde_json::to_value(&source).unwrap();
    assert_eq!(encoded["allow_catalog_failure"], true);
    assert!(encoded.get("optional_startup").is_none());
    let old: SourceConfig =
        toml::from_str("spec='https://api.example/spec'\noptional_startup=true").unwrap();
    let error = crate::config::resolve(old.provider, Path::new("deployment.toml"))
        .await
        .err()
        .expect("obsolete field must not be silently ignored");
    assert!(format!("{error:#}").contains("optional_startup"));
    let default: SourceConfig = toml::from_str("spec='https://api.example/spec'").unwrap();
    assert!(!default.allow_catalog_failure);
}
