use crate::catalog_state::{self, CatalogSnapshot, CatalogState};
use crate::{catalog::ToolSpec, payment::PaidClient};
use anyhow::Result;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, model::*, service::RequestContext};
use serde_json::Map;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct Server {
    pub catalog: Arc<CatalogState>,
    pub discovery: Option<Arc<crate::discovery::Manager>>,
    pub catalog_server: String,
    pub name: String,
    pub instructions: Option<String>,
    pub max_response_chars: Option<usize>,
}
impl Server {
    pub fn new(
        tools: Vec<ToolSpec>,
        client: PaidClient,
        base_url: String,
        instructions: Option<String>,
        max_response_chars: Option<usize>,
    ) -> Self {
        Self::from_bindings(
            tools
                .into_iter()
                .map(|t| (t, client.clone(), base_url.clone()))
                .collect(),
            instructions,
            max_response_chars,
        )
    }
    pub fn from_bindings(
        tools: Vec<(ToolSpec, PaidClient, String)>,
        instructions: Option<String>,
        max_response_chars: Option<usize>,
    ) -> Self {
        Self {
            discovery: None,
            catalog: Arc::new(CatalogState::new(CatalogSnapshot {
                generation: 0,
                views: BTreeMap::from([("default".into(), catalog_state::bind(tools))]),
            })),
            catalog_server: "default".into(),
            name: "x402-treazure".into(),
            instructions,
            max_response_chars,
        }
    }
    fn definition(t: &ToolSpec) -> Tool {
        Tool::new(
            t.name.clone(),
            t.description.clone(),
            Arc::new(t.input_schema.as_object().unwrap().clone()),
        )
    }
    pub async fn invoke(
        &self,
        name: &str,
        args: &Map<String, serde_json::Value>,
    ) -> Result<String> {
        if let Some(manager) = &self.discovery
            && name.starts_with("treazure_")
        {
            anyhow::ensure!(
                self.management_tools().iter().any(|t| t.name == name),
                "unknown tool"
            );
            if name != "treazure_tool_call" {
                return Ok(serde_json::to_string(
                    &manager
                        .invoke(
                            &self.catalog_server,
                            name,
                            serde_json::Value::Object(args.clone()),
                        )
                        .await?,
                )?);
            }
            let call: crate::discovery::Call =
                serde_json::from_value(serde_json::Value::Object(args.clone()))?;
            let snapshot = self.catalog.read();
            let bound = catalog_state::find(&snapshot, &self.catalog_server, &call.tool_id)?;
            anyhow::ensure!(
                bound.source.as_ref().map_or(0, |s| s.1) == call.expected_revision,
                "source_revision_conflict"
            );
            return Ok(self.limit(bound.invoke(&call.arguments).await?));
        }
        let snapshot = self.catalog.read();
        let text = catalog_state::find(&snapshot, &self.catalog_server, name)?
            .invoke(args)
            .await?;
        Ok(self.limit(text))
    }
    fn management_tools(&self) -> Vec<Tool> {
        self.discovery
            .as_ref()
            .map(|m| {
                crate::discovery::tools::definitions(
                    m.enabled(&self.catalog_server),
                    m.accepts(&self.catalog_server),
                )
            })
            .unwrap_or_default()
    }
    fn limit(&self, text: String) -> String {
        match self.max_response_chars {
            Some(max) if text.chars().count() > max => {
                tracing::warn!(
                    limit_chars = max,
                    "tool output truncated by max_response_chars"
                );
                format!(
                    "{}\n[truncated by --max-response-chars]",
                    text.chars().take(max).collect::<String>()
                )
            }
            _ => text,
        }
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.instructions = self.instructions.clone();
        info.server_info = Implementation::new(self.name.clone(), env!("CARGO_PKG_VERSION"));
        info
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        if let Some(tool) = self.management_tools().into_iter().find(|t| t.name == name) {
            return Some(tool);
        }
        let snapshot = self.catalog.read();
        catalog_state::find(&snapshot, &self.catalog_server, name)
            .ok()
            .map(|t| Self::definition(&t.tool))
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let snapshot = self.catalog.read();
        let mut tools = self.management_tools();
        tools.extend(
            snapshot
                .views
                .get(&self.catalog_server)
                .into_iter()
                .flatten()
                .map(|t| Self::definition(&t.tool)),
        );
        let dynamic = self
            .discovery
            .as_ref()
            .is_some_and(|m| m.accepts(&self.catalog_server));
        let prefix = format!(
            "{}:{}:{}:",
            self.catalog.instance(),
            self.catalog_server,
            snapshot.generation
        );
        let offset = if let Some(cursor) = request.and_then(|r| r.cursor) {
            match cursor
                .strip_prefix(&prefix)
                .and_then(|o| o.parse::<usize>().ok())
            {
                Some(o) if dynamic && o <= tools.len() => o,
                _ => {
                    return Err(McpError::invalid_params(
                        "stale or invalid cursor: restart listing",
                        None,
                    ));
                }
            }
        } else {
            0
        };
        let end = if dynamic {
            offset.saturating_add(100).min(tools.len())
        } else {
            tools.len()
        };
        Ok(ListToolsResult {
            next_cursor: (end < tools.len()).then(|| format!("{prefix}{end}")),
            tools: tools[offset..end].to_vec(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let result = match self
            .invoke(&request.name, &request.arguments.unwrap_or_default())
            .await
        {
            Ok(text) => {
                let mut result = CallToolResult::success(vec![ContentBlock::text(text.clone())]);
                if request.name.starts_with("treazure_") && request.name != "treazure_tool_call" {
                    result.structured_content = serde_json::from_str(&text).ok();
                }
                result
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))]),
        };
        Ok(result.into())
    }
}

