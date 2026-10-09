//! Read a single stateless MCP response, with JSON or SSE framing.
//! Callers retaining evidence must bound the wire bytes before decoding.
use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub fn response_json(body: &[u8], content_type: &str) -> Result<Vec<u8>> {
    match content_type.split(';').next().unwrap_or("").trim() {
        "application/json" => Ok(body.to_vec()),
        "text/event-stream" => {
            let text = std::str::from_utf8(body).context("MCP SSE response is not UTF-8")?;
            let mut data = String::new();
            let mut terminal = None;
            for line in text.lines() {
                if line.is_empty() {
                    if data.is_empty() {
                        continue;
                    }
                    let message: Value =
                        serde_json::from_str(&data).context("invalid MCP SSE message")?;
                    ensure!(message["jsonrpc"] == "2.0", "invalid MCP SSE protocol");
                    if message.get("result").is_some() || message.get("error").is_some() {
                        ensure!(terminal.is_none(), "multiple terminal MCP responses");
                        terminal = Some(std::mem::take(&mut data).into_bytes());
                    } else {
                        ensure!(
                            message.get("method").is_some() && message.get("id").is_none(),
                            "unexpected MCP SSE message"
                        );
                        data.clear();
                    }
                } else if let Some(value) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.strip_prefix(' ').unwrap_or(value));
                }
            }
            ensure!(data.is_empty(), "incomplete MCP SSE event");
            terminal.context("MCP stream ended without a terminal response")
        }
        _ => anyhow::bail!("unsupported MCP response content type"),
    }
}

/// Convenience for local fixtures; production evidence readers apply byte bounds first.
pub trait McpResponse {
    fn mcp_json<T: DeserializeOwned + Send>(
        self,
    ) -> impl std::future::Future<Output = Result<T>> + Send;
}
impl McpResponse for reqwest::Response {
    async fn mcp_json<T: DeserializeOwned + Send>(self) -> Result<T> {
        let content_type = self
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let bytes = self.bytes().await?;
        Ok(serde_json::from_slice(&response_json(
            &bytes,
            &content_type,
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_comments_multiline_and_terminal_completeness() {
        let body = b": keepalive\r\n\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":1,\"result\":{}}\r\n\r\n";
        let decoded = response_json(body, "text/event-stream; charset=utf-8").unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&decoded).unwrap()["id"], 1);
        assert!(response_json(b": keepalive\n\n", "text/event-stream").is_err());
        assert!(response_json(&body[..body.len() - 2], "text/event-stream").is_err());
        assert!(
            response_json(
                &[body.as_slice(), body.as_slice()].concat(),
                "text/event-stream"
            )
            .is_err()
        );
    }
}
