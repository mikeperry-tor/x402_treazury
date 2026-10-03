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
        if let Some(url) = &self.tool.help_url {
            Ok(self
                .help
                .get_or_try_init(|| async {
                    let response = crate::network::discovery(url, self.client.timeout())?
                        .get(url)
                        .send()
                        .await
                        .map_err(reqwest::Error::without_url)?
                        .error_for_status()
                        .map_err(reqwest::Error::without_url)?;
                    let bytes = crate::limits::read(
                        response,
                        self.client.max_help_bytes(),
                        "help document",
                        "max_help_bytes",
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(String::from_utf8_lossy(&bytes).into_owned())
                })
                .await?
                .clone())
        } else {
            self.client
                .execute(self.tool.route(&self.base, args)?)
                .await
        }
    }
}
#[derive(Clone, Default)]
pub struct CatalogSnapshot {
    pub generation: u64,
    pub views: BTreeMap<String, Vec<BoundTool>>,
}
pub struct CatalogState {
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
            snapshot: RwLock::new(Arc::new(snapshot)),
            instance: uuid::Uuid::new_v4().to_string(),
        }
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn read(&self) -> Arc<CatalogSnapshot> {
        self.snapshot.read().expect("catalog lock poisoned").clone()
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
