//! Synthetic distribution contracts; no keys, state, sockets or vendor snapshots.
use super::routing::{CompiledRules, EffectiveBindings, SendToolsRule};
use super::*;
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({"version":1,
        "sources":{"alias":{"prefix":"vendor","base_url":"https://example.com",
            "send_tools":[{"to":"one"}]}},
        "wallets":{"payer":{"mode":"static","private_key_env":"ABSENT_ROUTING_KEY",
            "max_api_payment_usdc":"1"}},
        "servers":{
            "one":{"listen":"127.0.0.1:0","auth":false,"wallet":"payer","discover_on_demand":true},
            "two":{"listen":"127.0.0.1:0","auth":false,"wallet":"payer","discover_on_demand":true}}})
}
fn config(value: &Value) -> MetaConfig {
    serde_json::from_value(value.clone()).unwrap()
}

#[test]
fn research_desk_separates_health_and_preserves_registry_and_court_workflows() {
    let config: MetaConfig = toml::from_str(include_str!(
        "../../examples/deployments/research_desk.toml"
    ))
    .unwrap();
    EffectiveBindings::resolve(&config).unwrap();
    for (source, suffix, target) in [
        ("twos", "papers_search", Some("science")),
        ("twos", "bio_gene", Some("science")),
        ("twos", "clinical_trials", Some("health")),
        ("twos", "health_provider_profile", Some("health")),
        ("twos", "medical_npi", Some("health")),
        ("twos", "nutrition_food_search", Some("health")),
        ("twos", "gov_fda_drugs", Some("health")),
        ("twos", "law_case_search", None),
        ("twos", "law_docket_search", Some("business")),
        ("twos", "law_opinion", Some("business")),
        ("twos", "business_sos_search", Some("business")),
        ("strale", "swedish_company_data", Some("business")),
        ("maxwell", "courts_search", None),
        ("maxwell", "gov_pl_companies", Some("business")),
        ("maxwell", "openfda_drug_label", Some("health")),
        ("x402stock", "clinical_trials", Some("health")),
        ("findmydirectdoctor", "doctors_search", Some("health")),
        ("findmydirectdoctor", "help", Some("health")),
        (
            "healthtravel",
            "health_ema_shortages_safety",
            Some("health"),
        ),
        ("companyrecords", "gov_justia_case_law", Some("business")),
        ("companyrecords", "b2b_ny_business_entity", Some("business")),
        (
            "companyrecords",
            "b2b_bolagsverket_sweden_companies",
            Some("business"),
        ),
        ("companyrecords", "gov_krs_poland", Some("business")),
        ("companyrecords", "gov_healthcare_gov", Some("health")),
        (
            "companyrecords",
            "b2b_webmd_healthcare_provider_directory",
            Some("health"),
        ),
        ("companyrecords", "b2b_ada_find_a_dentist", None),
    ] {
        let rules = config.sources[source].send_tools.as_ref().unwrap();
        let compiled = CompiledRules::new(rules, "test").unwrap();
        let name = format!("test_{suffix}");
        let decision = compiled.decision(&name);
        let expected: Vec<_> = target.into_iter().collect();
        assert_eq!(decision.targets, expected, "{source}/{suffix}");
        assert_eq!(decision.excluded, target.is_none(), "{source}/{suffix}");
    }
}
fn rules(value: Value) -> Vec<SendToolsRule> {
    serde_json::from_value(value).unwrap()
}
fn source(send_tools: Vec<SendToolsRule>, settings: &str) -> Result<Source> {
    let cfg = toml::from_str(&format!(
        "prefix='vendor'\nbase_url='https://example.com'\n{settings}"
    ))?;
    Source::from_document(
        "alias",
        cfg,
        std::sync::Arc::new(json!({"operations":[
            {"method":"GET","path":"/read"},
            {"method":"GET","path":"/hidden"},
            {"method":"GET","path":"/family/new"}
        ]})),
    )?
    .with_send_tools("alias", Some(send_tools))
}

