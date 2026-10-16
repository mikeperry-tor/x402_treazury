//! One immutable generation binds definitions, routes and payers for every listener.
use crate::{catalog::ToolSpec, payment::PaidClient};
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use tokio::sync::OnceCell;
#[derive(Clone)]
pub struct BoundTool {
    pub tool: ToolSpec,
    pub client: PaidClient,
    pub base: String,
    pub source: Option<(String, u64)>,
    pub help: Arc<OnceCell<String>>,
}
impl BoundTool {
    pub async fn invoke(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.invoke_output(args).await?.into_text()
    }
    pub async fn invoke_output(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<crate::output::ToolOutput> {
        if let Some(url) = &self.tool.help_url {
            let already_cached = self.help.get().is_some();
            crate::qualification::record_help(true, None, None).await?;
            let fetched = std::sync::atomic::AtomicBool::new(false);
            let result = self
                .help
                .get_or_try_init(|| async {
                    fetched.store(true, std::sync::atomic::Ordering::Relaxed);
                    let response = crate::network::provider_discovery(
                        url,
                        self.client.timeout(),
                        self.client.transport(),
                    )?
                    .get(url)
                    .send()
                    .await
                    .map_err(reqwest::Error::without_url)?
                    .error_for_status()
                    .map_err(reqwest::Error::without_url)?;
                    crate::network::log_http(&response, "help");
                    let bytes = crate::limits::read(
                        response,
                        self.client.max_help_bytes(),
                        "help document",
                        "max_help_bytes",
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(String::from_utf8_lossy(&bytes).into_owned())
                })
                .await
                .cloned();
            let cache = if fetched.load(std::sync::atomic::Ordering::Relaxed) {
                crate::qualification::HelpCache::Fetch
            } else if already_cached {
                crate::qualification::HelpCache::Hit
            } else {
                crate::qualification::HelpCache::Shared
            };
            crate::qualification::record_help(false, Some(cache), Some(&result)).await?;
            Ok(crate::output::ToolOutput::text(result?))
        } else {
            self.client
                .execute_response_scoped(
                    self.tool.route(&self.base, args)?,
                    &crate::payment::SubmissionProgress::default(),
                    self.client.operation_key(
                        self.source
                            .as_ref()
                            .map(|(id, revision)| (id.as_str(), *revision)),
                        &self.tool.method,
                        &self.tool.path,
                        &self.base,
                    ),
                )
                .await?
                .render(self.tool.response_mapping.as_ref(), &self.tool.image_limits)
        }
    }
}
#[derive(Clone, Default)]
pub struct CatalogSnapshot {
    pub generation: u64,
    pub instructions: BTreeMap<String, Option<String>>,
    pub views: BTreeMap<String, Vec<BoundTool>>,
}
pub struct CatalogState {
    publication_failed: std::sync::atomic::AtomicBool,
    snapshot: RwLock<Arc<CatalogSnapshot>>,
    instance: String,
}
impl Default for CatalogState {
    fn default() -> Self {
        Self::new(CatalogSnapshot::default())
    }
}
impl CatalogState {
    pub fn new(snapshot: CatalogSnapshot) -> Self {
        Self {
            publication_failed: Default::default(),
            snapshot: RwLock::new(Arc::new(snapshot)),
            instance: uuid::Uuid::new_v4().to_string(),
        }
    }
    pub(crate) fn mark_publication_failed(&self) {
        self.publication_failed
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn publication_failed(&self) -> bool {
        self.publication_failed
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn read(&self) -> Arc<CatalogSnapshot> {
        self.snapshot.read().expect("catalog lock poisoned").clone()
    }
    pub(crate) fn replace_static(
        &self,
        expected: u64,
        mut snapshot: CatalogSnapshot,
    ) -> Result<u64> {
        let mut current = self.snapshot.write().expect("catalog lock poisoned");
        anyhow::ensure!(current.generation == expected, "static_revision_changed");
        anyhow::ensure!(
            current.views.keys().eq(snapshot.views.keys()),
            "static_listener_views_changed"
        );
        snapshot.generation = expected.checked_add(1).context("generation exhausted")?;
        let revision = snapshot.generation;
        *current = Arc::new(snapshot);
        Ok(revision)
    }
    pub fn publish(&self, snapshot: CatalogSnapshot) {
        *self.snapshot.write().expect("catalog lock poisoned") = Arc::new(snapshot);
    }
}
pub fn bind(tools: Vec<(ToolSpec, PaidClient, String)>) -> Vec<BoundTool> {
    let mut help = BTreeMap::new();
    tools
        .into_iter()
        .map(|(tool, client, base)| {
            let cell = help
                .entry((
                    tool.help_url.clone().unwrap_or_default(),
                    client.max_help_bytes(),
                    client.transport(),
                    crate::network::global().read_timeout(client.timeout()),
                ))
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone();
            BoundTool {
                tool,
                client,
                base,
                source: None,
                help: cell,
            }
        })
        .collect()
}
pub fn find<'a>(snapshot: &'a CatalogSnapshot, server: &str, name: &str) -> Result<&'a BoundTool> {
    snapshot
        .views
        .get(server)
        .and_then(|v| v.iter().find(|t| t.tool.name == name))
        .context("unknown tool")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{Config, build_tools},
        network::HttpPolicy,
    };
    use serde_json::json;
    use std::time::Duration;

    fn help(url: &str) -> ToolSpec {
        build_tools(
            &Config {
                help_url: Some(url.into()),
                ..Default::default()
            },
            &json!({"paths":{"/unused":{"get":{}}}}),
            "test",
        )
        .unwrap()
        .into_iter()
        .find(|t| t.help_url.is_some())
        .unwrap()
    }

    #[test]
    fn help_sharing_requires_equal_effective_read_and_transport_policies() {
        let tool = help("https://example.com/help");
        let default = PaidClient::unsigned();
        let effective = crate::network::global().read_timeout(None);
        let clients = vec![
            default.clone(),
            default.clone().with_timeout(effective),
            default
                .clone()
                .with_timeout(effective + Duration::from_secs(1)),
            default.clone().with_transport(HttpPolicy {
                allow_http1: true,
                allow_tls12: false,
            }),
            default.clone().with_transport(HttpPolicy {
                allow_http1: false,
                allow_tls12: true,
            }),
            default.clone().with_transport(HttpPolicy {
                allow_http1: true,
                allow_tls12: true,
            }),
            default.with_download_limits(1024, 1),
        ];
        let bound = bind(
            clients
                .into_iter()
                .map(|c| (tool.clone(), c, String::new()))
                .collect(),
        );
        for i in 0..bound.len() {
            for j in 0..bound.len() {
                assert_eq!(
                    Arc::ptr_eq(&bound[i].help, &bound[j].help),
                    i == j || i < 2 && j < 2,
                    "bindings {i}, {j}"
                );
            }
        }
    }

    #[tokio::test]
    async fn cached_and_inflight_help_cannot_bypass_a_shorter_read_policy() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/help", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/help",
                    axum::routing::get(move || {
                        let seen = seen.clone();
                        async move {
                            seen.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            "documentation"
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let tool = help(&url);
        let bound = bind(vec![
            (
                tool.clone(),
                PaidClient::unsigned().with_timeout(Duration::from_secs(2)),
                String::new(),
            ),
            (
                tool,
                PaidClient::unsigned().with_timeout(Duration::from_millis(100)),
                String::new(),
            ),
        ]);
        let args = Default::default();
        let (long, short) = tokio::join!(bound[0].invoke(&args), bound[1].invoke(&args));
        assert_eq!(long.unwrap(), "documentation");
        assert!(short.is_err());
        assert!(bound[1].help.get().is_none());
        assert!(bound[1].invoke(&args).await.is_err());
        assert_eq!(bound[0].invoke(&args).await.unwrap(), "documentation");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        server.abort();
        let _ = server.await;
    }
}