pub fn http_app(server: Server, token: String) -> axum::Router {
    use axum::{
        extract::{Request, State},
        http::{StatusCode, header},
        middleware::{self, Next},
        response::{IntoResponse, Response},
    };
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use subtle::ConstantTimeEq;
    async fn gate(State(token): State<String>, request: Request, next: Next) -> Response {
        let expected = format!("Bearer {token}");
        let actual = request
            .headers()
            .get(header::AUTHORIZATION)
            .map(|v| v.as_bytes())
            .unwrap_or_default();
        if !bool::from(expected.as_bytes().ct_eq(actual)) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
            )
                .into_response();
        }
        let response = next.run(request).await;
        if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
            tracing::warn!(
                limit_bytes = crate::limits::MCP_REQUEST_BYTES,
                "MCP request rejected: incoming body exceeds the request byte limit"
            );
        }
        response
    }
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(crate::limits::MCP_REQUEST_BYTES);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    axum::Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token, gate))
}

// Shared contract for standalone and deployment HTTP listeners.
pub(crate) const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
pub(crate) const SHUTDOWN_TIMEOUT_MESSAGE: &str =
    "shutdown deadline exceeded; pending paid calls may have unknown outcomes";

pub(crate) fn log_http_shutdown() {
    tracing::warn!(
        "Shutting down: draining in-flight requests for up to 10 seconds to let pending payments finish safely. Please wait."
    );
}

/// Serve standalone HTTP with the same signal-triggered drain deadline as deployments.
/// The executable drops its runtime on return, cancelling any unfinished handlers.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: Server,
    token: String,
    shutdown: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use anyhow::Context;
    use std::future::IntoFuture;
    let stop = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = stop.clone().drop_guard();
    let serving = axum::serve(listener, http_app(server, token))
        .with_graceful_shutdown(stop.clone().cancelled_owned())
        .into_future();
    tokio::pin!(serving);
    let signal = tokio::select! {
        result = &mut serving => return Ok(result?),
        signal = shutdown => signal,
    };
    log_http_shutdown();
    stop.cancel();
    tokio::time::timeout(SHUTDOWN_TIMEOUT, serving)
        .await
        .context(SHUTDOWN_TIMEOUT_MESSAGE)??;
    signal.context("shutdown signal handler failed")?;
    Ok(())
}
