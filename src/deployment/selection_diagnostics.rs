//! Report listener selection only when its own source loads have settled.
use super::*;

pub(super) fn report_completed(
    listeners: &BTreeMap<String, ListenerConfig>,
    sources: &BTreeMap<String, Source>,
    pending: &BTreeMap<String, startup::PendingSource>,
    completed: Option<&str>,
) {
    for (name, server) in listeners {
        if completed.is_some_and(|id| !server.sources.iter().any(|source| source == id))
            || server.sources.iter().any(|id| pending.contains_key(id))
        {
            continue;
        }
        // Diagnostics cannot change catalog acceptance or publication.
        match warnings(name, server, sources) {
            Ok(warnings) => {
                for warning in warnings {
                    tracing::debug!(
                        server = name,
                        setting = warning.setting,
                        selector = warning.selector,
                        "{}",
                        warning.explanation
                    );
                }
            }
            Err(_) => tracing::debug!(
                server = name,
                "Listener selection diagnostics could not be generated"
            ),
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
) -> Result<Vec<Warning>> {
    let mut available = BTreeSet::new();
    for source in server.sources.iter().filter_map(|id| sources.get(id)) {
        available.extend(
            source
                .listener_tools(name, server)?
                .into_iter()
                .map(|tool| tool.name.as_str()),
        );
    }
    let unavailable: Vec<_> = server
        .sources
        .iter()
        .filter(|id| !sources.contains_key(*id))
        .collect();
    let mut unmatched = Vec::new();
    for (setting, selectors) in [
        ("include_tools", &server.include_tools),
        ("exclude_tools", &server.exclude_tools),
    ] {
        for selector in selectors.iter().filter(|s| s.contains(['*', '?'])) {
            let pattern = pattern(selector)?;
            if !available.iter().any(|name| pattern.is_match(name)) {
                unmatched.push((setting, selector, pattern));
            }
        }
    }
    if unmatched.is_empty() {
        return Ok(Vec::new());
    }
    // Temporary evidence is shared by every selector in this report, never
    // retained in runtime state. One witness per filter explains the cause.
    let removals = selection_removals(name, server, sources)?;
    Ok(unmatched.into_iter().map(|(setting, selector, pattern)| {
        let causes = removal_causes(&removals, &pattern);
        let explanation = if causes.is_empty() {
            format!("Listener tool pattern matches no available tools; check the pattern, source availability and source/listener filters. Unavailable sources: {unavailable:?}")
        } else {
            format!("Listener tool pattern matches no available tools; matching tools were already removed before listener name filtering: {}. Listener filters cannot restore source tools", causes.join("; "))
        };
        Warning { setting, selector: selector.clone(), explanation }
    }).collect())
}

pub(super) fn removal_causes(
    removals: &BTreeMap<String, Vec<String>>,
    pattern: &Regex,
) -> Vec<String> {
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

pub(super) fn selection_removals(
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
    // derives names only, without rebuilding schemas, descriptions or prices.
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
            let actual = warnings("main", &server, &sources).unwrap();
            assert_eq!(actual.len(), 2);
            assert_eq!(actual[0].setting, "include_tools");
            assert_eq!(actual[1].setting, "exclude_tools");
            for warning in actual {
                assert!(warning.explanation.contains(expected), "{warning:?}");
                assert!(warning.explanation.contains("removes vendor_hidden"));
                assert_eq!(warning.explanation.matches("(removes ").count(), 1);
                assert!(warning.explanation.contains("cannot restore source tools"));
            }
        }
    }

    #[test]
    fn advisory_recommendations_and_listener_filters_are_not_source_conflicts() {
        let sources =
            BTreeMap::from([("alias".into(), source("recommended_tools=['vendor_read']"))]);
        let mut server = listener();
        assert!(warnings("main", &server, &sources).unwrap().is_empty());
        server.tags = vec!["visible".into()];
        let actual = warnings("main", &server, &sources).unwrap();
        assert_eq!(actual.len(), 1);
        assert!(
            !actual[0]
                .explanation
                .contains("already removed by source filters")
        );
        assert!(!actual[0].explanation.contains("recommended_tools"));
        let sources = BTreeMap::from([("alias".into(), source("exclude=['/hidden']"))]);
        server.tags.clear();
        // The sibling remains selected, so narrow the listener to the removed path.
        server.include_tools = vec!["vendor_hidde?".into()];
        let actual = warnings("main", &server, &sources).unwrap();
        assert_eq!(actual.len(), 1);
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
        let actual = warnings("main", &server, &sources).unwrap();
        assert!(actual[0].explanation.contains(
            "sources.alias.exclude_tools contains \"vendor_help\" (removes vendor_help)"
        ));
    }

    #[tokio::test]
    async fn unmatched_distribution_and_listener_patterns_are_debug_diagnostics() {
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
            report_completed(
                &BTreeMap::from([("main".into(), server)]),
                &sources,
                &BTreeMap::new(),
                None,
            );
        })
        .await;
        for message in [
            "Source distribution pattern matches no available tools",
            "Source distribution rule assigns no current tools",
            "Listener tool pattern matches no available tools",
        ] {
            assert!(
                logs.lines()
                    .any(|line| line.contains("DEBUG") && line.contains(message)),
                "{logs}"
            );
        }
        assert!(
            !logs.contains("WARN") && !logs.contains("ERROR") && !logs.contains("INFO"),
            "{logs}"
        );
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
            let actual = warnings("main", &server, &sources).unwrap();
            assert_eq!(actual.len(), 1);
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
                let error =
                    listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false)
                        .err()
                        .unwrap();
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
            server.include_tools.clear();
            server.exclude_tools.clear();
            let selected =
                listener_tool_inventory("main", &server, &sources, &BTreeMap::new(), false)
                    .unwrap();
            assert_eq!(selected.tools.len(), 1);
            assert_eq!(selected.tools[0].1.name, "vendor_read");
        }
    }

    #[test]
    fn mixed_source_filters_and_distribution_report_both_removal_causes() {
        let send_tools = serde_json::from_value(json!([
            {"to":"sibling","tools":["hidden"]},{"to":"main"}
        ]))
        .unwrap();
        let routed = source("exclude_tools=['vendor_hidden_extra']")
            .with_send_tools("alias", Some(send_tools))
            .unwrap();
        let sources = BTreeMap::from([("alias".into(), routed)]);
        let actual = warnings("main", &listener(), &sources).unwrap();
        assert_eq!(actual.len(), 1);
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
    }

    #[test]
    fn unavailable_exact_selectors_stay_silent_without_hiding_ready_source_typos() {
        let sources = BTreeMap::from([("alias".into(), source(""))]);
        let unavailable = BTreeMap::from([("offline".into(), "offline".into())]);
        let mut server = listener();
        server.sources.push("offline".into());
        server.include_tools = vec!["offline_read".into()];
        listener_tool_inventory("one", &server, &sources, &unavailable, true).unwrap();
        assert!(warnings("main", &server, &sources).unwrap().is_empty());
        for selector in ["vendor_typo", "unknown_read"] {
            server.include_tools = vec![selector.into()];
            assert!(listener_tool_inventory("one", &server, &sources, &unavailable, true).is_err());
        }
        server.include_tools = vec!["offline_*".into()];
        let actual = warnings("main", &server, &sources).unwrap();
        assert!(
            actual[0]
                .explanation
                .contains("Unavailable sources: [\"offline\"]")
        );
    }
}
