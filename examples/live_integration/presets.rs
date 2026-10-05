//! Offline validation of committed runner presets against reviewed catalogs.
#[cfg(test)]
mod tests {
    use crate::{manifest::Manifest, planner, schema};
    use serde_json::Value;
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
    };
    use x402_treazury::{
        catalog::{self, Config},
        deployment::Deployment,
    };

    #[tokio::test]
    async fn scenario_presets_plan_explicit_wallet_scope_allocations_and_rounds() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let directory = root.join("tests/live/integration");
        for (name, count, reserve, jobs, pools) in [
            ("short", 1, 200_000, 0, 1),
            ("standard", 4, 800_000, 0, 2),
            ("rotation", 14, 2_800_000, 1, 2),
            ("extended", 28, 5_600_000, 2, 2),
            ("treasury-only", 28, 5_600_000, 6, 2),
            ("catalog", 3, 0, 0, 0),
            ("reliability", 2, 400_000, 0, 1),
        ] {
            let text =
                std::fs::read_to_string(directory.join(format!("{name}.example.toml"))).unwrap();
            let mut m: Manifest = toml::from_str(&text).unwrap();
            assert!(
                m.validate().is_err(),
                "{name} must retain execution-blocking placeholders"
            );
            m.run_id = "fixture".into();
            m.treasury_id = "11111111-1111-4111-8111-111111111111".into();
            m.deployment = directory.join(&m.deployment);
            for (i, window) in m.windows.iter_mut().enumerate() {
                window.not_before = 1 + i as u64 * 86400;
                window.not_after = 100 + i as u64 * 86400;
            }
            let config = Deployment::show_config(&m.deployment).await.unwrap();
            let plan = planner::build(&m, &config).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert_eq!(m.cases.len(), count, "{name}");
            assert_eq!(plan.api_reservation_atomic, reserve, "{name}");
            assert_eq!(plan.planned_new_funding_jobs, jobs, "{name}");
            assert_eq!(plan.declared_managed_pools.len(), pools, "{name}");
            assert!(!plan.execution_authorized);
            for case in &m.cases {
                let settings: Config =
                    serde_json::from_value(config["sources"][&case.source]["settings"].clone())
                        .unwrap();
                let fixture = match case.source.as_str() {
                    "depletion" => "tests/live/fixtures/arkham.json",
                    "directory" => "tests/fixtures/x402_list_openapi.json",
                    _ => "tests/fixtures/socialfetch_openapi.json",
                };
                let document: Value =
                    serde_json::from_slice(&std::fs::read(root.join(fixture)).unwrap()).unwrap();
                let tools =
                    catalog::build_tools(&settings, &document, settings.prefix.as_deref().unwrap())
                        .unwrap();
                let tool = tools.iter().find(|t| t.name == case.tool).unwrap();
                schema::validate(&tool.input_schema, &Value::Object(case.arguments.clone()))
                    .unwrap();
                assert!(
                    config["servers"][&case.server]["include_tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| t == &case.tool)
                );
            }
            if name == "standard" {
                assert_eq!(
                    plan.case_bindings["shared_a"]["wallet"],
                    plan.case_bindings["shared_b"]["wallet"]
                );
                assert_ne!(
                    plan.case_bindings["parallel_a"]["wallet"],
                    plan.case_bindings["parallel_c"]["wallet"]
                );
            }
            if name == "extended" || name == "treasury-only" {
                let crate::manifest::Scenario::Lifecycle { rounds, .. } = &m.phases[0].scenario
                else {
                    panic!("missing lifecycle")
                };
                assert_eq!(rounds.len(), 2);
                for round in rounds {
                    assert_eq!(round.depletion_cases.len(), 12);
                    assert_eq!(round.service_cases.len(), 2);
                    assert_ne!(
                        plan.case_bindings[&round.service_cases[0]]["wallet"],
                        plan.case_bindings[&round.service_cases[1]]["wallet"]
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn provider_preset_preserves_reviewed_calls_and_matches_offline_catalogs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let text =
            std::fs::read_to_string(root.join("tests/live/integration/providers.example.toml"))
                .unwrap();
        let mut manifest: Manifest = toml::from_str(&text).unwrap();
        assert!(
            manifest.validate().is_err(),
            "committed placeholders must prohibit execution"
        );
        manifest.run_id = "provider_fixture".into();
        manifest.treasury_id = "11111111-1111-4111-8111-111111111111".into();
        manifest.windows[0].not_before = 1;
        manifest.windows[0].not_after = 20000;
        manifest.deployment = root.join("examples/live-tor-providers.toml");
        let config = Deployment::show_config(&manifest.deployment).await.unwrap();
        let plan = planner::build(&manifest, &config).unwrap();
        assert_eq!(plan.api_reservation_atomic, 4_400_000);
        assert_eq!(plan.planned_new_funding_jobs, 0);
        assert!(!plan.execution_authorized);
        assert_eq!(manifest.cases.len(), 63);
        let sources: BTreeSet<_> = manifest.cases.iter().map(|c| c.source.as_str()).collect();
        assert_eq!(sources.len(), 23);

        let frozen: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/catalogs/cases.json")).unwrap();
        let mut fixtures: BTreeMap<String, PathBuf> = frozen
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| {
                v["spec"]
                    .as_str()
                    .map(|p| (v["name"].as_str().unwrap().replace('-', "_"), root.join(p)))
            })
            .collect();
        for (source, file) in [
            ("arkham", "arkham"),
            ("botsmith", "botsmith"),
            ("google_trends", "google-trends"),
            ("x402stock", "x402stock"),
        ] {
            fixtures.insert(
                source.into(),
                root.join(format!("tests/live/fixtures/{file}.json")),
            );
        }
        let mut tools = BTreeMap::new();
        for source in sources {
            let settings: Config =
                serde_json::from_value(config["sources"][source]["settings"].clone()).unwrap();
            let path = fixtures
                .get(source)
                .cloned()
                .unwrap_or_else(|| PathBuf::from(&settings.spec));
            assert!(path.is_file(), "missing offline catalog for {source}");
            let document: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let generated =
                catalog::build_tools(&settings, &document, settings.prefix.as_deref().unwrap())
                    .unwrap();
            tools.insert(source.to_owned(), generated);
        }
        for case in &manifest.cases {
            let tool = tools[&case.source]
                .iter()
                .find(|t| t.name == case.tool)
                .unwrap_or_else(|| panic!("missing reviewed tool {}", case.tool));
            schema::validate(&tool.input_schema, &Value::Object(case.arguments.clone()))
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.id));
            assert!(
                config["servers"][&case.server]["include_tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t == &case.tool)
            );
            if case.help_cache.is_some() {
                assert!(case.unsigned && tool.help_url.is_some());
            }
        }
        let old: toml::Value =
            toml::from_str(include_str!("../../tests/live/providers.toml")).unwrap();
        for legacy in old["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["id"].as_str().unwrap().starts_with("sweep_"))
        {
            let case = manifest
                .cases
                .iter()
                .find(|c| Some(c.id.as_str()) == legacy["id"].as_str())
                .unwrap();
            assert_eq!(
                serde_json::to_value(&case.arguments).unwrap(),
                serde_json::to_value(&legacy["arguments"]).unwrap()
            );
            assert_eq!(case.tool, legacy["tool"].as_str().unwrap());
            assert_eq!(case.reserve_usdc, legacy["reserve_usdc"].as_str().unwrap());
        }
    }
}
