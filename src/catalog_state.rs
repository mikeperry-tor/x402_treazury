//! One immutable generation binds definitions, routes and payers for every listener.
#[cfg(test)]
mod operation_url_tests;

use crate::discovery_relay::document::{self, SharedFailure};
use crate::{
    catalog::ToolSpec,
    payment::{Authority, PaidClient},
};
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
    pub help: Arc<HelpBinding>,
}
/// Documentation pins its financial authority without freezing ordinary API calls.
pub struct HelpBinding {
    client: PaidClient,
    authority: Authority,
    allowed_origins: Vec<String>,
    result: OnceCell<HelpResult>,
}
struct HelpResult {
    result: Result<String, SharedFailure>,
    possible: bool,
}
type HelpKey = (
    Authority,
    usize,
    crate::network::HttpPolicy,
    std::time::Duration,
    (bool, Option<usize>),
    Vec<String>,
);
impl HelpBinding {
    pub fn new(client: PaidClient) -> Self {
        Self::with_origins(client, Vec::new())
    }
    pub(crate) fn with_origins(client: PaidClient, mut allowed_origins: Vec<String>) -> Self {
        let (client, authority) = client.pin_document_authority();
        allowed_origins.sort();
        allowed_origins.dedup();
        Self {
            client,
            authority,
            allowed_origins,
            result: OnceCell::new(),
        }
    }
    fn key(&self) -> HelpKey {
        (
            self.authority.clone(),
            self.client.max_help_bytes(),
            self.client.transport(),
            crate::network::global().read_timeout(self.client.timeout()),
            self.client.document_policy_key(),
            self.allowed_origins.clone(),
        )
    }
    pub(crate) fn compatible(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
    pub(crate) fn get(&self) -> Option<&Result<String, SharedFailure>> {
        self.result.get().map(|help| &help.result)
    }
    #[cfg(test)]
    pub(crate) fn set(&self, text: String) -> Result<(), String> {
        self.result
            .set(HelpResult {
                result: Ok(text),
                possible: false,
            })
            .map_err(|error| {
                let value = match error {
                    tokio::sync::SetError::AlreadyInitializedError(value)
                    | tokio::sync::SetError::InitializingError(value) => value,
                };
                value.result.expect("set inserts successful fixture text")
            })
    }

    async fn invoke(&self, client: &PaidClient, url: &str) -> Result<String> {
        let already_cached = self.get().is_some();
        let started = crate::qualification::record_help(true, None, None).await;
        match (self.result.get(), started) {
            (Some(help), Err(observation)) => {
                if let Err(failure) = &help.result {
                    return Err(failure.error());
                }
                return document::with_payment_context(help.possible, Err(observation));
            }
            (_, started) => started?,
        }
        let fetched = std::sync::atomic::AtomicBool::new(false);
        let result = self
            .result
            .get_or_try_init(|| async {
                fetched.store(true, std::sync::atomic::Ordering::Relaxed);
                let result = self.fetch(client, url).await;
                match result {
                    Ok((text, possible)) => Ok(HelpResult {
                        result: Ok(text),
                        possible,
                    }),
                    Err(error) => {
                        let failure = SharedFailure::capture(&error, "help document unavailable");
                        if failure.possible_payment() {
                            Ok(HelpResult {
                                result: Err(failure),
                                possible: true,
                            })
                        } else {
                            Err(error)
                        }
                    }
                }
            })
            .await;
        let (result, possible) = match result {
            Ok(help) => (
                help.result.clone().map_err(|failure| failure.error()),
                help.possible,
            ),
            Err(error) => (Err(error), false),
        };
        let cache = if fetched.load(std::sync::atomic::Ordering::Relaxed) {
            crate::qualification::HelpCache::Fetch
        } else if already_cached {
            crate::qualification::HelpCache::Hit
        } else {
            crate::qualification::HelpCache::Shared
        };
        let completion = crate::qualification::record_help(false, Some(cache), Some(&result)).await;
        // A secondary observation failure must not replace the original paid
        // failure. Successful fetched/cached text retains its fee evidence too.
        let text = result?;
        document::with_payment_context(possible, completion.map(|()| text))
    }
    async fn fetch(&self, client: &PaidClient, url: &str) -> Result<(String, bool)> {
        let mut options = document::Options::new(
            url,
            crate::network::IsolationId::discovery(url)?,
            self.client.max_help_bytes(),
            "max_help_bytes",
        );
        options.public_only = self.client.document_policy_key().0;
        options.allowed_origins = self.allowed_origins.clone();
        options.transport = self.client.transport();
        options.timeout = self.client.timeout();
        let document = document::read(client, &options).await?;
        let text = document.validate(|status, _, bytes| {
            anyhow::ensure!(
                status.is_success(),
                "help document HTTP status {}",
                status.as_u16()
            );
            String::from_utf8(bytes.to_vec())
                .inspect_err(|_| {
                    tracing::warn!(
                        category = "help_invalid_utf8",
                        http_status = status.as_u16(),
                        authorization_may_charge = document.payment_may_have_occurred,
                        "Help document is not valid UTF-8 text; tool call failed and no partial content was returned"
                    );
                })
                .context("help document is not valid UTF-8 text")
        })?;
        Ok((text, document.payment_may_have_occurred))
    }
}
impl BoundTool {
    pub async fn invoke(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<String> {
        self.invoke_output(args).await?.into_text()
    }
    #[tracing::instrument(level = "warn", name = "tool_call", skip_all, fields(tool = %self.tool.name, source = self.source.as_ref().map(|(id, _)| id.as_str())))]
    pub async fn invoke_output(
        &self,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<crate::output::ToolOutput> {
        if let Some(url) = &self.tool.help_url {
            let help = self.help.clone();
            let url = url.clone();
            let client = match &self.source {
                Some((source, _)) => help.client.clone().with_source_identity(source),
                None => help.client.clone(),
            };
            let owner = client.clone();
            owner
                .run_owned(async move {
                    let text = help.invoke(&client, &url).await?;
                    let mut output = crate::output::ToolOutput::text(text);
                    output.payment_may_have_occurred =
                        help.result.get().is_some_and(|result| result.possible);
                    Ok(output)
                })
                .await?
        } else {
            let client = match &self.source {
                Some((source, _)) => self.client.clone().with_source_identity(source),
                None => self.client.clone(),
            };
            let client = if self.tool.request_url.is_some() {
                client.public_destinations()
            } else {
                client
            };
            client
                .execute_rendered_scoped(
                    self.tool.route(&self.base, args)?,
                    client.operation_key(
                        self.source
                            .as_ref()
                            .map(|(id, revision)| (id.as_str(), *revision)),
                        &self.tool.method,
                        &self.tool.path,
                        self.tool.request_url.as_deref().unwrap_or(&self.base),
                    ),
                    self.tool.response_mapping.as_ref(),
                    &self.tool.image_limits,
                )
                .await
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
    // Authority orders only its retained Arc identity, never mutable pool state.
    #[allow(clippy::mutable_key_type)]
    let mut help = BTreeMap::new();
    tools
        .into_iter()
        .map(|(tool, client, base)| {
            let binding = HelpBinding::new(client.clone());
            let cell = help
                .entry((tool.help_url.clone().unwrap_or_default(), binding.key()))
                .or_insert_with(|| Arc::new(binding))
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

#[cfg(test)]
#[path = "catalog_state/help_tests.rs"]
mod help_tests;