#[test]
fn malformed_rules_reject_ambiguous_or_incomplete_distribution() {
    for send_tools in [
        json!([]),
        json!([{"to":[]}]),
        json!([{"to":["one","one"]}]),
        json!([{"to":["one","exclude"]}]),
        json!([{"to":"missing"}]),
        json!([{"to":"one","tools":[]}]),
        json!([{"to":"one","tools":["read"]}]),
        json!([{"to":"one"},{"to":"two"}]),
        json!([{"to":"one","tools":[""]},{"to":"two"}]),
    ] {
        let mut value = fixture();
        value["sources"]["alias"]["send_tools"] = send_tools.clone();
        assert!(
            EffectiveBindings::resolve(&config(&value)).is_err(),
            "{send_tools}"
        );
    }
    assert!(serde_json::from_value::<SendToolsRule>(json!({"to":"one","typo":true})).is_err());
    // Deployment syntax must not escape into reusable provider configuration.
    assert!(toml::from_str::<Config>("send_tools = [{to = 'one'}]").is_err());
}

#[tokio::test]
async fn obsolete_send_field_is_rejected_with_or_without_send_tools() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    for keep_send_tools in [false, true] {
        let mut value = fixture();
        if !keep_send_tools {
            value["sources"]["alias"]
                .as_object_mut()
                .unwrap()
                .remove("send_tools");
        }
        value["sources"]["alias"]["send"] = json!([{"to":"one"}]);
        std::fs::write(&path, toml::to_string(&config(&value)).unwrap()).unwrap();
        let error = Deployment::show_config(&path).await.err().unwrap();
        assert!(
            format!("{error:#}").contains("unknown field `send`"),
            "{error:#}"
        );
    }
}

#[test]
fn exclude_reservation_and_legacy_attachment_bypass_are_conditional() {
    let mut value = fixture();
    value["servers"]["exclude"] = value["servers"]["two"].clone();
    assert!(EffectiveBindings::resolve(&config(&value)).is_err());
    value["sources"]["alias"]
        .as_object_mut()
        .unwrap()
        .remove("send_tools");
    value["servers"]["one"]["sources"] = json!(["alias"]);
    let legacy = config(&value);
    assert!(EffectiveBindings::resolve(&legacy).is_ok());
    value["sources"]["alias"]["send_tools"] = json!([{"to":"two"}]);
    value["servers"].as_object_mut().unwrap().remove("exclude");
    let error = EffectiveBindings::resolve(&config(&value)).err().unwrap();
    assert!(error.to_string().contains("cannot also appear"));
}

#[test]
fn potential_graph_retains_shadowed_and_future_destinations_with_origins() {
    let mut value = fixture();
    value["sources"]["alias"]["send_tools"] = json!([
        {"to":["one","two"],"tools":["*"]},
        {"to":"two","tools":["future_*"]},
        {"to":"exclude"}
    ]);
    value["sources"]["legacy"] = json!({});
    value["servers"]["one"]["sources"] = json!(["legacy"]);
    value["sources"]["excluded"] = json!({"send_tools":[{"to":"exclude"}]});
    let authored = config(&value);
    let graph = EffectiveBindings::resolve(&authored).unwrap();
    assert_eq!(
        graph.0["two"]["alias"],
        [
            "sources.alias.send_tools[0].to",
            "sources.alias.send_tools[1].to"
        ]
    );
    assert_eq!(graph.0["one"]["legacy"], ["servers.one.sources[0]"]);
    assert!(
        graph
            .0
            .values()
            .all(|edges| !edges.contains_key("excluded"))
    );
    let effective = graph.listeners(&authored);
    assert_eq!(effective["one"].sources, ["legacy", "alias"]);
    assert_eq!(authored.servers["one"].sources, ["legacy"]);
    assert!(authored.servers["two"].sources.is_empty());
}

