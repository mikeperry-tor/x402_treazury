use super::policy::Policy;
use crate::{
    catalog::{self, Config, ToolSpec},
    network::{self, IsolationId},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    #[default]
    Process,
    Persistent,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Server,
    Servers,
    Process,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub name: String,
    pub spec_url: String,
    pub base_url: Option<String>,
    #[serde(default)]
    pub selection: Selection,
    #[serde(default)]
    pub visibility: Visibility,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub lifetime: Lifetime,
}
#[derive(Clone, Debug)]
pub struct Built {
    pub tools: Vec<ToolSpec>,
    pub base: String,
}
pub fn endpoint(policy: &Policy, url: &str) -> Result<reqwest::Url> {
    let u = network::public_url(url)?;
    ensure!(
        policy.allowed_origins.is_empty()
            || policy
                .allowed_origins
                .contains(&u.origin().ascii_serialization()),
        "destination origin not authorized"
    );
    Ok(u)
}
pub async fn fetch(policy: &Policy, url: &str) -> Result<Vec<u8>> {
    endpoint(policy, url)?;
    let client = network::global().http_public(
        &IsolationId::discovery(url)?,
        url,
        Duration::from_secs(policy.fetch_timeout_seconds),
    )?;
    fetch_response(policy, client.get(url)).await
}
async fn fetch_response(policy: &Policy, request: reqwest::RequestBuilder) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(policy.fetch_timeout_seconds), async {
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("source_fetch_failed"))?;
        ensure!(
            response.status().is_success(),
            "source_http_{}",
            response.status().as_u16()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= policy.max_spec_bytes as u64),
            "spec_too_large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("source_read_failed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= policy.max_spec_bytes,
                "spec_too_large"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await
    .context("source_fetch_timeout")?
}
#[cfg(test)]
pub async fn fixture_fetch(policy: &Policy, url: &str) -> Result<Vec<u8>> {
    let client = network::discovery(url, Duration::from_secs(policy.fetch_timeout_seconds))?;
    fetch_response(policy, client.get(url)).await
}

fn inspect(
    root: &Value,
    v: &Value,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    *nodes += 1;
    ensure!(
        depth <= 64 && *nodes <= 500_000 && *bytes <= 64 * 1024 * 1024,
        "document_complexity_limit"
    );
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref") {
                let reference = r
                    .as_str()
                    .filter(|r| r.starts_with("#/"))
                    .context("external_reference_unsupported")?;
                let target = root
                    .pointer(&reference[1..])
                    .context("unresolved_reference")?;
                ensure!(
                    seen.insert(reference.to_owned()),
                    "cyclic_reference_unsupported"
                );
                inspect(root, target, depth + 1, nodes, bytes, seen)?;
                seen.remove(reference);
            }
            for (key, child) in o {
                *bytes += key.len();
                inspect(root, child, depth + 1, nodes, bytes, seen)?;
            }
        }
        Value::Array(a) => {
            for child in a {
                inspect(root, child, depth + 1, nodes, bytes, seen)?;
            }
        }
        Value::String(s) => {
            *bytes += s.len();
            ensure!(*bytes <= 64 * 1024 * 1024, "document_complexity_limit");
        }
        _ => (),
    }
    Ok(())
}
pub fn matches(name: &str, include: &[String], exclude: &[String]) -> Result<bool> {
    let any = |patterns: &[String]| -> Result<bool> {
        for p in patterns {
            if crate::deployment::pattern(p)?.is_match(name) {
                return Ok(true);
            }
        }
        Ok(false)
    };
    Ok((include.is_empty() || any(include)?) && !any(exclude)?)
}
pub fn build(policy: &Policy, c: &Candidate, id: &str, bytes: &[u8]) -> Result<Built> {
    ensure!(bytes.len() <= policy.max_spec_bytes, "spec_too_large");
    ensure!(
        !c.name.is_empty()
            && c.name.len() <= 64
            && c.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "invalid source name"
    );
    let spec = endpoint(policy, &c.spec_url)?;
    let root: Value = serde_json::from_slice(bytes).context("invalid OpenAPI JSON")?;
    inspect(
        &root,
        &root,
        0,
        &mut 0,
        &mut 0,
        &mut std::collections::BTreeSet::new(),
    )?;
    ensure!(
        root["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.")),
        "OpenAPI 3 JSON required"
    );
    let paths = root["paths"]
        .as_object()
        .context("OpenAPI paths required")?;
    ensure!(paths.len() <= 10000, "operation_count_limit");
    let base = if let Some(base) = &c.base_url {
        base.clone()
    } else {
        let server = root["servers"][0]["url"]
            .as_str()
            .context("base_url or OpenAPI server required")?;
        spec.join(server)
            .context("invalid OpenAPI server URL")?
            .to_string()
    };
    let base_url = endpoint(policy, &base)?;
    ensure!(
        base_url.query().is_none(),
        "base_url must not contain query parameters"
    );
    // Do not silently ignore per-operation server changes supported differently by providers.
    let mut operation_count = 0;
    for item in paths.values() {
        ensure!(item.get("$ref").is_none(), "path references unsupported");
        ensure!(
            item.get("servers").is_none(),
            "per-path servers unsupported"
        );
        if let Some(obj) = item.as_object() {
            for (method, op) in obj {
                if [
                    "get", "post", "put", "delete", "patch", "head", "options", "trace",
                ]
                .contains(&method.as_str())
                {
                    operation_count += 1;
                    ensure!(operation_count <= 10000, "operation_count_limit");
                    ensure!(
                        op.get("servers").is_none(),
                        "per-operation servers unsupported"
                    );
                }
            }
        }
    }
    let cfg = Config {
        spec: c.spec_url.clone(),
        base_url: Some(base.clone()),
        include: c.selection.include.clone(),
        exclude: c.selection.exclude.clone(),
        tags: c.selection.tags.clone(),
        exclude_tags: c.selection.exclude_tags.clone(),
        probe_pricing: false,
        pricing_key: Some("x-payment-info".into()),
        ..Default::default()
    };
    let prefix = format!("dyn_{}", id.replace('-', ""));
    let tools = catalog::build_tools(&cfg, &root, &prefix)?
        .into_iter()
        .filter_map(|t| {
            match matches(
                &t.name,
                &c.selection.include_tools,
                &c.selection.exclude_tools,
            ) {
                Ok(true) => Some(Ok(t)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(!tools.is_empty(), "no tools selected");
    ensure!(
        tools.len() <= policy.max_tools_per_source,
        "source_tool_limit"
    );
    for tool in &tools {
        ensure!(tool.name.len() <= 128, "tool_name_too_long");
        ensure!(
            serde_json::to_vec(&tool.input_schema)?.len() <= 262144,
            "schema_size_limit"
        );
        let url = if tool.path.starts_with("https://") || tool.path.starts_with("http://") {
            tool.path.clone()
        } else {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                tool.path.trim_start_matches('/')
            )
        };
        let url = endpoint(policy, &url)?;
        ensure!(
            url.origin() == base_url.origin()
                || policy
                    .allowed_origins
                    .contains(&url.origin().ascii_serialization()),
            "cross_origin_operation_rejected"
        );
    }
    Ok(Built { tools, base })
}
