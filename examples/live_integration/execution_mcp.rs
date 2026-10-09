//! Local stateless MCP wire client; it never contacts a provider itself.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub struct Client {
    pub http: reqwest::Client,
    pub endpoint: String,
    pub token: String,
}
impl Client {
    async fn rpc(
        &self,
        id: &str,
        method: &str,
        params: Value,
        limit: usize,
    ) -> Result<(Vec<u8>, Value)> {
        let mut response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        ensure!(
            response.status().is_success(),
            "MCP HTTP status {}",
            response.status().as_u16()
        );
        ensure!(
            !response.headers().contains_key("mcp-session-id"),
            "qualification requires stateless MCP"
        );
        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(reqwest::Error::without_url)?
        {
            ensure!(
                chunk.len() <= limit.saturating_sub(body.len()),
                "MCP response exceeds result_bytes={limit}; evidence rejected and case remains reserved"
            );
            body.extend_from_slice(&chunk);
        }
        let body = x402_treazury::mcp_wire::response_json(&body, &content_type)?;
        let response: Value = serde_json::from_slice(&body).context("MCP response is not JSON")?;
        ensure!(
            response["jsonrpc"] == "2.0" && response["id"] == id && response.get("error").is_none(),
            "MCP response identity/protocol error; request may have executed"
        );
        let result = response
            .get("result")
            .context("MCP result missing")?
            .clone();
        Ok((body, result))
    }
    pub async fn initialize(&self) -> Result<()> {
        self.rpc("qualification-initialize","initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"treazury-live-integration","version":"1"}}),super::files::DOCUMENT_BYTES).await?;
        Ok(())
    }
    pub async fn check_inventory(&self, expected: &Value, limit: usize) -> Result<()> {
        let mut actual = BTreeMap::new();
        let mut cursor = Value::Null;
        let mut seen_cursors = BTreeSet::new();
        let mut complete = false;
        for page in 0..100 {
            let (_, result) = self
                .rpc(
                    &format!("qualification-inventory-{page}"),
                    "tools/list",
                    if cursor.is_null() {
                        json!({})
                    } else {
                        json!({"cursor":cursor})
                    },
                    limit,
                )
                .await?;
            for tool in result["tools"]
                .as_array()
                .context("MCP inventory missing tools")?
            {
                let name = tool["name"].as_str().context("MCP tool missing name")?;
                ensure!(
                    actual.len() < 10000,
                    "MCP inventory exceeds 10000-tool qualification limit"
                );
                ensure!(
                    actual
                        .insert(
                            name.to_owned(),
                            json!({"description":tool["description"],"schema":tool["inputSchema"]})
                        )
                        .is_none(),
                    "duplicate MCP tool name"
                );
            }
            cursor = result["nextCursor"].clone();
            if cursor.is_null() {
                complete = true;
                break;
            }
            ensure!(
                cursor.is_string() && seen_cursors.insert(cursor.as_str().unwrap().to_owned()),
                "invalid/repeated inventory cursor"
            );
        }
        ensure!(
            complete,
            "MCP inventory exceeds 100-page qualification limit; inventory is incomplete"
        );
        let mut prepared = BTreeMap::new();
        for tool in expected["tools"]
            .as_array()
            .context("prepared inventory missing tools")?
        {
            let name = tool["name"]
                .as_str()
                .context("prepared tool missing name")?;
            ensure!(
                prepared
                    .insert(
                        name.to_owned(),
                        json!({"description":tool["description"],"schema":tool["input_schema"]})
                    )
                    .is_none(),
                "duplicate prepared tool name"
            );
        }
        if let Some(local) = expected.get("management_tools") {
            for tool in local
                .as_array()
                .context("prepared local tools must be an array")?
            {
                let name = tool["name"]
                    .as_str()
                    .context("prepared local tool missing name")?;
                ensure!(
                    prepared
                        .insert(
                            name.to_owned(),
                            json!({"description":tool["description"],"schema":tool["inputSchema"]})
                        )
                        .is_none(),
                    "duplicate prepared tool name"
                );
            }
        }
        ensure!(
            actual == prepared,
            "served MCP inventory differs from pinned catalog"
        );
        Ok(())
    }
    pub async fn call(
        &self,
        id: &str,
        tool: &str,
        arguments: Value,
        limit: usize,
    ) -> Result<(Vec<u8>, bool)> {
        let (body, result) = self
            .rpc(
                id,
                "tools/call",
                json!({"name":tool,"arguments":arguments}),
                limit,
            )
            .await?;
        let result: rmcp::model::CallToolResult =
            serde_json::from_value(result).context("malformed MCP tool result")?;
        Ok((body, !result.is_error.unwrap_or(false)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn inventory_includes_pinned_local_tools_and_rejects_drift() {
        let local = json!({"name":"fixture_local","description":"local fixture","inputSchema":{"type":"object","properties":{}}});
        let api = json!({"name":"api_read","description":"read","inputSchema":{"type":"object","properties":{}}});
        let tools = std::sync::Arc::new(std::sync::Mutex::new(vec![api.clone(), local.clone()]));
        let state = tools.clone();
        let app =
            axum::Router::new().route(
                "/mcp",
                axum::routing::post(move |axum::Json(v): axum::Json<Value>| {
                    let tools = state.lock().unwrap().clone();
                    async move {
                        axum::Json(json!({"jsonrpc":"2.0","id":v["id"],"result":{"tools":tools}}))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let network = x402_treazury::network::NetworkContext::new(Default::default()).unwrap();
        let client = Client {
            http: network
                .discovery(&endpoint, std::time::Duration::from_secs(2))
                .unwrap(),
            endpoint,
            token: "fixture".into(),
        };
        let expected = json!({"tools":[{"name":"api_read","description":"read","input_schema":api["inputSchema"]}],"management_tools":[local]});
        client.check_inventory(&expected, 10000).await.unwrap();
        tools.lock().unwrap().pop();
        assert!(client.check_inventory(&expected, 10000).await.is_err());
        tools
            .lock()
            .unwrap()
            .push(expected["management_tools"][0].clone());
        tools.lock().unwrap()[1]["description"] = json!("changed");
        assert!(client.check_inventory(&expected, 10000).await.is_err());
        tools.lock().unwrap()[1] = expected["management_tools"][0].clone();
        tools
            .lock()
            .unwrap()
            .push(json!({"name":"unexpected","description":"extra","inputSchema":{}}));
        assert!(client.check_inventory(&expected, 10000).await.is_err());
        server.abort();
    }
}
