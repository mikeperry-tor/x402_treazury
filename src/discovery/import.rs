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
    tokio::time::timeout(
        network::global().request_timeout(Duration::from_secs(policy.fetch_timeout_seconds)),
        async {
            let mut response = request
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("source_fetch_failed"))?;
            ensure!(
                response.status().is_success(),
                "source_http_{}",
                response.status().as_u16()
            );
            if response
                .content_length()
                .is_some_and(|n| n > policy.max_spec_bytes as u64)
            {
                return Err(crate::limits::exceeded(
                    "imported spec",
                    "max_spec_bytes",
                    policy.max_spec_bytes,
                )
                .context("spec_too_large"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| anyhow::anyhow!("source_read_failed"))?
            {
                if chunk.len() > policy.max_spec_bytes.saturating_sub(bytes.len()) {
                    return Err(crate::limits::exceeded(
                        "imported spec",
                        "max_spec_bytes",
                        policy.max_spec_bytes,
                    )
                    .context("spec_too_large"));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        },
    )
    .await
    .context("source_fetch_timeout")?
}
#[cfg(test)]
pub async fn fixture_fetch(policy: &Policy, url: &str) -> Result<Vec<u8>> {
    let client = network::discovery(url, Duration::from_secs(policy.fetch_timeout_seconds))?;
    fetch_response(policy, client.get(url)).await
}

fn check_complexity(depth: usize, nodes: usize, bytes: usize) -> Result<()> {
    if depth > 64 || nodes > 500_000 || bytes > 64 * 1024 * 1024 {
        tracing::warn!(
            depth,
            nodes,
            expanded_bytes = bytes,
            "import rejected: document_complexity_limit (depth 64, nodes 500000, expanded bytes 67108864)"
        );
        anyhow::bail!(
            "document_complexity_limit: depth <= 64, nodes <= 500000, expanded bytes <= 67108864; document rejected"
        );
    }
    Ok(())
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
    check_complexity(depth, *nodes, *bytes)?;
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
            check_complexity(depth, *nodes, *bytes)?;
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
    if bytes.len() > policy.max_spec_bytes {
        return Err(crate::limits::exceeded(
            "imported spec",
            "max_spec_bytes",
            policy.max_spec_bytes,
        )
        .context("spec_too_large"));
    }
    ensure!(
        !c.name.is_empty()
            && c.name.len() <= 64
            && c.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "invalid source name"
    );
    let spec = endpoint(policy, &c.spec_url)?;
    let root = parse_document(bytes)?;
    let paths = root["paths"]
        .as_object()
        .context("OpenAPI paths required")?;
    ensure!(paths.len() <= 10000, "operation_count_limit");
    let (base, base_url) = resolve_base(policy, c, &spec, &root)?;
    validate_paths(paths)?;
    let tools = selected_tools(c, id, &root, &base)?;
    validate_tools(policy, &tools, &base, &base_url)?;
    Ok(Built { tools, base })
}

// Inspect reference expansion before deriving any catalog from untrusted JSON.
fn parse_document(bytes: &[u8]) -> Result<Value> {
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
    Ok(root)
}

fn resolve_base(
    policy: &Policy,
    c: &Candidate,
    spec: &reqwest::Url,
    root: &Value,
) -> Result<(String, reqwest::Url)> {
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
    Ok((base, base_url))
}

fn validate_paths(paths: &serde_json::Map<String, Value>) -> Result<()> {
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
    Ok(())
}

fn selected_tools(c: &Candidate, id: &str, root: &Value, base: &str) -> Result<Vec<ToolSpec>> {
    let cfg = Config {
        spec: c.spec_url.clone(),
        base_url: Some(base.to_owned()),
        include: c.selection.include.clone(),
        exclude: c.selection.exclude.clone(),
        tags: c.selection.tags.clone(),
        exclude_tags: c.selection.exclude_tags.clone(),
        probe_pricing: false,
        pricing_key: Some("x-payment-info".into()),
        ..Default::default()
    };
    let prefix = format!("dyn_{}", id.replace('-', ""));
    let tools = catalog::build_tools(&cfg, root, &prefix)?
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
    Ok(tools)
}

fn validate_tools(
    policy: &Policy,
    tools: &[ToolSpec],
    base: &str,
    base_url: &reqwest::Url,
) -> Result<()> {
    ensure!(!tools.is_empty(), "no tools selected");
    ensure!(
        tools.len() <= policy.max_tools_per_source,
        "source_tool_limit"
    );
    for tool in tools {
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
    Ok(())
}

#[cfg(test)]
mod complexity_tests {
    use super::*;
    fn check(value: &Value) -> Result<()> {
        inspect(value, value, 0, &mut 0, &mut 0, &mut Default::default())
    }
    #[test]
    fn exact_depth_node_and_expanded_byte_limits() {
        let mut nested = Value::Null;
        for _ in 0..64 {
            nested = serde_json::json!([nested]);
        }
        check(&nested).unwrap();
        assert!(
            check(&serde_json::json!([nested]))
                .unwrap_err()
                .to_string()
                .contains("complexity")
        );
        let mut nodes = Value::Array(vec![Value::Null; 499_999]);
        check(&nodes).unwrap();
        nodes.as_array_mut().unwrap().push(Value::Null);
        assert!(check(&nodes).is_err());
        drop(nodes);
        let mut bytes = Value::String("x".repeat(64 * 1024 * 1024));
        check(&bytes).unwrap();
        if let Value::String(text) = &mut bytes {
            text.push('x');
        }
        assert!(check(&bytes).is_err());
    }
    #[test]
    fn repeated_acyclic_refs_escape_pointers_and_reject_bad_documents() {
        let doc = serde_json::json!({"components":{"schemas":{"a/b~c":{"type":"string"}}},
        "paths":{"/test":{"get":{"parameters":[
            {"name":"a","in":"query","schema":{"$ref":"#/components/schemas/a~1b~0c"}},
            {"name":"b","in":"query","schema":{"$ref":"#/components/schemas/a~1b~0c"}}
        ]}}}});
        check(&doc).unwrap();
        let tools = catalog::build_tools(&Config::default(), &doc, "t").unwrap();
        assert_eq!(tools[0].input_schema["properties"]["a"]["type"], "string");
        assert_eq!(tools[0].input_schema["properties"]["b"]["type"], "string");
        for doc in [
            serde_json::json!({"$ref":"#/missing"}),
            serde_json::json!({"loop":{"$ref":"#/loop"}}),
            serde_json::json!({"$ref":"https://example.com"}),
        ] {
            assert!(check(&doc).is_err());
        }
    }
}