#[test]
fn matching_strips_only_resolved_prefix_once_and_keeps_case_and_suffixes() {
    let send_tools = rules(json!([
        {"to":"exclude","tools":["help"]},
        {"to":"one","tools":["locus_*","read_?et","item_*_2"]},
        {"to":"two"}
    ]));
    let compiled = CompiledRules::new(&send_tools, "locus").unwrap();
    for (name, suffix, winner) in [
        ("locus_help", "help", 0),
        ("locus_locus_zoning", "locus_zoning", 1),
        ("locus_read_get", "read_get", 1),
        ("locus_item_name_2", "item_name_2", 1),
        ("locus_READ_get", "READ_get", 2),
        ("alias_help", "alias_help", 2),
        ("locus_future", "future", 2),
    ] {
        let decision = compiled.decision(name);
        assert_eq!((decision.suffix, decision.rule_index), (suffix, winner));
    }
}

#[test]
fn first_match_consumes_tools_but_exact_selector_validation_precedes_consumption() {
    let send_tools = rules(json!([
        {"to":"exclude","tools":["hidden"]},
        {"to":["one","two"],"tools":["read"]},
        {"to":"two","tools":["hidden"]},
        {"to":"one"}
    ]));
    let loaded = source(send_tools, "").unwrap();
    let compiled = CompiledRules::new(loaded.send_tools.as_ref().unwrap(), "vendor").unwrap();
    let hidden = compiled.decision("vendor_hidden");
    assert!(hidden.excluded && hidden.targets.is_empty());
    assert_eq!(hidden.rule_index, 0);
    assert_eq!(compiled.decision("vendor_read").targets, ["one", "two"]);
    assert_eq!(compiled.decision("vendor_family_new").rule_index, 3);
    // A wildcard with no matches remains advisory.
    assert!(
        source(
            rules(json!([
                {"to":"two","tools":["future_*"]},{"to":"one"}
            ])),
            ""
        )
        .is_ok()
    );
}

#[test]
fn source_filters_precede_distribution_even_for_exclusion_only_sources() {
    let send_tools = rules(json!([
        {"to":"exclude","tools":["hidden"]},{"to":"exclude"}
    ]));
    assert!(source(send_tools.clone(), "").is_ok());
    let error = source(send_tools, "exclude_tools=['vendor_hidden']")
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("unknown tool selector hidden"));
}

#[test]
fn invalid_distribution_selectors_preserve_details_with_safe_selection_diagnostics() {
    let error = source(
        rules(json!([
            {"to":"two","tools":["private_selector"]},{"to":"one"}
        ])),
        "",
    )
    .err()
    .unwrap();
    assert!(
        format!("{error:#}")
            .contains("source alias: send_tools[0] unknown tool selector private_selector")
    );
    let failure = startup::catalog_failure(&error, "fetch_parse");
    assert_eq!(failure.stage, "selection");
    assert_eq!(
        failure.reason,
        "Source distribution tool selectors are invalid; check the source send_tools rules against its available tools"
    );
    assert!(!failure.reason.contains("private_selector"));
    assert_eq!(failure.http_status, None);
}

#[test]
fn routing_and_final_listener_filters_share_selected_inventory() {
    let mut value = fixture();
    value["sources"]["alias"]["send_tools"] = json!([
        {"to":"exclude","tools":["hidden"]},
        {"to":["one","two"],"tools":["read"]},
        {"to":"two"}
    ]);
    value["servers"]["two"]["exclude_tools"] = json!(["vendor_read"]);
    let authored = config(&value);
    let listeners = EffectiveBindings::resolve(&authored)
        .unwrap()
        .listeners(&authored);
    let sources = BTreeMap::from([(
        "alias".into(),
        source(authored.sources["alias"].send_tools.clone().unwrap(), "").unwrap(),
    )]);
    let selected = |name: &str| {
        listener_tool_inventory(name, &listeners[name], &sources, &BTreeMap::new(), false)
            .unwrap()
            .tools
            .into_iter()
            .map(|(_, t)| t.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(selected("one"), ["vendor_read"]);
    assert_eq!(selected("two"), ["vendor_family_new"]);
}

fn managed() -> Value {
    json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},
        "max_api_payment_usdc":"1","max_retirement_remainder_usdc":"0",
        "max_conversion_overhead_percent":5,"max_attempts":3})
}

