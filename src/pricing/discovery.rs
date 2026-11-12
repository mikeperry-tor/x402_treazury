//! Optional advertisements owned by one published source generation.
use super::advertised::{Acquisition, NormalizedPrice, PricingConfig, PricingFormat};
use crate::{
    catalog::{Config, ToolSpec},
    network,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

mod matching;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Operation {
    method: String,
    resource: String,
}

fn resource(base: &str, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.to_owned()
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

/// Use the original inventory before POST preference and tool-name filtering.
/// Otherwise those filters could turn a methodless advertisement into authority
/// to assign a price to the wrong member of a GET/POST pair.
fn inventory(cfg: &Config, root: &Value, base: &str) -> Result<Vec<Operation>> {
    Ok(crate::catalog::operation_identities(root)?
        .into_iter()
        .map(|(method, path, original)| {
            let explicit = cfg
                .operation_url_extension
                .as_deref()
                .and_then(|key| original.get(key))
                .and_then(Value::as_str);
            Operation {
                method: method.to_ascii_uppercase(),
                resource: resource(base, explicit.unwrap_or(path)),
            }
        })
        .collect())
}

fn assignments_indexed(
    config: &PricingConfig,
    root: &Value,
    index: &matching::Index,
) -> Result<BTreeMap<usize, NormalizedPrice>> {
    ensure!(
        root["x402Version"].as_u64() == Some(2),
        "pricing document requires x402 version 2"
    );
    let key = match config.format {
        PricingFormat::X402Resources => "resources",
        PricingFormat::X402Discovery => "items",
        _ => anyhow::bail!("unsupported external pricing format"),
    };
    let records = root[key]
        .as_array()
        .context("pricing document records missing")?;
    let mut matches: BTreeMap<usize, Option<NormalizedPrice>> = BTreeMap::new();
    for record in records {
        let Some(url) = record["resource"].as_str() else {
            continue;
        };
        let method = match record.get("method") {
            None => None,
            Some(Value::String(method)) if !method.is_empty() => Some(method.as_str()),
            Some(_) => continue,
        };
        if record.get("type").is_some_and(|kind| kind != "http")
            || record
                .get("x402Version")
                .is_some_and(|version| version.as_u64() != Some(2))
        {
            continue;
        }
        if config.format == PricingFormat::X402Resources && method.is_none() {
            continue;
        }
        let candidates = index.candidates(url, method);
        if let [index] = candidates.as_slice() {
            matches
                .entry(*index)
                .and_modify(|price| *price = None)
                .or_insert_with(|| Some(NormalizedPrice::parse(config, record)));
        } else {
            tracing::debug!(
                reason = "pricing_resource_ambiguous_or_unmatched",
                matches = candidates.len(),
                "An advertised price could not be matched to one API operation; check its method and resource URL"
            );
        }
    }
    Ok(matches.into_iter().filter_map(|(index, price)| {
        if let Some(price) = price { Some((index, price)) } else {
            tracing::debug!(reason="pricing_resource_conflict", "Multiple advertised prices matched one API operation; its price remains unknown");
            None
        }
    }).collect())
}

/// Clones retain one acquisition until the last tool of this generation is gone.
/// Output clones are captured before attaching the owner, avoiding a task cycle.
fn start(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    load: impl std::future::Future<Output = Result<crate::catalog::LoadedCatalog>> + Send + 'static,
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let Ok(Some(config)) = cfg.effective_pricing() else {
        return None;
    };
    if config.source == "catalog" {
        return None;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return None;
    };
    let outputs = tools
        .iter()
        .filter(|tool| tool.help_url.is_none())
        .map(|tool| {
            (
                Operation {
                    method: tool.method.to_ascii_uppercase(),
                    resource: resource(base, tool.request_url.as_deref().unwrap_or(&tool.path)),
                },
                tool.advertised_pricing.clone(),
            )
        })
        .collect::<Vec<_>>();
    if outputs.is_empty() {
        return None;
    }
    // Every output of this acquisition retires with its source. Capture a cell
    // before attaching the task owner, so the commit fence introduces no cycle.
    let publication = outputs[0].1.clone();
    let parent_expires = cfg.catalog_pricing_expires;
    let source = cfg.discovery_source.clone().or_else(|| cfg.prefix.clone());
    let settings = cfg.clone();
    let root = root.clone();
    let base = base.to_owned();
    let task = runtime.spawn(async move {
        if let Some(ready) = ready && ready.await.is_err() {
            return;
        }
        let prepared = tokio::task::spawn_blocking(move || {
            let operations = inventory(&settings, &root, &base)?;
            let index = matching::Index::new(&operations);
            let mut targets: BTreeMap<Operation, Vec<_>> = BTreeMap::new();
            for (operation, cell) in outputs {
                targets.entry(operation).or_default().push(cell);
            }
            Ok::<_, anyhow::Error>((operations, index, targets))
        }).await;
        let Ok(Ok((operations, index, mut targets))) = prepared else {
            tracing::info!(source, reason = "pricing_inventory_unavailable",
                "Advertised prices are unavailable because API operation identities could not be resolved");
            return;
        };
        match load.await {
            Ok(mut document) => {
                let expires = parent_expires.or(document.advertised_expires).unwrap_or(crate::http_cache::INDEFINITE_EXPIRY);
                let matching_config = config.clone();
                let root = document.document.clone();
                // Blocking work owns no publication cells. Aborting this async
                // waiter cannot let a completed obsolete job publish afterwards.
                let prices = tokio::task::spawn_blocking(move || {
                    assignments_indexed(&matching_config, &root, &index)
                }).await;
                match prices {
                    Ok(Ok(prices)) => {
                        document.commit_if_current(Some(publication)).await;
                        let until = crate::http_cache::advertised_until(expires);
                        for (identity, price) in prices {
                            if let Some(cells) = targets.remove(&operations[identity]) {
                                for cell in cells {
                                    cell.replace(price.clone(), config.source.clone(), until);
                                }
                            }
                        }
                    }
                    _ => tracing::info!(source, reason="pricing_document_invalid", "Advertised prices are unavailable because the metadata format is invalid; API tools remain usable"),
                }
            }
            Err(error) => {
                let failure=crate::discovery_relay::document::SharedFailure::capture(&error,"pricing_metadata_unavailable");
                tracing::info!(source,reason="pricing_metadata_unavailable",http_status=failure.status(),authorization_may_charge=failure.possible_payment(),
                    "{}", if failure.possible_payment() { "Advertised prices could not be loaded; API tools remain usable; a relay payment authorization may still charge" } else { "Advertised prices could not be loaded; API tools remain usable with challenge-based prices" });
            }
        }
    });
    let owner = Arc::new(Acquisition::new(task.abort_handle()));
    for tool in tools.iter_mut().filter(|tool| tool.help_url.is_none()) {
        tool.advertised_pricing.attach_owner(owner.clone());
    }
    Some(task)
}

fn static_task(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    gate: Option<Arc<network::DiscoveryCapacity>>,
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let Ok(Some(pricing)) = cfg.effective_pricing() else {
        return None;
    };
    if pricing.source == "catalog" {
        return None;
    }
    let mut settings = cfg.clone();
    settings.spec = pricing.source;
    settings.catalog_cache_force_refresh |= cfg.catalog_pricing_refresh;
    // Retain the original configuration revision and runtime relay principal.
    // The explicit URL owns its cache entry and refresh timestamp.
    start(
        cfg,
        root,
        base,
        tools,
        async move { crate::catalog::load_json_pricing(&settings, network::global(), gate).await },
        ready,
    )
}

#[cfg(test)]
pub(crate) fn attach_dynamic(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    client: crate::payment::PaidClient,
) {
    let _ = dynamic_task(cfg, root, base, tools, client, None);
}

fn dynamic_task(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    client: crate::payment::PaidClient,
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let Ok(Some(pricing)) = cfg.effective_pricing() else {
        return None;
    };
    if pricing.source == "catalog" {
        return None;
    }
    let settings = cfg.clone();
    let (client, _) = client.pin_document_authority();
    start(
        cfg,
        root,
        base,
        tools,
        load_dynamic(settings, pricing, client),
        ready,
    )
}

/// Retain pricing ownership on proposed tools without starting metadata I/O.
pub(crate) fn prepare_dynamic(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    client: crate::payment::PaidClient,
) -> Option<tokio::sync::oneshot::Sender<()>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    dynamic_task(cfg, root, base, tools, client, Some(receive)).map(|_| send)
}

pub(crate) fn attach_static(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    gate: Option<Arc<network::DiscoveryCapacity>>,
) {
    let _ = static_task(cfg, root, base, tools, gate, None);
}

/// Attach ownership before publication, but defer pricing I/O until replacement
/// definitions are published and the previous generation has retired.
pub(crate) fn prepare_static(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    gate: Option<Arc<network::DiscoveryCapacity>>,
) -> Option<tokio::sync::oneshot::Sender<()>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    static_task(cfg, root, base, tools, gate, Some(receive)).map(|_| send)
}
pub(crate) async fn refresh_static(
    cfg: &Config,
    root: &Arc<Value>,
    base: &str,
    tools: &mut [ToolSpec],
    gate: Option<Arc<network::DiscoveryCapacity>>,
) {
    if let Some(task) = static_task(cfg, root, base, tools, gate, None) {
        let _ = task.await;
    }
}
async fn load_dynamic(
    settings: Config,
    pricing: PricingConfig,
    client: crate::payment::PaidClient,
) -> Result<crate::catalog::LoadedCatalog> {
    use crate::discovery_relay::document::{self, Options, RequestKind};
    let mut options = Options::new(
        &pricing.source,
        network::IsolationId::discovery(&pricing.source)?,
        settings.max_spec_bytes,
        "max_spec_bytes",
    );
    options.kind = RequestKind::Pricing;
    options.timeout = settings.read_timeout_seconds.map(Duration::from_secs_f64);
    options.transport = settings.transport();
    let document = match document::origin(&options).await {
        Ok(document) => document,
        Err(failure) => {
            let owned_client = client.clone();
            client
                .run_owned(
                    async move { document::fallback(&owned_client, &options, failure).await },
                )
                .await??
        }
    };
    let root = document.validate(|_, _, bytes| crate::catalog::parse_document(bytes))?;
    Ok(crate::catalog::LoadedCatalog {
        document: Arc::new(root),
        hints: None,
        pending: None,
        refresh_pricing: true,
        advertised_expires: Some(crate::http_cache::advertised_expiry(
            settings.catalog_cache_ttl_seconds,
            Duration::ZERO,
        )),
    })
}

#[cfg(test)]
fn assignments(
    config: &PricingConfig,
    root: &Value,
    operations: &[Operation],
) -> Result<BTreeMap<usize, NormalizedPrice>> {
    assignments_indexed(config, root, &matching::Index::new(operations))
}

#[cfg(test)]
fn template_matches(template: &str, candidate: &str) -> bool {
    !matching::Index::new(&[Operation {
        method: "GET".into(),
        resource: template.into(),
    }])
    .candidates(candidate, None)
    .is_empty()
}

#[cfg(test)]
mod tests;
