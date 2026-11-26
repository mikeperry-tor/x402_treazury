//! Report listener collisions on publication and filter gaps once sources settle.
use super::*;

#[derive(Debug)]
pub(super) struct Selection {
    pub(super) unmatched: Vec<UnmatchedSelector>,
    pub(super) empty: bool,
}

#[derive(Debug)]
pub(super) struct UnmatchedSelector {
    pub(super) setting: &'static str,
    pub(super) selector: String,
    pub(super) pattern: Regex,
}

pub(super) fn report_completed(
    listeners: &BTreeMap<String, ListenerConfig>,
    sources: &BTreeMap<String, Source>,
    pending: &BTreeMap<String, startup::PendingSource>,
    selections: &BTreeMap<String, Selection>,
    conflicts: &BTreeMap<String, Vec<String>>,
    previous_conflicts: &BTreeMap<String, Vec<String>>,
    completed: Option<&str>,
) {
    for (name, selection) in selections {
        let server = &listeners[name];
        if completed.is_some_and(|id| !server.sources.iter().any(|source| source == id)) {
            continue;
        }
        let settled = !server.sources.iter().any(|id| {
            pending
                .get(id)
                .is_some_and(|entry| !entry.is_relay_blocked())
        });
        if (settled || !server.wait_for_catalogs)
            && let Some(names) = conflicts.get(name).filter(|names| !names.is_empty())
            && previous_conflicts.get(name) != Some(names)
        {
            tracing::warn!(
                server = name,
                category = "ambiguous_tool_names",
                tool_names = ?names,
                "Listener {name:?} omitted these tools because multiple configured sources define them: {}. \
                 Configure distinct prefixes or source selections and restart.",
                names.join(", ")
            );
        }
        if !settled {
            continue;
        }
        for warning in warnings(name, server, sources, selection) {
            match warning.setting {
                "exclude_tools" => tracing::info!(
                    server = name,
                    setting = warning.setting,
                    selector = warning.selector,
                    "{}",
                    warning.explanation
                ),
                _ => tracing::warn!(
                    server = name,
                    setting = warning.setting,
                    selector = warning.selector,
                    "{}",
                    warning.explanation
                ),
            }
        }
    }
}

#[derive(Debug)]
struct Warning {
    setting: &'static str,
    selector: String,
    explanation: String,
}

fn warnings(
    name: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
    selection: &Selection,
) -> Vec<Warning> {
    let unavailable: Vec<_> = server
        .sources
        .iter()
        .filter(|id| !sources.contains_key(*id))
        .collect();
    // Optional removal evidence must not suppress known selection failures when
    // a partially compatible catalog cannot reconstruct its complete inventory.
    // One temporary witness per filter explains the cause when available.
    let removals = if selection.unmatched.is_empty() {
        BTreeMap::new()
    } else {
        selection_removals(name, server, sources).unwrap_or_default()
    };
    let mut warnings: Vec<_> = selection.unmatched.iter().map(|miss| {
        let causes = removal_causes(&removals, &miss.pattern);
        let subject = if miss.setting == "exclude_tools" {
            format!("Listener {name:?} exclusion {:?} matches no available tools", miss.selector)
        } else {
            format!("Listener {name:?} inclusion {:?} matches no available tools", miss.selector)
        };
        let explanation = if causes.is_empty() {
            format!("{subject}. Check the selector, source availability, and filters. Unavailable sources: {unavailable:?}")
        } else {
            format!("{subject}. Earlier filters removed matching tools. {}", causes.join(". "))
        };
        Warning { setting: miss.setting, selector: miss.selector.clone(), explanation }
    }).collect();
    if !server.sources.is_empty() && selection.empty {
        warnings.push(Warning {
            setting: "tool_filters",
            selector: "*".into(),
            explanation: format!("No static API tools are selected for listener {name:?}. Check source availability, routing, tags, tool filters, and duplicate tool names"),
        });
    }
    warnings
}

pub(super) fn unknown_selector(
    name: &str,
    server: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
    selector: &str,
    pattern: &Regex,
) -> anyhow::Error {
    let removals = match selection_removals(name, server, sources) {
        Ok(removals) => removals,
        Err(error) => return error,
    };
    let causes = removal_causes(&removals, pattern);
    let detail = if causes.is_empty() {
        String::new()
    } else {
        format!(
            "; matching tools were removed before listener name filtering: {}",
            causes.join("; ")
        )
    };
    anyhow::anyhow!("server {name}: unknown tool selector {selector}{detail}")
        .context(catalog::diagnostics::Problem("A listener tool selector matches no assigned tools; inspect source send_tools rules and listener filters"))
}