#[test]
fn observation_retains_known_groups_despite_unrelated_invalid_bindings() {
    let mut value = fixture();
    value["wallets"]["explicit"] = managed();
    value["wallet_templates"] = json!({"template":managed()});
    value["wallet_assignment"] = json!({"scope":"binding","template":"template"});
    value["servers"]["one"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    value["sources"]["broken"] = json!({"send_tools":[{"to":"unknown"}]});
    let cfg = config(&value);
    assert!(EffectiveBindings::resolve(&cfg).is_err());
    assert_eq!(
        assignment::managed_profiles(&cfg)
            .into_keys()
            .collect::<Vec<_>>(),
        ["auto_v1_binding_3_one_alias", "explicit"]
    );
}

#[test]
fn automatic_identities_depend_on_bindings_not_rule_order() {
    for (scope, expected) in [
        ("deployment", vec!["auto_v1_deployment"]),
        ("server", vec!["auto_v1_server_one", "auto_v1_server_two"]),
        ("source", vec!["auto_v1_source_alias"]),
        (
            "binding",
            vec!["auto_v1_binding_3_one_alias", "auto_v1_binding_3_two_alias"],
        ),
    ] {
        let mut value = fixture();
        for name in ["one", "two"] {
            value["servers"][name]
                .as_object_mut()
                .unwrap()
                .remove("wallet");
        }
        value["wallet_templates"] = json!({"template":managed()});
        value["wallet_assignment"] = json!({"scope":scope,"template":"template"});
        for send_tools in [
            json!([{"to":"one","tools":["read"]},{"to":"two"}]),
            json!([{"to":"two","tools":["hidden"]},{"to":"one"}]),
        ] {
            value["sources"]["alias"]["send_tools"] = send_tools;
            let resolved = assignment::resolve(&config(&value)).unwrap();
            assert_eq!(
                resolved
                    .generated
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn fanout_preserves_source_then_listener_wallet_precedence_and_exclusion_has_no_demand() {
    let mut value = fixture();
    value["wallets"]["other"] = value["wallets"]["payer"].clone();
    value["servers"]["two"]["wallet"] = json!("other");
    value["sources"]["alias"]["send_tools"] = json!([{"to":["one","two"]}]);
    let resolved = assignment::resolve(&config(&value)).unwrap();
    assert_eq!(resolved.bindings["one"]["alias"].wallet, "payer");
    assert_eq!(resolved.bindings["two"]["alias"].wallet, "other");
    value["sources"]["alias"]["wallet"] = json!("payer");
    let resolved = assignment::resolve(&config(&value)).unwrap();
    for listener in ["one", "two"] {
        assert_eq!(resolved.bindings[listener]["alias"].wallet, "payer");
        assert_eq!(
            resolved.bindings[listener]["alias"].origin,
            "sources.alias.wallet"
        );
    }
    value["sources"]["alias"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    value["sources"]["alias"]["send_tools"] = json!([{"to":"exclude"}]);
    for listener in ["one", "two"] {
        value["servers"][listener]
            .as_object_mut()
            .unwrap()
            .remove("wallet");
    }
    let resolved = assignment::resolve(&config(&value)).unwrap();
    assert!(resolved.bindings.values().all(BTreeMap::is_empty));
    assert!(resolved.generated.is_empty());
}

#[tokio::test]
async fn offline_show_explanation_and_authored_snapshot_agree_with_publication() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        serde_json::to_vec(&json!({"paths":{
            "/read":{"get":{}},"/hidden":{"get":{}},"/future":{"post":{}}
        }}))
        .unwrap(),
    )
    .unwrap();
    let mut value = fixture();
    value["sources"]["alias"]["spec"] = json!("spec.json");
    value["sources"]["alias"]["send_tools"] = json!([
        {"to":"exclude","tools":["hidden"]},
        {"to":["one","two"],"tools":["read"]},{"to":"two"}
    ]);
    value["servers"]["two"]["exclude_tools"] = json!(["vendor_read"]);
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, toml::to_string(&config(&value)).unwrap()).unwrap();
    let shown = Deployment::show_config(&path).await.unwrap();
    assert!(shown["sources"]["alias"].get("send").is_none());
    assert_eq!(
        shown["sources"]["alias"]["send_tools"],
        value["sources"]["alias"]["send_tools"]
    );
    assert_eq!(shown["servers"]["one"]["sources"], json!([]));
    let deployment = Deployment::load(&path).await.unwrap();
    let explained = deployment.explain_routing().unwrap();
    let decision = |name: &str| {
        explained["alias"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()
    };
    assert_eq!(decision("vendor_hidden")["action"], "exclude");
    assert_eq!(decision("vendor_hidden")["effective_targets"], json!([]));
    assert_eq!(
        decision("vendor_read")["routed_targets"],
        json!(["one", "two"])
    );
    assert_eq!(decision("vendor_read")["effective_targets"], json!(["one"]));
    assert_eq!(
        decision("vendor_read")["wallet_bindings"]["one"]["wallet"],
        "payer"
    );
    assert_eq!(decision("vendor_future")["catch_all"], true);
    let snapshot = deployment.qualification_snapshot();
    assert!(
        snapshot["deployment"]["sources"]["alias"]
            .get("send")
            .is_none()
    );
    assert_eq!(
        snapshot["deployment"]["sources"]["alias"]["send_tools"],
        value["sources"]["alias"]["send_tools"]
    );
    let restored: MetaConfig = serde_json::from_value(snapshot["deployment"].clone()).unwrap();
    assert!(
        restored
            .servers
            .values()
            .all(|listener| listener.sources.is_empty())
    );
    let restored_path = dir.path().join("restored.toml");
    std::fs::write(&restored_path, toml::to_string(&restored).unwrap()).unwrap();
    let reloaded = Deployment::load(&restored_path).await.unwrap();
    assert_eq!(
        serde_json::to_value(reloaded.inventory()).unwrap(),
        snapshot["inventory"]
    );
    assert_eq!(reloaded.explain_routing().unwrap(), explained);
    // Old snapshots remain legacy attachments, with suffixes still explained.
    value["sources"]["alias"]
        .as_object_mut()
        .unwrap()
        .remove("send_tools");
    value["servers"]["one"]["sources"] = json!(["alias"]);
    value["servers"]["two"]["sources"] = json!(["alias"]);
    std::fs::write(&restored_path, toml::to_string(&config(&value)).unwrap()).unwrap();
    let legacy = Deployment::load(&restored_path).await.unwrap();
    let legacy_explanation = legacy.explain_routing().unwrap();
    assert!(
        legacy_explanation["alias"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["suffix"]
                .as_str()
                .is_some_and(|suffix| !suffix.starts_with("vendor_"))
                && tool["rule_index"].is_null())
    );
}

#[tokio::test]
async fn serving_selection_distinguishes_endpoint_and_source_failure_reasons() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"paths":{"/read":{"get":{"description":"private upstream prose"}}}}"#,
    )
    .unwrap();
    let mut value = fixture();
    value["sources"]["alias"]["spec"] = json!("spec.json");
    value["sources"]["other"] =
        json!({"spec":"spec.json","base_url":"https://example.com","send_tools":[{"to":"two"}]});
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, toml::to_string(&config(&value)).unwrap()).unwrap();
    let mut reserved = value.clone();
    reserved["sources"]["alias"]["prefix"] = json!("dyn");
    let strict_path = dir.path().join("reserved.toml");
    std::fs::write(&strict_path, toml::to_string(&config(&reserved)).unwrap()).unwrap();
    let strict_error = Deployment::load(&strict_path).await.err().unwrap();
    assert!(format!("{strict_error:#}").contains("static tool uses reserved namespace"));
    let payer = PaidClient::new(
        Payer::new(&format!("{:064x}", 2), SpendPolicy::dollars("1").unwrap()).unwrap(),
    );
    let wallets = BTreeMap::from([("payer".into(), payer)]);
    for (case, expected) in [
        (
            "selector",
            "A listener tool selector matches no assigned tools",
        ),
        (
            "empty",
            "The endpoint has no selected tools or discovery capabilities",
        ),
        (
            "reserved",
            "Static tools use a reserved management namespace",
        ),
    ] {
        let mut deployment = Deployment::load(&path).await.unwrap();
        match case {
            "selector" => {
                deployment.listeners.get_mut("one").unwrap().include_tools =
                    vec!["vendor_private_typo".into()]
            }
            "empty" => {
                let listener = deployment.listeners.get_mut("one").unwrap();
                listener.discover_on_demand = false;
                listener.include_tools = vec!["vendor_missing*".into()];
            }
            "reserved" => {
                let source = deployment.sources.get_mut("alias").unwrap();
                let mut cfg = source.config.clone();
                cfg.prefix = Some("dyn".into());
                *source = Source::from_document("alias", cfg, source.document.clone())
                    .unwrap()
                    .with_send_tools("alias", source.send_tools.clone())
                    .unwrap();
            }
            _ => unreachable!(),
        }
        if case == "reserved" {
            let error = deployment.refresh_selection(true).unwrap_err();
            assert!(
                startup::catalog_failure(&error, "selection")
                    .reason
                    .starts_with(expected)
            );
            assert!(deployment.selection_errors.is_empty());
            assert_eq!(deployment.selected["one"][0].1.name, "vendor_read");
            assert_eq!(deployment.selected["two"][0].0, "other");
            continue;
        }
        deployment.refresh_selection(true).unwrap();
        assert!(deployment.selection_errors["one"].starts_with(expected));
        assert!(!deployment.selection_errors.contains_key("two"));
        let servers = deployment.build_servers(&wallets);
        let instructions = servers["one"].instructions.as_deref().unwrap();
        assert!(instructions.contains(expected), "{instructions}");
        assert!(
            !instructions.contains("vendor_private_typo")
                && !instructions.contains("private upstream prose"),
            "{instructions}"
        );
        assert_eq!(servers["one"].catalog.read().views["default"].len(), 0);
        assert_eq!(servers["two"].catalog.read().views["default"].len(), 1);
    }
}

