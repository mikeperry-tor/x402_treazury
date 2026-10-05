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
    ensure!(
        settings.timeout.is_finite() && settings.timeout > 0.0 && settings.timeout <= 86400.0,
        "timeout must be in (0, 86400]"
    );
    ensure!(
        settings.max_description_chars != Some(0),
        "max_description_chars must be positive"
    );
    for operation in &settings.include_operations {
        let (method, path) = operation
            .split_once(' ')
            .context("include_operations requires METHOD path")?;
        ensure!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method)
                && (path.starts_with('/')
                    || path.starts_with("https://")
                    || path.starts_with("http://")),
            "invalid include_operations entry"
        );
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
    crate::pricing::validate(settings)
}
pub async fn resolve(mut local: toml::Table, declaring: &Path) -> Result<ResolvedProvider> {
    let declaring = absolute(declaring)?;
    let mut inherited = toml::Table::new();
    let mut origins = BTreeMap::new();
    if let Some(extends) = local.remove("extends") {
        let relative = extends
            .as_str()
            .context("extends must be a file path string")?;
        let provider = declaring.parent().unwrap().join(relative);
        inherited = read_table(&provider).await?;
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
