//! Offline shipped provider metadata and exact URL lookup; no deployment or wallet work.
use crate::catalog::Config;
use anyhow::{Result, ensure};
use reqwest::Url;
use serde_json::{Value, json};
use std::sync::OnceLock;

include!("providers/assets.rs");

pub struct Provider {
    pub id: &'static str,
    pub config: Config,
    pub local_schema: Option<&'static str>,
    base_key: Option<String>,
    spec_key: Option<String>,
}

impl Provider {
    pub fn metadata(&self) -> Value {
        json!({
            "provider_id": self.id,
            "name": self.config.name.as_deref().unwrap_or(self.id),
            "description": self.config.description,
            "api_base": self.config.base_url,
            "schema": {
                "kind": if self.local_schema.is_some() { "bundled" } else { "remote" },
                "reference": self.config.spec,
            },
        })
    }
}

macro_rules! embedded_schema {
    (None) => {
        None
    };
    (Some($path:literal)) => {
        Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/",
            $path
        )))
    };
}

macro_rules! load_providers {
    ($(($id:literal, $path:literal, $kind:ident $(($local:literal))?)),* $(,)?) => {
        vec![$({
            let config: Config = toml::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)))
                .expect(concat!("valid bundled provider ", $id));
            let local_schema = embedded_schema!($kind $(($local))?);
            Provider {
                id: $id,
                base_key: config.base_url.as_deref().map(|raw| canonical_base(raw).expect("valid bundled API base")),
                spec_key: if local_schema.is_none() { Some(canonical_spec(&config.spec).expect("valid bundled specification URL")) } else { None },
                config,
                local_schema,
            }
        }),*]
    };
}

pub fn all() -> &'static [Provider] {
    static PROVIDERS: OnceLock<Vec<Provider>> = OnceLock::new();
    PROVIDERS.get_or_init(|| provider_manifest!(load_providers))
}

pub fn get(id: &str) -> Option<&'static Provider> {
    all().iter().find(|provider| provider.id == id)
}

fn https_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "provider lookup requires an HTTPS URL without credentials or fragment"
    );
    Ok(url)
}

fn canonical_base(raw: &str) -> Result<String> {
    let mut url = https_url(raw)?;
    ensure!(url.query().is_none(), "API base must not contain a query");
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string())
}

fn canonical_spec(raw: &str) -> Result<String> {
    Ok(https_url(raw)?.to_string())
}

pub fn matching_base(raw: &str) -> Result<Vec<&'static Provider>> {
    let key = canonical_base(raw)?;
    Ok(all()
        .iter()
        .filter(|provider| provider.base_key.as_ref() == Some(&key))
        .collect())
}

pub fn matching_spec(raw: &str) -> Result<Vec<&'static Provider>> {
    let key = canonical_spec(raw)?;
    Ok(all()
        .iter()
        .filter(|provider| provider.spec_key.as_ref() == Some(&key))
        .collect())
}

/// Shares browse pagination with directory tools; metadata alone grants no execution authority.
pub fn browse(q: Option<&str>, page: u64, per_page: u64) -> Result<Value> {
    browse_providers(all(), q, page, per_page)
}