#[tokio::test]
async fn serving_selection_preserves_cross_listener_cover_ownership() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.json"),
        r#"{"paths":{"/read":{"get":{}}}}"#,
    )
    .unwrap();
    let mut value = fixture();
    value["sources"]["alias"]["spec"] = json!("spec.json");
    value["sources"]["other"] =
        json!({"spec":"spec.json","base_url":"https://example.com","send_tools":[{"to":"two"}]});
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, toml::to_string(&config(&value)).unwrap()).unwrap();
    let mut deployment = Deployment::load(&path).await.unwrap();
    let table: toml::Table = toml::from_str(
        include_str!("../../docs/cover-traffic.md")
            .split("```toml\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap(),
    )
    .unwrap();
    let mut cover: crate::cover::Config = table["cover_traffic"].clone().try_into().unwrap();
    cover.url = "https://example.com/cover".into();
    for source in deployment.sources.values_mut() {
        source.config.cover_traffic = Some(cover.clone());
    }
    // The same owner key spans two listeners. Per-listener validation would
    // incorrectly accept these conflicting profiles independently.
    deployment
        .sources
        .get_mut("other")
        .unwrap()
        .config
        .cover_traffic
        .as_mut()
        .unwrap()
        .concurrency = 1;
    let error = deployment.refresh_selection(true).unwrap_err();
    assert!(format!("{error:#}").contains("conflicting cover settings for shared wallet/origin"));
    assert!(deployment.selection_errors.is_empty());
    assert_eq!(deployment.selected["one"].len(), 1);
    assert_eq!(deployment.selected["two"].len(), 1);
    deployment
        .sources
        .get_mut("other")
        .unwrap()
        .config
        .cover_traffic = Some(cover);
    deployment.refresh_selection(true).unwrap();
    assert_eq!(deployment.selected["one"][0].0, "alias");
    assert_eq!(deployment.selected["two"][0].0, "other");
}

