//! Report listener selection only when its own source loads have settled.
use super::*;

pub(super) fn report_completed(
    config: &MetaConfig,
    sources: &BTreeMap<String, Source>,
    pending: &BTreeMap<String, startup::PendingSource>,
    completed: Option<&str>,
) {
    for (name, server) in &config.servers {
        if completed.is_some_and(|id| !server.sources.iter().any(|source| source == id))
            || server.sources.iter().any(|id| pending.contains_key(id))
        {
            continue;
        }
        // Diagnostics cannot change catalog acceptance or publication.
        match warnings(server, sources) {
            Ok(warnings) => {
                for warning in warnings {
                    tracing::warn!(
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

fn warnings(server: &ListenerConfig, sources: &BTreeMap<String, Source>) -> Result<Vec<Warning>> {
    let mut available = BTreeSet::new();
    for source in server.sources.iter().filter_map(|id| sources.get(id)) {
        let operations = catalog::operations(&source.document, None)?;
        available.extend(
            source
                .tools
                .iter()
                .filter(|tool| {
                    catalog::matches_operation_tags(
                        &operations,
                        tool,
                        &server.tags,
                        &server.exclude_tags,
                    )
                })
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
    let mut removals = BTreeMap::new();
    for id in &server.sources {
        if let Some(source) = sources.get(id) {
            removals.extend(source_filter_removals(id, source)?);
        }
    }
    Ok(unmatched.into_iter().map(|(setting, selector, pattern)| {
        let causes: Vec<_> = removals.iter().filter_map(|(reason, names)| {
            names.iter().find(|name| pattern.is_match(name))
                .map(|name| format!("{reason} (removes {name})"))
        }).collect();
        let explanation = if causes.is_empty() {
            format!("Listener tool pattern matches no available tools; check the pattern, source availability and source/listener filters. Unavailable sources: {unavailable:?}")
        } else {
            format!("Listener tool pattern matches no available tools; matching tools were already removed by source filters: {}. Listener filters cannot restore source tools", causes.join("; "))
        };
        Warning { setting, selector: selector.clone(), explanation }
    }).collect())
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
            let actual = warnings(&server, &sources).unwrap();
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
        assert!(warnings(&server, &sources).unwrap().is_empty());
        server.tags = vec!["visible".into()];
        let actual = warnings(&server, &sources).unwrap();
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
        let actual = warnings(&server, &sources).unwrap();
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
        let actual = warnings(&server, &sources).unwrap();
        assert!(actual[0].explanation.contains(
            "sources.alias.exclude_tools contains \"vendor_help\" (removes vendor_help)"
        ));
    }

    #[test]
    fn unavailable_exact_selectors_stay_silent_without_hiding_ready_source_typos() {
        let sources = BTreeMap::from([("alias".into(), source(""))]);
        let unavailable = BTreeMap::from([("offline".into(), "offline".into())]);
        let mut server = listener();
        server.sources.push("offline".into());
        server.include_tools = vec!["offline_read".into()];
        listener_tool_inventory("one", &server, &sources, &unavailable, true).unwrap();
        assert!(warnings(&server, &sources).unwrap().is_empty());
        for selector in ["vendor_typo", "unknown_read"] {
            server.include_tools = vec![selector.into()];
            assert!(listener_tool_inventory("one", &server, &sources, &unavailable, true).is_err());
        }
        server.include_tools = vec!["offline_*".into()];
        let actual = warnings(&server, &sources).unwrap();
        assert!(
            actual[0]
                .explanation
                .contains("Unavailable sources: [\"offline\"]")
        );
    }
}
