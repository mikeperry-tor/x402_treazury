use crate::catalog_state::{self, CatalogSnapshot, CatalogState};
use crate::{catalog::ToolSpec, payment::PaidClient};
use anyhow::Result;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, model::*, service::RequestContext};
use serde_json::Map;
use std::{collections::BTreeMap, sync::Arc};
mod auth;
pub mod host;
pub mod work;

const PRICING_GUIDANCE: &str = "Prices are estimates or sampled payment offers, not guaranteed quotes. Costs may vary with arguments; metered charges may be below the displayed maximum. Unknown does not mean free.";

fn pricing_instructions(authored: Option<&str>) -> String {
    match authored.filter(|text| !text.is_empty()) {
        Some(text) if text.contains(PRICING_GUIDANCE) => text.to_owned(),
        Some(text) => format!("{text}\n\n{PRICING_GUIDANCE}"),
        None => PRICING_GUIDANCE.to_owned(),
    }
}

#[derive(Clone)]
pub struct Server {
    pub work: work::Work,
    pub catalog: Arc<CatalogState>,
    pub discovery: Option<Arc<crate::discovery::Manager>>,
    pub catalog_server: String,
    pub name: String,
    pub host_policy: host::HostPolicy,
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
            work: Default::default(),
            host_policy: Default::default(),
            discovery: None,
            catalog: Arc::new(CatalogState::new(CatalogSnapshot {
                generation: 0,
                views: BTreeMap::from([("default".into(), catalog_state::bind(tools))]),
            })),
            catalog_server: "default".into(),
            name: "x402_treazury".into(),
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
        self.invoke_output(name, args).await?.into_text()
    }
    pub async fn invoke_output(
        &self,
        name: &str,
        args: &Map<String, serde_json::Value>,
    ) -> Result<crate::output::ToolOutput> {
        if let Some(manager) = &self.discovery
            && name.starts_with("x402_treazury_")
        {
            anyhow::ensure!(
                self.management_tools().iter().any(|t| t.name == name),
                "unknown tool"
            );
            if matches!(
                name,
                "x402_treazury_sources_search" | "x402_treazury_source_details"
            ) {
                return Ok(self.limit(
                    manager
                        .invoke_directory(&self.catalog_server, name, args)
                        .await?,
                ));
            }
            if name != "x402_treazury_tool_call" {
                return Ok(crate::output::ToolOutput::text(serde_json::to_string(
                    &manager
                        .invoke(
                            &self.catalog_server,
                            name,
                            serde_json::Value::Object(args.clone()),
                        )
                        .await?,
                )?));
            }
            let call: crate::discovery::Call =
                serde_json::from_value(serde_json::Value::Object(args.clone()))?;
            let bound = manager.find_reference(&self.catalog_server, &call.tool_ref)?;
            return Ok(self.limit(bound.invoke_output(&call.arguments).await?));
        }
        let snapshot = self.catalog.read();
        let text = catalog_state::find(&snapshot, &self.catalog_server, name)?
            .invoke_output(args)
            .await?;
        Ok(self.limit(text))
    }
    fn management_tools(&self) -> Vec<Tool> {
        self.discovery
            .as_ref()
            .map(|m| crate::discovery::tools::definitions(m.enabled(&self.catalog_server)))
            .unwrap_or_default()
    }
    fn limit(&self, mut output: crate::output::ToolOutput) -> crate::output::ToolOutput {
        let text = output.text;
        output.text = match self.max_response_chars {
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
        };
        output
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.instructions = Some(pricing_instructions(self.instructions.as_deref()));
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
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let server = self.clone();
        let started = std::time::Instant::now();
        let work = self
            .work
            .run(async move { server.call_tool_owned(request, context.id).await });
        tokio::pin!(work);
        let result = tokio::select! {
            result = &mut work => result,
            _ = context.ct.cancelled() => {
                tracing::warn!(listener = %self.catalog_server,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    category = "mcp_response_cancelled",
                    "MCP request cancelled or response channel closed; accepted tool work continues; do not retry uncertain paid work");
                let result = work.await;
                tracing::info!(listener = %self.catalog_server,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    category = "mcp_work_finished_after_cancellation",
                    "Accepted tool work finished after response cancellation; result delivery is not confirmed");
                result
            }
        };
        result.map_err(|error| McpError::internal_error(error.to_string(), None))?
    }
}

impl Server {
    async fn call_tool_owned(
        &self,
        request: CallToolRequestParams,
        id: RequestId,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = request.arguments.unwrap_or_default();
        let claim = crate::qualification::claim(&self.name, &request.name, &arguments, &id)
            .await
            .map_err(|error| {
                tracing::warn!("qualification application admission refused: {error:#}");
                McpError::invalid_request(
                    format!("qualification admission refused: {error:#}"),
                    None,
                )
            })?;
        let call = self.invoke_output(&request.name, &arguments);
        let invocation = match &claim {
            Some(claim) => claim.invoke(call).await,
            None => call.await,
        };
        let failure = invocation
            .as_ref()
            .err()
            .map(crate::qualification::failure_category);
        let result = match invocation {
            Ok(output) => {
                let structured = if request.name.starts_with("x402_treazury_")
                    && request.name != "x402_treazury_tool_call"
                {
                    serde_json::from_str(&output.text).ok()
                } else {
                    None
                };
                let mut result = CallToolResult::success(output.into_content());
                result.structured_content = structured;
                result
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))]),
        };
        if let Some(claim) = claim {
            claim
                .finish(result.is_error.unwrap_or(false), failure)
                .await
                .map_err(|error| {
                    tracing::error!("qualification application evidence incomplete: {error:#}");
                    McpError::internal_error(
                        "qualification application evidence incomplete; case remains reserved",
                        None,
                    )
                })?;
        }
        Ok(result.into())
    }
}