fn removal_causes(removals: &BTreeMap<String, Vec<String>>, pattern: &Regex) -> Vec<String> {
    removals
        .iter()
        .filter_map(|(reason, names)| {
            names
                .iter()
                .find(|name| pattern.is_match(name))
                .map(|name| format!("{reason} (removes {name})"))
        })
        .collect()
}

fn selection_removals(
    name: &str,
    listener: &ListenerConfig,
    sources: &BTreeMap<String, Source>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut removals = BTreeMap::new();
    for id in &listener.sources {
        let Some(source) = sources.get(id) else {
            continue;
        };
        removals.extend(source_filter_removals(id, source)?);
        if let Some(rules) = &source.send_tools {
            let routing =
                routing::CompiledRules::new(rules, source.config.prefix.as_deref().unwrap())?;
            for tool in &source.tools {
                let decision = routing.decision(&tool.name);
                if decision.targets.iter().any(|target| target == name) {
                    continue;
                }
                let action = if decision.excluded {
                    "excludes matching tools".to_owned()
                } else {
                    format!("assigns matching tools to {}", decision.targets.join(", "))
                };
                removals
                    .entry(format!(
                        "sources.{id}.send_tools[{}] {action}",
                        decision.rule_index
                    ))
                    .or_insert_with(Vec::new)
                    .push(tool.name.clone());
            }
        }
    }
    Ok(removals)
}

