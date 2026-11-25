//! Accepted provider contracts, independent of the currently shipped bundle.
use crate::catalog::{self, Config};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedProvider {
    pub id: String,
    pub settings: Config,
}

impl AcceptedProvider {
    pub fn current(id: &str) -> Result<Self> {
        let provider = crate::providers::get(id).context("unknown_provider_id")?;
        Ok(Self::new(id, provider.config.clone()))
    }
    pub fn new(id: &str, mut settings: Config) -> Self {
        // Accepted bytes replace static discovery, warming and probe machinery.
        settings.http_cache_enabled = false;
        settings.use_recommended_tools = false;
        settings.cover_traffic = None;
        Self {
            id: id.into(),
            settings,
        }
    }
}

/// Select only an unambiguous full-specification match; guidance follows exposed routes.
pub(super) fn select_spec_match(
    matches: &[&crate::providers::Provider],
    local_available: bool,
) -> Result<Option<AcceptedProvider>> {
    match matches {
        [] => Ok(None),
        [provider] => Ok(Some(AcceptedProvider::new(
            provider.id,
            provider.config.clone(),
        ))),
        _ if local_available => anyhow::bail!(
            "ambiguous specification; use providers_add_local with provider_id from: {}",
            matches.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
        ),
        _ => anyhow::bail!(
            "ambiguous specification; ask the operator to resolve the bundled definition selection"
        ),
    }
}

pub(super) fn local_document(provider: &AcceptedProvider) -> Result<Vec<u8>> {
    let shipped = crate::providers::get(&provider.id).context("unknown_provider_id")?;
    anyhow::ensure!(
        shipped.config.spec == provider.settings.spec,
        "bundled_schema_reference_changed"
    );
    Ok(shipped
        .local_schema
        .context("bundled_schema_missing")?
        .as_bytes()
        .to_vec())
}

pub(super) fn build(
    provider: &AcceptedProvider,
    name: &str,
    root: &Value,
) -> Result<(catalog::CatalogBuild, Option<String>)> {
    let dynamic = super::import::tool_prefix(name)?;
    crate::config::validate(&provider.settings)?;
    let mut settings = provider.settings.clone();
    settings.use_recommended_tools = false;
    let canonical = settings.prefix.as_deref().unwrap_or(&provider.id);
    let mut built = catalog::compile_catalog(&settings, root, canonical)?;
    // Provider filters and overrides use canonical names. Publish the selected
    // tools in the registration's namespace.
    let prefix = format!("{canonical}_");
    for tool in &mut built.tools {
        let suffix = tool
            .name
            .strip_prefix(&prefix)
            .context("provider tool prefix mismatch")?;
        tool.name = format!("{dynamic}{suffix}");
    }
    built.render_descriptions();
    Ok((built, settings.instructions_text))
}
