use crate::{catalog::ToolSpec, payment::PaidClient};
use anyhow::{Context, Result};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, model::*, service::RequestContext};
use serde_json::Map;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct Server {
    pub tools: Arc<Vec<ToolSpec>>,
    pub name: String,
    bindings: Arc<BTreeMap<String, (PaidClient, String)>>,
    pub instructions: Option<String>,
    pub max_response_chars: Option<usize>,
    help: Arc<BTreeMap<String, OnceCell<String>>>,
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
        let help = Arc::new(
            tools
                .iter()
                .filter_map(|(t, _, _)| t.help_url.clone())
                .map(|u| (u, OnceCell::new()))
                .collect(),
        );
        let bindings = Arc::new(
            tools
                .iter()
                .map(|(t, c, b)| (t.name.clone(), (c.clone(), b.clone())))
                .collect(),
        );
        Self {
            tools: Arc::new(tools.into_iter().map(|(t, _, _)| t).collect()),
            name: "x402-treazure".into(),
            bindings,
            instructions,
            max_response_chars,
            help,
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
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == name)
            .context("unknown tool")?;
        let (client, base_url) = &self.bindings[name];
        let text = if let Some(url) = &tool.help_url {
            self.help[url]
                .get_or_try_init(|| async {
                    Ok::<_, anyhow::Error>(
                        crate::network::discovery(url, client.timeout())?
                            .get(url)
                            .send()
                            .await?
                            .error_for_status()?
                            .text()
                            .await?,
                    )
                })
                .await?
                .clone()
        } else {
            client.execute(tool.route(base_url, args)?).await?
        };
        Ok(match self.max_response_chars {
            Some(max) if text.chars().count() > max => format!(
                "{}\n[truncated by --max-response-chars]",
                text.chars().take(max).collect::<String>()
            ),
            _ => text,
        })
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
        self.tools
            .iter()
            .find(|t| t.name == name)
            .map(Self::definition)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tools.iter().map(Self::definition).collect(),
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
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
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
        next.run(request).await
    }
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    axum::Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token, gate))
}