pub fn http_app(server: Server, token: String) -> axum::Router {
    http_app_with_auth(server, Some(token))
}

pub fn http_app_with_auth(server: Server, token: Option<String>) -> axum::Router {
    use axum::{
        extract::{Request, State},
        http::{StatusCode, header},
        middleware::{self, Next},
        response::{IntoResponse, Response},
    };
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    async fn gate(
        State(auth): State<Option<Arc<auth::Gate>>>,
        request: Request,
        next: Next,
    ) -> Response {
        if let Some(auth) = auth
            && let Err(failure) = auth.check(request.headers())
        {
            auth.warn(failure);
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                failure.message(),
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
    if token.is_none() {
        tracing::warn!(listener = %server.catalog_server, category = "http_auth_disabled",
            "HTTP authentication is disabled for this listener; clients can call its tools without a bearer token");
    }
    let auth = token.map(|token| Arc::new(auth::Gate::new(token, server.catalog_server.clone())));
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(false)
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(2)))
        .with_max_request_body_bytes(crate::limits::MCP_REQUEST_BYTES);
    let config = server.host_policy.apply(config, &server.catalog_server);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    axum::Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, gate))
}

pub const FORCE_SHUTDOWN_MESSAGE: &str = "forced shutdown; unfinished paid operations may have unknown outcomes; reservations remain for recovery";

/// Only explicit operator force requests may bypass process teardown. Library
/// callers receive this marker; the executable exits without joining workers.
#[derive(Debug)]
pub struct ForcedShutdown;
impl std::fmt::Display for ForcedShutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(FORCE_SHUTDOWN_MESSAGE)
    }
}
impl std::error::Error for ForcedShutdown {}

pub(crate) fn log_http_shutdown() {
    tracing::info!(
        "Shutting down: draining accepted work so pending payments finish safely. Please wait. Send a second signal to force exit."
    );
}

/// Stop admission and retain accepted calls until their work finishes.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: Server,
    token: Option<String>,
    shutdown: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use anyhow::Context;
    use std::future::IntoFuture;
    let stop = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = stop.clone().drop_guard();
    let work = server.work.clone();
    let serving = axum::serve(listener, http_app_with_auth(server, token))
        .with_graceful_shutdown(stop.clone().cancelled_owned())
        .into_future();
    tokio::pin!(serving);
    let signal = tokio::select! {
        result = &mut serving => return Ok(result?),
        signal = shutdown => signal,
    };
    log_http_shutdown();
    work.close();
    stop.cancel();
    if let Some(engine) = &crate::network::global().cover {
        engine.stop_ranges().await;
    }
    tokio::select! {
        result = async {
            let (result, ()) = tokio::join!(wait_with_progress("http_transport", serving), work.drain());
            result
        } => result?,
        signal = force_signal() => { signal?; anyhow::bail!(ForcedShutdown); }
    }
    if let Some(engine) = &crate::network::global().cover {
        engine.emit_summary();
    }
    signal.context("shutdown signal handler failed")?;
    Ok(())
}

pub(crate) async fn wait_with_progress<T>(
    stage: &'static str,
    future: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(future);
    let started = std::time::Instant::now();
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                tracing::info!(stage, elapsed_seconds = started.elapsed().as_secs(), "Waiting for shutdown drain");
            }
        }
    }
}

/// Drain accepted work after shutdown; an additional operator signal is force.
/// The executable handles this marker before joining blocking workers.
pub(crate) async fn drain_or_force<T>(
    stage: &'static str,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tracing::info!(
        stage,
        "Draining accepted work; send another signal to force exit with unfinished liabilities retained"
    );
    tokio::select! {
        result = wait_with_progress(stage, work) => result,
        signal = force_signal() => { signal?; anyhow::bail!(ForcedShutdown); }
    }
}

/// A second operator signal is explicit force, independent of network progress.
pub async fn force_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(test)]
mod pricing_instruction_tests {
    use super::*;
    #[test]
    fn shared_caveat_preserves_authored_instructions_and_is_added_once() {
        assert_eq!(pricing_instructions(None), PRICING_GUIDANCE);
        assert_eq!(pricing_instructions(Some("")), PRICING_GUIDANCE);
        let text = pricing_instructions(Some("Read help first."));
        assert!(text.starts_with("Read help first.\n\n"));
        assert_eq!(text.matches(PRICING_GUIDANCE).count(), 1);
        assert_eq!(pricing_instructions(Some(&text)), text);
    }
}