fn source_filter_removals(id: &str, source: &Source) -> Result<BTreeMap<String, Vec<String>>> {
    let cfg = &source.config;
    if cfg.include_tools.is_empty() && cfg.exclude_tools.is_empty() && !cfg.use_recommended_tools {
        return Ok(BTreeMap::new());
    }
    // Reuse path/tag selection, POST preference and collision suffixes. This
    // derives names through compatibility projection, without generating descriptions or prices.
    let names = catalog::tool_names_before_filtering(
        cfg,
        &source.document,
        cfg.prefix.as_deref().unwrap(),
    )?;
    let mut rules = Vec::new();
    for excluded in &cfg.exclude_tools {
        rules.push((
            format!("sources.{id}.exclude_tools contains {excluded:?}"),
            vec![pattern(excluded)?],
            true,
        ));
    }
    for (setting, selectors, enabled) in [
        (
            "include_tools",
            &cfg.include_tools,
            !cfg.include_tools.is_empty(),
        ),
        (
            "recommended_tools (use_recommended_tools=true)",
            &cfg.recommended_tools,
            cfg.use_recommended_tools,
        ),
    ] {
        if enabled {
            let patterns = selectors
                .iter()
                .map(|selector| pattern(selector))
                .collect::<Result<Vec<_>>>()?;
            rules.push((
                format!("sources.{id}.{setting} does not include the matching tool"),
                patterns,
                false,
            ));
        }
    }
    Ok(rules
        .into_iter()
        .map(|(reason, patterns, exclude)| {
            let removed = names
                .iter()
                .filter(|name| patterns.iter().any(|pattern| pattern.is_match(name)) == exclude)
                .cloned()
                .collect();
            (reason, removed)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(settings: &str) -> Source {
        let cfg: Config = toml::from_str(&format!(
            "prefix='vendor'\nbase_url='https://example.com'\n{settings}"
        ))
        .unwrap();
        Source::from_document(
            "alias",
            cfg,
            std::sync::Arc::new(json!({"operations":[
                {"method":"GET","path":"/read","tags":["visible"]},
                {"method":"GET","path":"/hidden","tags":["private"]},
                {"method":"GET","path":"/hidden_extra","tags":["private"]}
            ]})),
        )
        .unwrap()
    }

    fn listener() -> ListenerConfig {
        serde_json::from_value(json!({"listen":"127.0.0.1:0","sources":["alias"],
            "discover_on_demand":true,"include_tools":["vendor_hidden*"]}))
        .unwrap()
    }

    fn selection(
        name: &str,
        server: &ListenerConfig,
        sources: &BTreeMap<String, Source>,
    ) -> Selection {
        let inventory =
            listener_tool_inventory(name, server, sources, &BTreeMap::new(), true).unwrap();
        Selection {
            unmatched: inventory.unmatched,
            empty: inventory.tools.is_empty(),
        }
    }

    fn test_warnings(
        name: &str,
        server: &ListenerConfig,
        sources: &BTreeMap<String, Source>,
    ) -> Vec<Warning> {
        warnings(name, server, sources, &selection(name, server, sources))
    }

    fn report_test_selection(
        listeners: &BTreeMap<String, ListenerConfig>,
        sources: &BTreeMap<String, Source>,
        completed: Option<&str>,
    ) {
        let selections = listeners
            .iter()
            .map(|(name, server)| (name.clone(), selection(name, server, sources)))
            .collect();
        report_completed(
            listeners,
            sources,
            &BTreeMap::new(),
            &selections,
            &BTreeMap::new(),
            &BTreeMap::new(),
            completed,
        );
    }

    #[test]
    fn source_filter_warnings_identify_resolved_settings_with_one_witness() {
        for (settings, expected) in [
            (
                "exclude_tools=['vendor_hidden*']",
                "sources.alias.exclude_tools contains \"vendor_hidden*\"",
            ),
            (
                "include_tools=['vendor_read']",
                "sources.alias.include_tools",
            ),
            (
                "recommended_tools=['vendor_read']\nuse_recommended_tools=true",
                "sources.alias.recommended_tools (use_recommended_tools=true)",
            ),
        ] {
            let sources = BTreeMap::from([("alias".into(), source(settings))]);
            let mut server = listener();
            server.exclude_tools = server.include_tools.clone();
            let actual = test_warnings("main", &server, &sources);
            assert_eq!(actual.len(), 3);
            assert_eq!(actual[2].setting, "tool_filters");
            assert_eq!(actual[0].setting, "include_tools");
            assert_eq!(actual[1].setting, "exclude_tools");
            for warning in &actual[..2] {
                assert_eq!(warning.selector, "vendor_hidden*");
                assert!(warning.explanation.contains(expected), "{warning:?}");
                assert!(warning.explanation.contains("removes vendor_hidden"));
                assert_eq!(warning.explanation.matches("(removes ").count(), 1);
            }
        }
    }

    #[test]
    fn advisory_recommendations_and_listener_filters_are_not_source_conflicts() {
        let sources =
            BTreeMap::from([("alias".into(), source("recommended_tools=['vendor_read']"))]);
        let mut server = listener();
        assert!(test_warnings("main", &server, &sources).is_empty());
        server.tags = vec!["visible".into()];
        let actual = test_warnings("main", &server, &sources);
        assert_eq!(actual.len(), 2);
        assert!(
            !actual[0]
                .explanation
                .contains("already removed by source filters")
        );
        assert!(!actual[0].explanation.contains("recommended_tools"));
        server.tags.clear();
        server.exclude_tags = vec!["private".into()];
        let selected = sources["alias"].listener_tools("main", &server).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|tool| tool.path.as_str())
                .collect::<Vec<_>>(),
            ["/read"]
        );
        assert_eq!(test_warnings("main", &server, &sources).len(), 2);
        let sources = BTreeMap::from([("alias".into(), source("exclude=['/hidden']"))]);
        server.exclude_tags.clear();
        // The sibling remains selected, so narrow the listener to the removed path.
        server.include_tools = vec!["vendor_hidde?".into()];
        let actual = test_warnings("main", &server, &sources);
        assert_eq!(actual.len(), 2);
        assert!(
            !actual[0]
                .explanation
                .contains("already removed by source filters")
        );
    }

    #[test]
    fn help_names_and_post_preference_use_the_catalog_naming_pipeline() {
        let mut cfg = source("").config;
        cfg.help_url = Some("https://example.com/help".into());
        let document = json!({"operations":[
            {"method":"GET","path":"/read"},
            {"method":"POST","path":"/read"},
            {"method":"GET","path":"/item/{id}"},
            {"method":"GET","path":"/item/{name}"}
        ]});
        let actual = catalog::tool_names_before_filtering(&cfg, &document, "vendor").unwrap();
        let tools = catalog::build_tools(&cfg, &document, "vendor").unwrap();
        assert_eq!(
            actual,
            tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(tools.iter().filter(|tool| tool.path == "/read").count(), 1);
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.path == "/read")
                .unwrap()
                .method,
            "POST"
        );
        cfg.exclude_tools = vec!["vendor_help".into()];
        let source = Source::from_document("alias", cfg, std::sync::Arc::new(document)).unwrap();
        let sources = BTreeMap::from([("alias".into(), source)]);
        let mut server = listener();
        server.include_tools = vec!["vendor_help*".into()];
        let actual = test_warnings("main", &server, &sources);
        assert!(actual[0].explanation.contains(
            "sources.alias.exclude_tools contains \"vendor_help\" (removes vendor_help)"
        ));
    }

    #[tokio::test]
    async fn distribution_advice_is_debug_and_missing_listener_selections_report_coverage() {
        if crate::test_log_capture::run_in_child(
            "deployment::selection_diagnostics::tests::distribution_advice_is_debug_and_missing_listener_selections_report_coverage",
        ) {
            return;
        }
        let (_, logs) = crate::limits::tests::capture(async {
            let send_tools = serde_json::from_value(json!([
                {"to":"main","tools":["*"]},
                {"to":"sibling","tools":["future_*"]},
                {"to":"main"}
            ]))
            .unwrap();
            let routed = source("")
                .with_send_tools("alias", Some(send_tools))
                .unwrap();
            let sources = BTreeMap::from([("alias".into(), routed)]);
            let mut server = listener();
            server.include_tools = vec!["vendor_missing*".into()];
            assert!(
                listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false)
                    .unwrap()
                    .tools
                    .is_empty()
            );
            report_test_selection(&BTreeMap::from([("main".into(), server)]), &sources, None);
        })
        .await;
        for message in [
            "Source distribution pattern matches no available tools",
            "Source distribution rule assigns no current tools",
        ] {
            assert!(
                logs.lines()
                    .any(|line| line.contains("DEBUG") && line.contains(message)),
                "{logs}"
            );
        }
        assert!(
            logs.lines()
                .any(|line| line.contains("WARN") && line.contains("setting=\"include_tools\"")),
            "{logs}"
        );
        assert!(
            logs.lines()
                .any(|line| line.contains("WARN") && line.contains("setting=\"tool_filters\"")),
            "{logs}"
        );
        assert!(!logs.contains("INFO"), "{logs}");
    }

    #[tokio::test]
    async fn collision_omission_reports_empty_listener_coverage() {
        if crate::test_log_capture::run_in_child(
            "deployment::selection_diagnostics::tests::collision_omission_reports_empty_listener_coverage",
        ) {
            return;
        }
        let (_, logs) = crate::limits::tests::capture(async {
            let sources =
                BTreeMap::from([("alias".into(), source("")), ("other".into(), source(""))]);
            let mut server = listener();
            server.sources.push("other".into());
            let selection = selection("main", &server, &sources);
            assert!(selection.empty);
            assert!(selection.unmatched.is_empty());
            report_completed(
                &BTreeMap::from([("main".into(), server)]),
                &sources,
                &BTreeMap::new(),
                &BTreeMap::from([("main".into(), selection)]),
                &BTreeMap::new(),
                &BTreeMap::new(),
                Some("other"),
            );
        })
        .await;
        assert!(
            logs.lines().any(|line| line.contains("WARN")
                && line.contains("server=\"main\"")
                && line.contains("setting=\"tool_filters\"")),
            "{logs}"
        );
        assert!(!logs.contains("setting=\"include_tools\""), "{logs}");
    }

    #[tokio::test]
    async fn partial_catalog_removal_evidence_does_not_suppress_endpoint_coverage_warnings() {
        if crate::test_log_capture::run_in_child(
            "deployment::selection_diagnostics::tests::partial_catalog_removal_evidence_does_not_suppress_endpoint_coverage_warnings",
        ) {
            return;
        }
        let (_, logs) = crate::limits::tests::capture(async {
            let cfg: Config = toml::from_str(
                "prefix='vendor'\nbase_url='https://example.com'\ninclude_tools=['vendor_read']",
            )
            .unwrap();
            let partial = Source::from_document(
                "alias",
                cfg,
                std::sync::Arc::new(json!({"paths":{
                    "/read":{"get":{}},
                    "/unsupported":{"post":{"parameters":[{
                        "name":"session","in":"cookie","schema":{"type":"string"}
                    }]}}
                }})),
            )
            .unwrap();
            assert_eq!(partial.tools.len(), 1);
            assert_eq!(partial.tools[0].name, "vendor_read");
            assert_eq!(partial.issues.len(), 1);
            let sources = BTreeMap::from([("alias".into(), partial)]);
            let mut server = listener();
            server.include_tools = vec!["vendor_missing*".into()];
            assert!(selection_removals("main", &server, &sources).is_err());
            let actual = test_warnings("main", &server, &sources);
            assert_eq!(actual.len(), 2);
            assert_eq!(actual[0].setting, "include_tools");
            assert_eq!(actual[0].selector, "vendor_missing*");
            assert_eq!(actual[1].setting, "tool_filters");
            report_test_selection(
                &BTreeMap::from([("main".into(), server)]),
                &sources,
                Some("alias"),
            );
        })
        .await;
        for (severity, setting) in [
            ("WARN", "setting=\"include_tools\""),
            ("WARN", "setting=\"tool_filters\""),
        ] {
            assert!(
                logs.lines()
                    .any(|line| line.contains(severity) && line.contains(setting)),
                "{logs}"
            );
        }
    }

    #[tokio::test]
    async fn unmatched_exclusions_are_info_and_do_not_block_valid_tools() {
        if crate::test_log_capture::run_in_child(
            "deployment::selection_diagnostics::tests::unmatched_exclusions_are_info_and_do_not_block_valid_tools",
        ) {
            return;
        }
        let (_, logs) = crate::limits::tests::capture(async {
            let send_tools = serde_json::from_value(json!([
                {"to":"exclude","tools":["obsolete","obsolete_*","hidden"]},
                {"to":"main"}
            ]))
            .unwrap();
            let routed = source("exclude_tools=['vendor_absent','vendor_future*']")
                .with_send_tools("alias", Some(send_tools))
                .unwrap();
            let sources = BTreeMap::from([("alias".into(), routed)]);
            let mut server = listener();
            server.include_tools.clear();
            server.exclude_tools = vec![
                "vendor_absent".into(),
                "vendor_future*".into(),
                "vendor_hidden".into(),
                "vendor_hidden_extra".into(),
            ];
            let inventory =
                listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false)
                    .unwrap();
            assert_eq!(inventory.tools.len(), 1);
            assert_eq!(inventory.tools[0].1.name, "vendor_read");
            let listeners = BTreeMap::from([("main".into(), server)]);
            // Unrelated completions must not repeat a listener's diagnostics.
            report_test_selection(&listeners, &sources, Some("unrelated"));
            report_test_selection(&listeners, &sources, Some("alias"));
            report_test_selection(&listeners, &sources, Some("unrelated"));
        })
        .await;
        for (target, field, count) in [
            ("::catalog:", "setting=\"exclude_tools\"", 2),
            ("::routing:", "rule=0", 2),
            ("::selection_diagnostics:", "setting=\"exclude_tools\"", 3),
        ] {
            let lines: Vec<_> = logs
                .lines()
                .filter(|line| line.contains(target) && line.contains(field))
                .collect();
            assert_eq!(lines.len(), count, "{logs}");
            assert!(lines.iter().all(|line| line.contains("INFO")), "{logs}");
        }
        assert!(logs.contains("sources.alias.send_tools[0]"), "{logs}");
        assert!(!logs.contains("WARN") && !logs.contains("ERROR"), "{logs}");
    }

    #[test]
    fn routing_removals_explain_exact_and_wildcard_listener_selectors() {
        for (target, expected) in [
            ("exclude", "excludes matching tools"),
            ("sibling", "assigns matching tools to sibling"),
        ] {
            let send_tools = serde_json::from_value(json!([
                {"to":target,"tools":["hidden*"]},{"to":"main"}
            ]))
            .unwrap();
            let routed = source("")
                .with_send_tools("alias", Some(send_tools))
                .unwrap();
            let sources = BTreeMap::from([("alias".into(), routed)]);
            let mut server = listener();
            let actual = test_warnings("main", &server, &sources);
            assert_eq!(actual.len(), 2);
            assert_eq!(actual[1].setting, "tool_filters");
            assert!(
                actual[0]
                    .explanation
                    .contains("sources.alias.send_tools[0]")
            );
            assert!(actual[0].explanation.contains(expected));
            assert!(actual[0].explanation.contains("removes vendor_hidden"));
            for exclude in [false, true] {
                server.include_tools = if exclude {
                    Vec::new()
                } else {
                    vec!["vendor_hidden".into()]
                };
                server.exclude_tools = if exclude {
                    vec!["vendor_hidden".into()]
                } else {
                    Vec::new()
                };
                let selected =
                    listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false);
                if exclude {
                    let selected = selected.unwrap();
                    assert_eq!(selected.tools.len(), 1);
                    let diagnostic = test_warnings("main", &server, &sources);
                    assert_eq!(diagnostic[0].setting, "exclude_tools");
                    assert!(diagnostic[0].explanation.contains(expected));
                } else {
                    let error = selected.err().unwrap();
                    let detail = format!("{error:#}");
                    assert!(
                        detail.contains("unknown tool selector vendor_hidden"),
                        "{detail}"
                    );
                    assert!(
                        detail.contains("sources.alias.send_tools[0]") && detail.contains(expected),
                        "{detail}"
                    );
                }
            }
            server.include_tools.clear();
            server.exclude_tools.clear();
            let selected =
                listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false)
                    .unwrap();
            assert_eq!(selected.tools.len(), 1);
            assert_eq!(selected.tools[0].1.name, "vendor_read");
        }
    }

    #[tokio::test]
    async fn mixed_source_filters_and_distribution_report_both_removal_causes() {
        if crate::test_log_capture::run_in_child(
            "deployment::selection_diagnostics::tests::mixed_source_filters_and_distribution_report_both_removal_causes",
        ) {
            return;
        }
        let (_, logs) = crate::limits::tests::capture(async {
            let send_tools = serde_json::from_value(json!([
                {"to":"sibling","tools":["hidden"]},{"to":"main"}
            ]))
            .unwrap();
            let routed = source("exclude_tools=['vendor_hidden_extra']")
                .with_send_tools("alias", Some(send_tools))
                .unwrap();
            let sources = BTreeMap::from([("alias".into(), routed)]);
            let actual = test_warnings("main", &listener(), &sources);
            assert_eq!(actual.len(), 2);
            assert_eq!(actual[1].setting, "tool_filters");
            assert!(
                actual[0]
                    .explanation
                    .contains("sources.alias.exclude_tools")
            );
            assert!(
                actual[0]
                    .explanation
                    .contains("sources.alias.send_tools[0]")
            );
            assert!(
                actual[0]
                    .explanation
                    .contains("removes vendor_hidden_extra")
            );
            assert!(actual[0].explanation.contains("removes vendor_hidden)"));
            report_test_selection(
                &BTreeMap::from([("main".into(), listener())]),
                &sources,
                Some("alias"),
            );
        })
        .await;
        let diagnostic = logs
            .lines()
            .find(|line| line.contains("WARN") && line.contains("setting=\"include_tools\""))
            .expect("missing selection must warn");
        for expected in [
            "selector=\"vendor_hidden*\"",
            "sources.alias.exclude_tools",
            "removes vendor_hidden_extra",
            "sources.alias.send_tools[0] assigns matching tools to sibling",
            "removes vendor_hidden)",
        ] {
            assert!(diagnostic.contains(expected), "{logs}");
        }
    }

    #[test]
    fn missing_selectors_report_coverage_without_namespace_validation() {
        let sources = BTreeMap::from([("alias".into(), source(""))]);
        let unavailable = BTreeMap::from([("offline".into(), "offline".into())]);
        let mut server = listener();
        server.sources.push("offline".into());
        for selector in ["offline_read", "vendor_typo", "unknown_read", "offline_*"] {
            server.include_tools = vec![selector.into()];
            let inventory =
                listener_tool_inventory("one", &server, &sources, &unavailable, true).unwrap();
            assert!(inventory.tools.is_empty());
            let actual = warnings(
                "one",
                &server,
                &sources,
                &Selection {
                    unmatched: inventory.unmatched,
                    empty: inventory.tools.is_empty(),
                },
            );
            assert_eq!(actual.len(), 2);
            assert_eq!(actual[0].setting, "include_tools");
            assert!(
                actual[0]
                    .explanation
                    .contains("Unavailable sources: [\"offline\"]")
            );
            assert_eq!(actual[1].setting, "tool_filters");
        }
    }
}
