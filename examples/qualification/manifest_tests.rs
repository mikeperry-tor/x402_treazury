//! Offline checks for the reviewed mainnet input templates. Never loads secrets.
use super::*;
use std::collections::BTreeMap;

#[tokio::test]
async fn full_manifest_resolves_scope_arguments_coverage_and_budget_offline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut manifest: Manifest =
        toml::from_str(include_str!("../../tests/live/providers.toml")).unwrap();
    assert!(
        manifest.validate(1000).is_err(),
        "template must refuse execution"
    );
    manifest.expires_at = 2000;
    manifest.validate(1000).unwrap();
    assert_eq!(manifest.cases.len(), 50);
    let reserved: u64 = manifest
        .cases
        .iter()
        .map(|c| atomic(&c.reserve_usdc).unwrap())
        .sum();
    assert_eq!(reserved, atomic(&manifest.api_budget_usdc).unwrap());
    assert_eq!(manifest.max_funding_jobs, 6);

    let coverage: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/live/coverage.json")).unwrap();
    fn providers(dir: &Path, root: &Path, files: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                providers(&path, root, files);
            } else if path.extension().is_some_and(|x| x == "toml") {
                files.insert(path.strip_prefix(root).unwrap().to_str().unwrap().into());
            }
        }
    }
    let mut files = BTreeSet::new();
    providers(&root.join("providers"), root, &mut files);
    assert_eq!(
        files,
        coverage
            .iter()
            .map(|v| v["config"].as_str().unwrap().into())
            .collect()
    );
    assert_eq!(coverage.len(), 24);
    for row in &coverage {
        match row["status"].as_str().unwrap() {
            "not_attempted" => assert!(manifest.cases.iter().any(|c| c.id == row["case"])),
            "blocked_by_dependency" => {
                assert!(row["case"].is_null());
                assert!(!row["reason"].as_str().unwrap().is_empty());
            }
            x => panic!("unexpected unexecuted status {x}"),
        }
    }

    let fixture_cases: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/catalogs/cases.json")).unwrap();
    let mut fixtures: BTreeMap<String, PathBuf> = fixture_cases
        .iter()
        .filter_map(|v| {
            Some((
                v["provider"].as_str()?.into(),
                root.join(v["spec"].as_str()?),
            ))
        })
        .collect();
    for name in ["arkham", "botsmith", "google-trends", "x402stock"] {
        fixtures.insert(
            format!("providers/{name}.toml"),
            root.join(format!("tests/live/fixtures/{name}.json")),
        );
    }
    let original = root.join("examples/live-tor-providers.toml");
    let mut table = x402_treazury::config::read_table(&original).await.unwrap();
    // Production templates retain live URLs and Tor. This test alone substitutes
    // request snapshots and the direct policy; every spec below must be local.
    table.remove("network");
    for (_, source) in table["sources"].as_table_mut().unwrap().iter_mut() {
        let s = source.as_table_mut().unwrap();
        let relative = s["extends"]
            .as_str()
            .unwrap()
            .strip_prefix("../")
            .unwrap()
            .to_owned();
        let provider = root.join(&relative);
        s.insert(
            "extends".into(),
            toml::Value::String(provider.to_str().unwrap().into()),
        );
        let local_spec = fixtures.get(&relative).cloned().unwrap_or_else(|| {
            let text = std::fs::read_to_string(&provider).unwrap();
            let p: toml::Table = toml::from_str(&text).unwrap();
            provider.parent().unwrap().join(p["spec"].as_str().unwrap())
        });
        assert!(local_spec.is_file(), "missing offline spec for {relative}");
        s.insert(
            "spec".into(),
            toml::Value::String(local_spec.to_str().unwrap().into()),
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("deployment.toml");
    std::fs::write(&path, toml::to_string(&table).unwrap()).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let inventory = deployment.inventory();
    for case in &manifest.cases {
        let server = inventory.iter().find(|s| s.server == case.server).unwrap();
        let entry = server
            .tools
            .iter()
            .find(|t| t.tool.name == case.tool)
            .unwrap_or_else(|| panic!("{} tool {} not exposed", case.id, case.tool));
        let schema = serde_json::to_value(&entry.tool).unwrap()["input_schema"].clone();
        for name in schema["required"].as_array().into_iter().flatten() {
            assert!(
                case.arguments.contains_key(name.as_str().unwrap()),
                "{} missing {name}",
                case.id
            );
        }
        for (name, value) in &case.arguments {
            let property = &schema["properties"][name];
            assert!(!property.is_null(), "{} unknown argument {name}", case.id);
            assert!(
                matches_type(value, property),
                "{} invalid argument {name}",
                case.id
            );
        }
        let expected = if case.id.starts_with("rotation_") || case.id.starts_with("shared_") {
            "coverage_a"
        } else {
            "coverage_b"
        };
        let binding = serde_json::to_value(&server.wallet_bindings[&entry.source]).unwrap();
        assert_eq!(binding["wallet"], expected, "{}", case.id);
    }
    let b = inventory.iter().find(|s| s.server == "b").unwrap();
    assert!(
        !b.tools
            .iter()
            .any(|t| t.tool.name == "rotation_balances_address")
    );
    let c = inventory.iter().find(|s| s.server == "c").unwrap();
    assert_eq!(c.tools.len(), 2, "isolated profile and help only");
}

// Check the simple types, choices and numeric bounds used by these reviewed flat
// inputs. This is deliberately not a general JSON Schema validator.
fn matches_type(v: &Value, s: &Value) -> bool {
    if let Some(choices) = s["anyOf"].as_array().or_else(|| s["oneOf"].as_array()) {
        return choices.iter().any(|s| matches_type(v, s));
    }
    let correct = match s["type"].as_str() {
        Some("string") => v.is_string(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("number") => v.is_number(),
        Some("boolean") => v.is_boolean(),
        Some("array") => v
            .as_array()
            .is_some_and(|a| a.iter().all(|v| matches_type(v, &s["items"]))),
        Some("object") => v.is_object(),
        Some("null") => v.is_null(),
        None => true,
        _ => false,
    };
    correct
        && s["enum"].as_array().is_none_or(|a| a.contains(v))
        && s["minimum"]
            .as_f64()
            .is_none_or(|n| v.as_f64().is_some_and(|x| x >= n))
        && s["maximum"]
            .as_f64()
            .is_none_or(|n| v.as_f64().is_some_and(|x| x <= n))
}
