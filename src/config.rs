//! TOML-only provider composition, independent of networking and credentials.
use crate::catalog::Config;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub struct ResolvedProvider {
    pub settings: Config,
    pub origins: BTreeMap<String, String>,
}
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}
fn resolve_spec(table: &mut toml::Table, declaring: &Path) {
    if let Some(toml::Value::String(spec)) = table.get_mut("spec")
        && !spec.starts_with("https://")
        && !spec.starts_with("http://")
        && !spec.is_empty()
    {
        *spec = declaring
            .parent()
            .unwrap_or(Path::new("."))
            .join(&*spec)
            .to_string_lossy()
            .into_owned();
    }
}
pub async fn read_table(path: &Path) -> Result<toml::Table> {
    toml::from_str(
        &tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("{}: expected TOML configuration", path.display()))
}
pub fn validate(settings: &Config) -> Result<()> {
    ensure!(!settings.spec.trim().is_empty(), "spec is required");
    if let Some(key) = &settings.operation_url_extension {
        ensure!(
            key.starts_with("x-") && key.len() > 2 && key.trim() == key,
            "operation_url_extension must name an x- extension"
        );
    }
    ensure!(
        settings
            .read_timeout_seconds
            .is_none_or(|v| v.is_finite() && v > 0.0 && v <= 86400.0),
        "read_timeout_seconds must be in (0, 86400]"
    );
    ensure!(
        settings.max_description_chars != Some(0),
        "max_description_chars must be positive"
    );
    for selector in settings
        .include_tools
        .iter()
        .chain(&settings.exclude_tools)
        .chain(&settings.recommended_tools)
    {
        crate::deployment::pattern(selector)?;
    }
    for (field, operations) in [
        ("include_operations", &settings.include_operations),
        ("exclude_operations", &settings.exclude_operations),
    ] {
        for operation in operations {
            let (method, path) = operation
                .split_once(' ')
                .with_context(|| format!("{field} requires METHOD path"))?;
            ensure!(
                ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method)
                    && (path.starts_with('/')
                        || path.starts_with("https://")
                        || path.starts_with("http://")),
                "invalid {field} entry"
            );
        }
    }
    ensure!(
        settings.max_response_bytes > 0
            && settings.max_help_bytes > 0
            && settings.max_spec_bytes > 0,
        "max_response_bytes, max_help_bytes and max_spec_bytes must be positive"
    );
    settings.image_limits.validate()?;
    if let Some(cover) = &settings.cover_traffic {
        cover.validate()?;
        if let Some(base) = &settings.base_url {
            cover.validate_origin(base)?;
        }
    }
    for (operation, mapping) in &settings.response_mappings {
        let (method, path) = operation
            .split_once(' ')
            .context("response_mappings requires METHOD path")?;
        ensure!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method) && path.starts_with('/'),
            "invalid response_mappings operation"
        );
        mapping.validate(&settings.image_limits)?;
    }
    if let Some(credits) = &settings.credit_pricing {
        credits.validate()?;
    }
    Ok(())
}
// Canonicalize before composition so legacy timeout overrides retain precedence.
fn normalize_timeout(table: &mut toml::Table) -> Result<()> {
    let legacy = table.remove("timeout");
    ensure!(
        legacy.is_none() || !table.contains_key("read_timeout_seconds"),
        "use only read_timeout_seconds, not both read_timeout_seconds and timeout"
    );
    if let Some(value) = legacy {
        table.insert("read_timeout_seconds".into(), value);
    }
    Ok(())
}
pub async fn resolve(mut local: toml::Table, declaring: &Path) -> Result<ResolvedProvider> {
    normalize_timeout(&mut local)?;
    let declaring = absolute(declaring)?;
    let mut inherited = toml::Table::new();
    let mut origins = BTreeMap::new();
    if let Some(extends) = local.remove("extends") {
        let relative = extends
            .as_str()
            .context("extends must be a file path string")?;
        let provider = declaring.parent().unwrap().join(relative);
        inherited = read_table(&provider).await?;
        normalize_timeout(&mut inherited)?;
        ensure!(
            !inherited.contains_key("extends"),
            "provider {}: nested extends is not allowed",
            provider.display()
        );
        // Validate inherited fields even if a local override would hide an error.
        let _: Config = toml::Value::Table(inherited.clone())
            .try_into()
            .context("invalid provider fields")?;
        for key in inherited.keys() {
            origins.insert(key.clone(), provider.display().to_string());
        }
        resolve_spec(&mut inherited, &provider);
    }
    resolve_spec(&mut local, &declaring);
    // Top-level fields replace in full; arrays and override maps never concatenate.
    for (key, value) in local {
        origins.insert(key.clone(), declaring.display().to_string());
        inherited.insert(key, value);
    }
    let settings: Config = toml::Value::Table(inherited)
        .try_into()
        .context("invalid source fields")?;
    validate(&settings)?;
    for key in serde_json::to_value(&settings)?.as_object().unwrap().keys() {
        origins
            .entry(key.clone())
            .or_insert_with(|| "default".into());
    }
    Ok(ResolvedProvider { settings, origins })
}
pub async fn load(path: &Path) -> Result<ResolvedProvider> {
    resolve(read_table(path).await?, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_probe_settings_reject_instead_of_becoming_ignored_configuration() {
        for setting in [
            "probe_pricing=false",
            "probe_ttl_seconds=60",
            "probe_concurrency=1",
            "probe_max_endpoints=1",
            "probe_methods=['GET']",
            "probe_timeout=30",
        ] {
            let table =
                toml::from_str::<toml::Table>(&format!("spec='schema.json'\n{setting}")).unwrap();
            let error = toml::Value::Table(table).try_into::<Config>().unwrap_err();
            assert!(
                error.to_string().contains("unknown field"),
                "{setting}: {error}"
            );
        }
    }

    #[test]
    fn general_timeout_and_credit_validation_do_not_depend_on_probe_code() {
        let mut cfg = Config {
            spec: "schema.json".into(),
            ..Default::default()
        };
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, 86_401.0] {
            cfg.read_timeout_seconds = Some(invalid);
            assert!(
                validate(&cfg)
                    .unwrap_err()
                    .to_string()
                    .contains("read_timeout_seconds")
            );
        }
        cfg.read_timeout_seconds = Some(90.0);
        validate(&cfg).unwrap();
        cfg.credit_pricing = Some(crate::catalog::CreditPricing {
            credit_cost_key: "tariff".into(),
            usdc_per_credit: "0".into(),
        });
        assert!(
            validate(&cfg)
                .unwrap_err()
                .to_string()
                .contains("credit_pricing")
        );
        cfg.credit_pricing.as_mut().unwrap().usdc_per_credit = "0.01".into();
        validate(&cfg).unwrap();
        let mut legacy = toml::from_str::<toml::Table>("timeout=90").unwrap();
        normalize_timeout(&mut legacy).unwrap();
        assert_eq!(legacy["read_timeout_seconds"].as_integer(), Some(90));
        assert!(!legacy.contains_key("timeout"));
        legacy.insert("timeout".into(), toml::Value::Integer(30));
        assert!(normalize_timeout(&mut legacy).is_err());
    }
}