fn browse_providers(
    providers: &[Provider],
    q: Option<&str>,
    page: u64,
    per_page: u64,
) -> Result<Value> {
    ensure!(page >= 1, "page must be at least 1");
    ensure!(per_page >= 1, "per_page must be at least 1");
    let query = q.unwrap_or_default().trim().to_lowercase();
    let matched: Vec<_> = providers
        .iter()
        .filter(|provider| {
            [
                Some(provider.id),
                provider.config.name.as_deref(),
                provider.config.description.as_deref(),
                provider.config.base_url.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|field| field.to_lowercase().contains(&query))
        })
        .collect();
    let total = matched.len() as u64;
    let offset = (page - 1).saturating_mul(per_page);
    let items: Vec<_> = matched
        .into_iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(per_page.min(total) as usize)
        .map(Provider::metadata)
        .collect();
    Ok(json!({
        "local_only": true, "items": items, "page": page, "per_page": per_page,
        "total": total, "next_page": if offset.saturating_add(per_page) < total { Some(page + 1) } else { None },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! check_schema_reference {
        ($config:ident, $path:literal, None) => {
            assert!(canonical_spec(&$config.spec).is_ok());
        };
        ($config:ident, $path:literal, Some($local:literal)) => {
            assert_eq!(
                std::path::Path::new($path)
                    .parent()
                    .unwrap()
                    .join(&$config.spec),
                std::path::Path::new($local),
                "bundled schema must be the file named by the declaring provider"
            );
        };
    }
    macro_rules! check_manifest_references {
        ($(($id:literal, $path:literal, $kind:ident $(($local:literal))?)),* $(,)?) => {
            $({
                let config: Config = toml::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path))).unwrap();
                check_schema_reference!(config, $path, $kind $(($local))?);
            };)*
        };
    }

    #[test]
    fn schema_manifest_references_match_declared_provider_paths() {
        provider_manifest!(check_manifest_references);
    }

    #[test]
    fn shipped_assets_are_complete_and_described_offline() {
        let mut previous = "";
        for provider in all() {
            assert!(provider.id > previous, "stable, unique ID order");
            previous = provider.id;
            crate::config::validate(&provider.config).unwrap();
            assert!(
                !provider
                    .config
                    .description
                    .as_deref()
                    .unwrap()
                    .trim()
                    .is_empty()
            );
            if let Some(schema) = provider.local_schema {
                let document = serde_json::from_str(schema).unwrap();
                crate::catalog::build_tools(
                    &provider.config,
                    &document,
                    provider.config.prefix.as_deref().unwrap_or(provider.id),
                )
                .unwrap();
            } else {
                assert!(provider.spec_key.is_some());
            }
        }
    }

    #[test]
    fn exact_normalized_lookup_preserves_ambiguity_and_spec_query() {
        assert_eq!(
            canonical_base("https://EXAMPLE.com:443/a/b/").unwrap(),
            canonical_base("https://example.com/a/b").unwrap()
        );
        assert_ne!(
            canonical_base("https://example.com/a").unwrap(),
            canonical_base("https://example.com/ab").unwrap()
        );
        assert_ne!(
            canonical_spec("https://example.com/spec?a=1").unwrap(),
            canonical_spec("https://example.com/spec?a=2").unwrap()
        );
        assert!(canonical_base("http://example.com").is_err());
        assert!(canonical_base("https://user@example.com").is_err());
        assert!(canonical_base("https://example.com/#fragment").is_err());
        assert!(matching_base("https://unknown.invalid").unwrap().is_empty());
        let candidates = matching_base("https://API.RELAYSHIELD.NET:443/").unwrap();
        assert_eq!(
            candidates.iter().map(|p| p.id).collect::<Vec<_>>(),
            ["relayshield", "relayshield-phone-reputation"]
        );
        assert_eq!(
            matching_spec("https://api.exa.ai/openapi.json").unwrap()[0].id,
            "exa"
        );
        assert!(
            matching_spec("https://api.exa.ai/openapi.json?other=1")
                .unwrap()
                .is_empty()
        );
    }

    fn synthetic(id: &'static str, name: &str, description: &str, base: &str) -> Provider {
        Provider {
            id,
            config: Config {
                name: Some(name.into()),
                description: Some(description.into()),
                base_url: Some(base.into()),
                ..Default::default()
            },
            local_schema: None,
            base_key: None,
            spec_key: None,
        }
    }

    #[test]
    fn local_search_and_pagination_use_metadata_only() {
        let entries = [
            synthetic("alpha", "First", "Satellite maps", "https://a.invalid"),
            synthetic("beta", "Second", "Registry records", "https://b.invalid"),
        ];
        for q in ["ALPHA", "first", " SATELLITE ", "a.invalid"] {
            let result = browse_providers(&entries, Some(q), 1, 1).unwrap();
            assert_eq!(result["total"], 1);
            assert_eq!(result["items"][0]["provider_id"], "alpha");
        }
        assert_eq!(
            browse_providers(&entries, None, 1, 1).unwrap(),
            browse_providers(&entries, Some("  "), 1, 1).unwrap()
        );
        let first = browse_providers(&entries, None, 1, 1).unwrap();
        assert_eq!(first["total"], 2);
        assert_eq!(first["next_page"], 2);
        let last = browse_providers(&entries, None, 2, 1).unwrap();
        assert_eq!(last["items"][0]["provider_id"], "beta");
        assert!(last["next_page"].is_null());
        assert_eq!(
            browse_providers(&entries, Some("absent"), 1, 1).unwrap()["total"],
            0
        );
        assert_eq!(
            browse_providers(&entries, None, u64::MAX, 100).unwrap()["items"],
            json!([])
        );
        assert!(browse_providers(&entries, None, 0, 1).is_err());
        for size in [101, u32::MAX as u64 + 1, u64::MAX] {
            assert_eq!(
                browse_providers(&entries, None, 1, size).unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        }
        assert!(browse_providers(&entries, None, 1, 0).is_err());
    }
}
