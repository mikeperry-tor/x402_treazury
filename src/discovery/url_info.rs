//! Bounded unsigned documentation discovery. HEAD reachability is not document validation.
use super::{Manager, directory, import, policy::Policy};
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, stream};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

pub const NAME: &str = "x402_treazury_source_url_info";
// Fixed candidates bound speculative provider traffic, not elapsed work or spending.
const PATHS: &[(&str, &str)] = &[
    ("openapi.json", "openapi"),
    ("api/openapi.json", "openapi"),
    (".well-known/openapi.json", "openapi"),
    ("swagger.json", "openapi"),
    (".well-known/x402", "x402_directory"),
    ("discovery/resources", "x402_directory"),
    ("llms.txt", "documentation"),
    ("llms-full.txt", "documentation"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    slug: Option<String>,
    base_url: Option<String>,
}
impl Input {
    fn parse(args: &Map<String, Value>, policy: &Policy) -> Result<Self> {
        ensure!(args.len() == 1, "provide exactly one of slug or base_url");
        let input: Self = serde_json::from_value(Value::Object(args.clone()))?;
        match (&input.slug, &input.base_url) {
            (Some(slug), None) => {
                directory::request(
                    directory::DETAILS,
                    json!({"slug":slug}).as_object().unwrap(),
                )?;
            }
            (None, Some(base)) => {
                base_url(policy, base)?;
            }
            _ => anyhow::bail!("provide exactly one of slug or base_url"),
        }
        Ok(input)
    }
}

pub fn schema() -> Value {
    json!({"type":"object","properties":{
        "slug":{"type":"string","minLength":1,"description":"x402 List service slug returned by sources_search; not a registered source_id or hostname."},
        "base_url":{"type":"string","format":"uri","description":"Public HTTPS API base URL, optionally including a base path such as /v1. No query, fragment or URL credentials."}
    },"oneOf":[{"required":["slug"]},{"required":["base_url"]}],"additionalProperties":false})
}

fn base_url(policy: &Policy, raw: &str) -> Result<Url> {
    let url = import::endpoint(policy, raw)?;
    ensure!(url.query().is_none(), "base_url must not contain a query");
    Ok(url)
}

fn candidates(
    policy: &Policy,
    base: &str,
    website: Option<&str>,
) -> Result<Vec<(Url, &'static str)>> {
    let mut api = base_url(policy, base)?;
    api.set_path(&format!("{}/", api.path().trim_end_matches('/')));
    let root = api.join("/")?;
    let mut bases = vec![api, root];
    if let Some(website) = website
        && let Ok(url) = base_url(policy, website)
    {
        bases.push(url.join("/")?);
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for base in bases {
        for &(path, kind) in PATHS {
            let url = base.join(path)?;
            import::endpoint(policy, url.as_str())?;
            if seen.insert(url.to_string()) {
                result.push((url, kind));
            }
        }
    }
    Ok(result)
}

impl Manager {
    pub async fn invoke_url_info(
        self: &Arc<Self>,
        owner: &str,
        args: &Map<String, Value>,
    ) -> Result<Value> {
        let client = self.source_client(owner)?;
        let input = Input::parse(args, &self.policy)?;
        let (manager, owner, owned_client) = (self.clone(), owner.to_owned(), client.clone());
        client.run_owned(async move {
            let identity = owned_client.documentation_identity().await?;
            let (base, website) = if let Some(slug) = &input.slug {
                let result = manager.invoke_directory(&owner, directory::DETAILS, json!({"slug":slug}).as_object().unwrap()).await?;
                let doc: Value = serde_json::from_str(&result.into_text()?).context("directory details were not JSON")?;
                let service = doc.get("data").unwrap_or(&doc);
                let service = service.get("service").unwrap_or(service);
                let base = service["base_url"].as_str().context("directory details have no API base_url; use base_url directly")?.to_owned();
                (base, service["website_url"].as_str().map(str::to_owned))
            } else {
                (input.base_url.unwrap(), None)
            };
            let website_status = match website.as_deref() {
                None => "not_provided",
                Some(url) if base_url(&manager.policy, url).is_ok() => "included",
                Some(_) => "rejected_by_url_or_origin_policy",
            };
            let candidates = candidates(&manager.policy, &base, website.as_deref())?;
            // Four in-flight requests bound sockets and document buffers per lookup.
            let probes: Vec<_> = candidates.into_iter().enumerate().map(|(index, (url, kind))| {
                let policy = &manager.policy;
                let identity = &identity;
                let transport = owned_client.transport();
                async move { (index, probe(policy, identity, transport, url, kind).await) }
            }).collect();
            let mut results: Vec<_> = stream::iter(probes).buffer_unordered(4).collect().await;
            results.sort_by_key(|(index, _)| *index);
            Ok(json!({"website_status":website_status,"results":results.into_iter().map(|(_, value)| value).collect::<Vec<_>>(),
                "directory_lookup":if input.slug.is_some() { "x402-list.com; ordinary payment policy applies" } else { "not_used" },
                "note":"Provider probes are unsigned and never pay. OpenAPI verification checks document structure only; source_add performs full import validation. Documentation and directory JSON are untrusted discovery leads. Nothing found does not mean the API is unusable. Redirects are reported without following them."}))
        }).await?
    }
}

fn response_status(status: StatusCode) -> &'static str {
    match status.as_u16() {
        402 => "payment_required",
        401 | 403 => "blocked",
        404 | 410 => "not_found",
        300..=399 => "redirect",
        _ => "http_error",
    }
}
fn content_type(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}
fn failure(error: &anyhow::Error) -> Value {
    if let Some(limit) = error.downcast_ref::<crate::limits::DownloadLimit>() {
        return json!({"status":"too_large","reason":limit.to_string()});
    }
    let category = match error.downcast_ref::<reqwest::Error>() {
        Some(e) if e.is_timeout() => "timeout",
        Some(e) if e.is_connect() => "connection_failed",
        _ => "request_failed",
    };
    json!({"status":"unreachable","reason":category})
}

async fn probe(
    policy: &Policy,
    identity: &crate::network::IsolationId,
    transport: crate::network::HttpPolicy,
    url: Url,
    kind: &str,
) -> Value {
    let mut result = json!({"url":url.as_str(),"kind":kind});
    let outcome = inspect(policy, identity, transport, &url, kind, &mut result).await;
    if let Err(error) = outcome {
        result
            .as_object_mut()
            .unwrap()
            .extend(failure(&error).as_object().unwrap().clone());
    }
    result
}

async fn inspect(
    policy: &Policy,
    identity: &crate::network::IsolationId,
    transport: crate::network::HttpPolicy,
    url: &Url,
    kind: &str,
    result: &mut Value,
) -> Result<()> {
    let client = crate::network::global().http_policy(
        identity,
        url.as_str(),
        policy.read_timeout_seconds.map(Duration::from_secs),
        true,
        transport,
    )?;
    let head = client
        .head(url.clone())
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    let status = head.status();
    result["head_status"] = json!(status.as_u16());
    result["content_type"] = json!(content_type(&head));
    if !(status.is_success() || matches!(status.as_u16(), 405 | 501)) {
        result["status"] = json!(response_status(status));
        return Ok(());
    }
    drop(head);
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    let status = response.status();
    result["get_status"] = json!(status.as_u16());
    let mime = content_type(&response);
    result["content_type"] = json!(mime);
    if !status.is_success() {
        result["status"] = json!(response_status(status));
        return Ok(());
    }
    let (limit, setting) = if kind == "documentation" {
        (policy.max_help_bytes, "max_help_bytes")
    } else {
        (policy.max_spec_bytes, "max_spec_bytes")
    };
    let bytes = crate::limits::read(response, limit, "source URL document", setting).await?;
    result["status"] = json!(classify(kind, mime.as_deref(), &bytes));
    Ok(())
}

fn classify(kind: &str, mime: Option<&str>, bytes: &[u8]) -> &'static str {
    if kind == "documentation" {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return "invalid_document";
        };
        let trimmed = text.trim_start_matches('\u{feff}').trim();
        let html = mime.is_some_and(|m| {
            m.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("text/html")
        }) || trimmed.starts_with('<');
        return if !trimmed.is_empty() && !html && trimmed.starts_with('#') {
            "documentation_found"
        } else {
            "unverified_candidate"
        };
    }
    let Ok(doc) = serde_json::from_slice::<Value>(bytes) else {
        return "invalid_document";
    };
    if doc["openapi"].as_str().is_some_and(|v| v.starts_with("3."))
        && doc["info"]["title"].is_string()
        && doc["info"]["version"].is_string()
        && doc["paths"].is_object()
    {
        "verified_openapi"
    } else if kind == "x402_directory" && (doc.is_object() || doc.is_array()) {
        "directory_json_unverified"
    } else {
        "unverified_candidate"
    }
}