#[test]
fn exclusion_only_sources_never_demand_a_metadata_relay_payer() {
    let example: MetaConfig = toml::from_str(include_str!(
        "../../examples/deployments/comprehensive.toml"
    ))
    .unwrap();
    let mut policy = example.relay_fallback.unwrap();
    policy.sources.clear();
    policy.wallet = None;
    let mut value = fixture();
    value["sources"]["alias"]["send_tools"] = json!([{"to":"exclude"}]);
    for explicit_source in [false, true] {
        if explicit_source {
            value["sources"]["alias"]["wallet"] = json!("payer");
        }
        for override_wallet in [None, Some("payer".to_owned())] {
            policy.wallet = override_wallet;
            let cfg = config(&value);
            let resolution = assignment::resolve(&cfg).unwrap();
            assert!(
                crate::discovery_relay::resolve_wallets(
                    &policy,
                    &cfg,
                    &resolution,
                    ["alias".to_owned()]
                )
                .unwrap()
                .is_empty()
            );
        }
    }
    value["sources"]["alias"]
        .as_object_mut()
        .unwrap()
        .remove("send_tools");
    value["sources"]["alias"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    policy.wallet = None;
    let cfg = config(&value);
    let resolution = assignment::resolve(&cfg).unwrap();
    assert!(
        crate::discovery_relay::resolve_wallets(&policy, &cfg, &resolution, ["alias".to_owned()])
            .is_err()
    );
}

#[test]
fn routing_precedes_same_prefix_collision_detection() {
    let left = source(rules(json!([{"to":"one"}])), "").unwrap();
    let right = source(rules(json!([{"to":"two"}])), "").unwrap();
    let sources = BTreeMap::from([("left".to_owned(), left), ("right".to_owned(), right)]);
    let mut value = fixture();
    value["sources"] = json!({
        "left":{"send_tools":[{"to":"one"}]},
        "right":{"send_tools":[{"to":"two"}]}
    });
    let config = config(&value);
    let listeners = EffectiveBindings::resolve(&config)
        .unwrap()
        .listeners(&config);
    for name in ["one", "two"] {
        let selected =
            listener_tool_inventory(name, &listeners[name], &sources, &BTreeMap::new(), false)
                .unwrap();
        assert_eq!(selected.tools.len(), 3);
        assert!(selected.conflicts.is_empty());
    }
    let mut overlapping = sources;
    overlapping.get_mut("right").unwrap().send_tools = Some(rules(json!([{"to":["one","two"]}])));
    let mut listener = listeners["one"].clone();
    listener.sources.push("right".to_owned());
    assert!(
        listener_tool_inventory("one", &listener, &overlapping, &BTreeMap::new(), false).is_err()
    );
    let serving =
        listener_tool_inventory("one", &listener, &overlapping, &BTreeMap::new(), true).unwrap();
    assert!(serving.tools.is_empty());
    assert_eq!(serving.conflicts.len(), 3);
}
