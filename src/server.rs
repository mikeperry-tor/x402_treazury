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
    pub direct_swap: Option<(crate::direct_swap::Service, crate::direct_swap::Config)>,
    pub discover_on_demand: bool,
    pub discovery_search_tool_instructions_text: Option<String>,
    pub providers_search_remote_tool_instructions_text: Option<String>,
    pub catalog_server: String,
    pub name: String,
    pub host_policy: host::HostPolicy,
    pub instructions: Option<String>,
}
impl Server {
    pub fn new(
        tools: Vec<ToolSpec>,
        client: PaidClient,
        base_url: String,
        instructions: Option<String>,
    ) -> Self {
        Self::from_bindings(
            tools
                .into_iter()
                .map(|t| (t, client.clone(), base_url.clone()))
                .collect(),
            instructions,
        )
    }
    pub fn from_bindings(
        tools: Vec<(ToolSpec, PaidClient, String)>,
        instructions: Option<String>,
    ) -> Self {
        Self {
            work: Default::default(),
            host_policy: Default::default(),
            discovery: None,
            direct_swap: None,
            discover_on_demand: false,
            discovery_search_tool_instructions_text: None,
            providers_search_remote_tool_instructions_text: None,
            catalog: Arc::new(CatalogState::new(CatalogSnapshot {
                registrations: Default::default(),
                instructions: Default::default(),
                generation: 0,
                views: BTreeMap::from([("default".into(), catalog_state::bind(tools))]),
            })),
            catalog_server: "default".into(),
            name: "x402_treazury".into(),
            instructions,
        }
    }
    fn definition(bound: &catalog_state::BoundTool) -> Tool {
        let t = &bound.tool;
        Tool::new(
            t.name.clone(),
            bound.description(),
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
        let firstparty = self.direct_definitions();
        if firstparty.iter().any(|t| t.name == name) {
            return self
                .invoke_direct(name, serde_json::Value::Object(args.clone()))
                .await;
        }
        if self.discover_on_demand
            || self
                .discovery
                .as_ref()
                .is_some_and(|m| m.policy(&self.catalog_server).management())
        {
            match name {
                "x402_treazury_tools_search" => {
                    let args = serde_json::Value::Object(args.clone());
                    let result = match self
                        .discovery
                        .as_ref()
                        .filter(|m| m.enabled(&self.catalog_server))
                    {
                        Some(manager) => manager.search_tools_with_firstparty(
                            &self.catalog_server,
                            name,
                            args,
                            &firstparty,
                        )?,
                        None => crate::discovery::search_catalog_with_firstparty(
                            &self.catalog,
                            &self.catalog_server,
                            name,
                            args,
                            &firstparty,
                        )?,
                    };
                    return Ok(crate::output::ToolOutput::text(serde_json::to_string(
                        &result,
                    )?));
                }
                "x402_treazury_tool_call" => {
                    let call: crate::discovery::Call =
                        serde_json::from_value(serde_json::Value::Object(args.clone()))?;
                    if firstparty.iter().any(|t| t.name.as_ref() == call.tool_id) {
                        return self
                            .invoke_direct(&call.tool_id, serde_json::Value::Object(call.arguments))
                            .await;
                    }
                    let bound = crate::discovery::find_catalog_tool(
                        &self.catalog,
                        &self.catalog_server,
                        &call.tool_id,
                    )?;
                    return bound.invoke_output(&call.arguments).await;
                }
                _ => {}
            }
        }
        if let Some(manager) = &self.discovery
            && name.starts_with("x402_treazury_")
        {
            anyhow::ensure!(
                self.management_tools().iter().any(|t| t.name == name),
                "unknown tool"
            );
            if name == "x402_treazury_providers_search_remote" {
                return manager
                    .invoke_search_remote(&self.catalog_server, args)
                    .await;
            }
            if name == crate::discovery::source_details::NAME {
                return Ok(crate::output::ToolOutput::text(serde_json::to_string(
                    &manager
                        .invoke_source_details(&self.catalog_server, args)
                        .await?,
                )?));
            }
            if name == crate::discovery::raw::NAME {
                return manager.invoke_raw(&self.catalog_server, args).await;
            }
            if name != "x402_treazury_tool_call" {
                let output = crate::output::ToolOutput::text(serde_json::to_string(
                    &manager
                        .invoke(
                            &self.catalog_server,
                            name,
                            serde_json::Value::Object(args.clone()),
                        )
                        .await?,
                )?);
                return Ok(output);
            }
            let call: crate::discovery::Call =
                serde_json::from_value(serde_json::Value::Object(args.clone()))?;
            let bound = manager.find_tool(&self.catalog_server, &call.tool_id)?;
            return bound.invoke_output(&call.arguments).await;
        }
        let snapshot = self.catalog.read();
        catalog_state::find(&snapshot, &self.catalog_server, name)?
            .invoke_output(args)
            .await
    }
    fn direct_definitions(&self) -> Vec<Tool> {
        self.direct_swap
            .as_ref()
            .map(|(_, config)| crate::direct_swap::definitions(config))
            .unwrap_or_default()
    }
    async fn invoke_direct(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::output::ToolOutput> {
        anyhow::ensure!(
            !crate::qualification::active(),
            "qualification forbids direct swaps"
        );
        let (service, config) = self
            .direct_swap
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("unknown tool"))?;
        let result = service
            .invoke(&self.catalog_server, config, name, args)
            .await?;
        Ok(crate::output::ToolOutput::text(serde_json::to_string(
            &result,
        )?))
    }
    fn management_tools(&self) -> Vec<Tool> {
        let mut tools = crate::discovery::tools::listener_definitions(
            self.discovery
                .as_ref()
                .map(|m| m.policy(&self.catalog_server))
                .unwrap_or_default(),
            self.discover_on_demand,
            self.discovery_search_tool_instructions_text.as_deref(),
            self.providers_search_remote_tool_instructions_text
                .as_deref(),
        );
        if !self.discover_on_demand {
            tools.extend(self.direct_definitions());
        }
        tools
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        let snapshot = self.catalog.read();
        let authored = snapshot
            .instructions
            .get(&self.catalog_server)
            .unwrap_or(&self.instructions);
        let mut instructions = pricing_instructions(authored.as_deref());
        if self.discover_on_demand {
            instructions.push_str("\n\nAPI tools are discovered on demand. Use x402_treazury_tools_search with a targeted query and a limit (default 25) to read complete descriptions and input schemas, then pass the returned tool_id and arguments to x402_treazury_tool_call. Discovery and calls stay within this listener's configured tools and wallets.");
        }
        if let Some((_, config)) = &self.direct_swap {
            if config.receive {
                instructions.push_str("\n\nReceive native ZEC directly at a shielded Zcash address, or request a foreign asset payment converted into this treasury. Native receiving needs no swap provider. Keep payment instructions and operation IDs; status separates provider progress from Zcash receipt and spendability. Disclosed payment routes may receive late payments.");
            }
            if config.send {
                instructions.push_str("\n\nPreview or send native shielded Zcash (ZEC), or swap treasury ZEC to a foreign asset. Preview grants no payment authority. Send binds the exact recipient and amount; retain its operation ID and inspect status before considering a retry. Native sending needs no swap provider.");
            }
            if !config.enabled() {
                instructions.push_str("\n\nInspect saved Zcash payments with swap_status. Status separates provider progress from Zcash receipt and spendability; disclosed payment routes may receive late payments.");
            }
        }
        if self.catalog.publication_failed() {
            instructions.push_str("\n\nDeferred catalog publication has stopped. Sources listed as PENDING are unavailable until restart.");
        }
        info.instructions = Some(instructions);
        info.server_info = Implementation::new(self.name.clone(), env!("CARGO_PKG_VERSION"));
        info
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        let snapshot = self.catalog.read();
        if let Some(tool) = self.management_tools().into_iter().find(|t| t.name == name) {
            return Some(tool);
        }
        if let Some(tool) = self
            .direct_definitions()
            .into_iter()
            .find(|t| t.name == name)
        {
            return Some(tool);
        }
        catalog_state::find(&snapshot, &self.catalog_server, name)
            .ok()
            .map(Self::definition)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let snapshot = self.catalog.read();
        let management = self.management_tools();
        let api = snapshot
            .views
            .get(&self.catalog_server)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let api = if self.discover_on_demand {
            &[][..]
        } else {
            api
        };
        let count = management.len() + api.len();
        if count > 500 {
            tracing::warn!(listener = %self.catalog_server, advertised_tools = count,
                "Endpoint '{}' currently has {} registered tools advertised; consider discover_on_demand = true",
                self.catalog_server, count);
        }
        let mut meta = MetaObject::new();
        meta.insert("x402_treazury/catalog".into(), serde_json::json!({
            "generation": snapshot.generation,
            "instructions": snapshot.instructions.get(&self.catalog_server).unwrap_or(&self.instructions),
            "publication_stopped": self.catalog.publication_failed(),
        }));
        Ok(ListToolsResult {
            meta: Some(meta),
            tools: management
                .into_iter()
                .chain(api.iter().map(Self::definition))
                .collect(),
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
        // Retain accepted work through dispatch, body conversion and response
        // construction, independently of response-waiter cancellation.
        let work = crate::payment::ownership::run(&self.work, async move {
            server.call_tool_owned(request, context.id).await
        });
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
            Err(e) => {
                let mut output = crate::output::ToolOutput::text(
                    crate::payment::present_api_error(&e).unwrap_or_else(|| format!("{e:#}")),
                );
                output.observed_price = crate::output::ObservedPrice::from_error(&e);
                CallToolResult::error(output.into_content())
            }
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

pub fn http_app(server: Server, token: String) -> Result<axum::Router> {
    http_app_with_auth(server, Some(token))
}

pub fn http_app_with_auth(server: Server, token: Option<String>) -> Result<axum::Router> {
    server.host_policy.validate_auth(token.as_deref())?;
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
    Ok(axum::Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, gate)))
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

/// Stop admission on an external trigger and retain accepted calls until done.
/// Operator force signals are registered after that trigger completes. For an
/// uninterrupted first/second operator-signal handoff, use `serve_http_with_signals`.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: Server,
    token: Option<String>,
    shutdown: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    serve_http_inner(listener, server, token, shutdown, None).await
}

/// Serve with one operator-signal subscription across shutdown and drain.
pub async fn serve_http_with_signals(
    listener: tokio::net::TcpListener,
    server: Server,
    token: Option<String>,
    signals: crate::shutdown::ShutdownSignals,
) -> Result<()> {
    serve_http_inner(
        listener,
        server,
        token,
        std::future::pending(),
        Some(signals),
    )
    .await
}

async fn serve_http_inner(
    listener: tokio::net::TcpListener,
    server: Server,
    token: Option<String>,
    shutdown: impl std::future::Future<Output = std::io::Result<()>>,
    mut signals: Option<crate::shutdown::ShutdownSignals>,
) -> Result<()> {
    use anyhow::Context;
    use std::future::IntoFuture;
    let stop = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = stop.clone().drop_guard();
    let work = server.work.clone();
    let serving = axum::serve(listener, http_app_with_auth(server, token)?)
        .with_graceful_shutdown(stop.clone().cancelled_owned())
        .into_future();
    tokio::pin!(serving);
    let signal = tokio::select! {
        result = &mut serving => return Ok(result?),
        signal = async {
            match signals.as_mut() {
                Some(signals) => signals.recv().await,
                None => shutdown.await,
            }
        } => signal,
    };
    let signals = match signals {
        Some(signals) => Ok(signals),
        None => crate::shutdown::ShutdownSignals::new(),
    };
    log_http_shutdown();
    work.close();
    stop.cancel();
    // Registration errors still close admission and request graceful shutdown.
    // Accepted calls retain their independent work owner on this error path.
    let mut signals = match signals {
        Ok(signals) => signals,
        Err(error) => {
            if let Some(engine) = &crate::network::global().cover {
                engine.stop_ranges().await;
            }
            return Err(error.into());
        }
    };
    tokio::select! {
        result = async {
            if let Some(engine) = &crate::network::global().cover {
                engine.stop_ranges().await;
            }
            let (result, ()) = tokio::join!(wait_with_progress("http_transport", serving), work.drain());
            result
        } => result?,
        signal = signals.recv() => { signal?; anyhow::bail!(ForcedShutdown); }
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
    signals: &mut crate::shutdown::ShutdownSignals,
) -> Result<T> {
    tracing::info!(
        stage,
        "Draining accepted work; send another signal to force exit with unfinished liabilities retained"
    );
    tokio::select! {
        result = wait_with_progress(stage, work) => result,
        signal = signals.recv() => { signal?; anyhow::bail!(ForcedShutdown); }
    }
}

/// A second operator signal is explicit force, independent of network progress.
/// This one-shot compatibility helper creates a new subscription; owners that
/// handle successive signals should retain `ShutdownSignals` instead.
pub async fn force_signal() -> std::io::Result<()> {
    crate::shutdown::ShutdownSignals::new()?.recv().await
}

#[cfg(test)]
mod direct_swap_tests;
#[cfg(test)]
mod security_tests;

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
