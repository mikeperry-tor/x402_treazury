//! Accepted provider contracts, independent of the currently shipped bundle.
use crate::catalog::{self, Config, ToolSpec};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

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
        settings.catalog_cache_ttl_seconds = 0;
        settings.probe_pricing = false;
        settings.use_recommended_tools = false;
        settings.cover_traffic = None;
        Self {
            id: id.into(),
            settings,
        }
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
) -> Result<(Vec<ToolSpec>, Option<String>)> {
    super::import::validate_source_name(name)?;
    crate::config::validate(&provider.settings)?;
    let mut settings = provider.settings.clone();
    settings.use_recommended_tools = false;
    let canonical = settings.prefix.as_deref().unwrap_or(&provider.id);
    let mut tools = catalog::build_tools(&settings, root, canonical)?;
    let prefix = format!("{canonical}_");
    let dynamic = super::import::tool_prefix(name)?;
    let mut names = BTreeMap::from([(format!("{canonical}_*"), format!("{dynamic}*"))]);
    for tool in &tools {
        let suffix = tool
            .name
            .strip_prefix(&prefix)
            .context("provider tool prefix mismatch")?;
        names.insert(tool.name.clone(), format!("{dynamic}{suffix}"));
    }
    for tool in &mut tools {
        tool.name = names[&tool.name].clone();
        tool.description = rebind(&tool.description, &names);
    }
    let instructions = settings
        .instructions_text
        .as_ref()
        .map(|text| rebind(text, &names));
    Ok((tools, instructions))
}

// Replace known symbols in prose, never wire/schema values or URL tokens. A single
// pass avoids cascading when the registration name resembles a canonical symbol.
fn rebind(text: &str, names: &BTreeMap<String, String>) -> String {
    static SYMBOLS: OnceLock<regex::Regex> = OnceLock::new();
    let symbols = SYMBOLS.get_or_init(|| regex::Regex::new(r"[A-Za-z0-9_-]+\*?").unwrap());
    text.split_inclusive(char::is_whitespace)
        .map(|chunk| {
            if chunk.contains("://") {
                return chunk.to_owned();
            }
            symbols
                .replace_all(chunk, |capture: &regex::Captures<'_>| {
                    names
                        .get(&capture[0])
                        .cloned()
                        .unwrap_or_else(|| capture[0].to_owned())
                })
                .into_owned()
        })
        .collect()
}
