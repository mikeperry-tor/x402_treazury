use anyhow::{Context, Result, bail, ensure};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_PRICE: &str =
    "Paid per call via x402 (price set by the API; per-payment cap via X402_MAX_PRICE_USD).";
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub timeout: f64,
    pub spec: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub prefix: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include_operations: Vec<String>,
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub pricing_key: Option<String>,
    pub probe_pricing: bool,
    pub probe_ttl_seconds: f64,
    pub probe_concurrency: usize,
    pub probe_timeout: f64,
    pub probe_max_endpoints: usize,
    pub probe_methods: Vec<String>,
    pub overrides: Value,
    pub additional_properties: Option<bool>,
    pub instructions_text: Option<String>,
    pub help_url: Option<String>,
    pub max_description_chars: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: 30.0,
            spec: String::new(),
            name: None,
            base_url: None,
            prefix: None,
            include: vec![],
            exclude: vec![],
            include_operations: vec![],
            tags: vec![],
            exclude_tags: vec![],
            pricing_key: None,
            probe_pricing: true,
            probe_ttl_seconds: 3600.0,
            probe_concurrency: 4,
            probe_timeout: 5.0,
            probe_max_endpoints: 200,
            probe_methods: vec!["GET".into()],
            overrides: json!({}),
            additional_properties: None,
            instructions_text: None,
            help_url: None,
            max_description_chars: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub method: String,
    pub path: String,
    pub input_schema: Value,
    pub param_routes: BTreeMap<String, String>,
    pub has_body: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
    #[serde(skip)]
    pub body_names: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct RoutedRequest {
    pub method: String,
    pub url: String,
    pub query: BTreeMap<String, Value>,
    pub body: Option<Value>,
}

impl ToolSpec {
    pub fn route(&self, base_url: &str, args: &Map<String, Value>) -> Result<RoutedRequest> {
        let mut path = self.path.clone();
        let mut query = BTreeMap::new();
        let mut body = Map::new();
        for (key, value) in args.iter().filter(|(_, v)| !v.is_null()) {
            match self
                .param_routes
                .get(key)
                .map(String::as_str)
                .unwrap_or(if self.has_body { "body" } else { "query" })
            {
                "path" => {
                    path = path.replace(
                        &format!("{{{key}}}"),
                        &utf8_percent_encode(&arg_text(value), PATH_SEGMENT).to_string(),
                    )
                }
                "query" => {
                    query.insert(key.clone(), value.clone());
                }
                _ => {
                    body.insert(
                        self.body_names.get(key).unwrap_or(key).clone(),
                        value.clone(),
                    );
                }
            }
        }
        ensure!(
            !path.contains('{'),
            "missing path parameter for {}",
            self.name
        );
        // httpx appends relative paths to base_url, including a gateway prefix.
        let url = if path.starts_with("https://") || path.starts_with("http://") {
            path
        } else {
            format!(
                "{}/{}",
                base_url.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        };
        reqwest::Url::parse(&url).context("invalid request URL")?;
        Ok(RoutedRequest {
            method: self.method.clone(),
            url,
            query,
            body: self.has_body.then_some(Value::Object(body)),
        })
    }
}

pub fn arg_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

pub fn slug(text: &str) -> String {
    let re = Regex::new("[^0-9a-z]+").expect("constant regex");
    re.replace_all(&text.to_lowercase(), "_")
        .trim_matches('_')
        .to_string()
}

pub fn default_prefix(base: &str) -> Result<String> {
    let url = reqwest::Url::parse(base)?;
    let mut labels: Vec<_> = url.host_str().unwrap_or("api").split('.').collect();
    if labels.len() > 1 && matches!(labels[0], "api" | "www") {
        labels.remove(0);
    }
    if labels.len() > 1 {
        labels.pop();
    }
    Ok(slug(&labels.join(".")))
}

fn anchor<'a>(path: &str, prefixes: &'a [String]) -> Option<&'a str> {
    prefixes
        .iter()
        .filter(|p| path == p.as_str() || path.starts_with(&format!("{p}/")))
        .max_by_key(|p| p.len())
        .map(String::as_str)
}

pub async fn load_json(source: &str, http: &reqwest::Client) -> Result<Value> {
    let bytes = if source.starts_with("https://") || source.starts_with("http://") {
        http.get(source)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec()
    } else {
        tokio::fs::read(source)
            .await
            .with_context(|| format!("reading {source}"))?
    };
    Ok(serde_json::from_slice(&bytes)?)
}

fn resolve(root: &Value, node: &Value, seen: &mut BTreeSet<String>) -> Value {
    match node {
        Value::Object(map) => {
            if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
                if reference.starts_with("#/") && seen.insert(reference.to_owned()) {
                    let result = root
                        .pointer(&reference[1..])
                        .map(|v| resolve(root, v, seen));
                    seen.remove(reference);
                    if let Some(value) = result {
                        return value;
                    }
                }
                return node.clone();
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), resolve(root, v, seen)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve(root, v, seen)).collect()),
        _ => node.clone(),
    }
}

pub fn sanitize(node: &Value) -> Value {
    match node {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter_map(|(k, v)| {
                    if matches!(k.as_str(), "exclusiveMinimum" | "exclusiveMaximum") {
                        let bound = if v == &Value::Bool(true) {
                            map.get(if k == "exclusiveMinimum" {
                                "minimum"
                            } else {
                                "maximum"
                            })
                        } else {
                            Some(v)
                        };
                        return bound
                            .filter(|v| v.is_number())
                            .map(|v| (k.clone(), v.clone()));
                    }
                    Some((k.clone(), sanitize(v)))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(sanitize).collect()),
        _ => node.clone(),
    }
}

fn schema(node: &Value) -> Value {
    let Some(map) = node.as_object() else {
        return node.clone();
    };
    let mut out = Map::new();
    for (key, value) in map {
        match key.as_str() {
            "type" | "format" | "enum" | "const" | "minimum" | "maximum" | "exclusiveMinimum"
            | "exclusiveMaximum" | "minLength" | "maxLength" | "minItems" | "maxItems"
            | "pattern" | "default" | "nullable" | "description" | "title" | "required" => {
                out.insert(key.clone(), value.clone());
            }
            "properties" => {
                out.insert(
                    key.clone(),
                    Value::Object(
                        value
                            .as_object()
                            .into_iter()
                            .flatten()
                            .map(|(k, v)| (k.clone(), schema(v)))
                            .collect(),
                    ),
                );
            }
            "items" | "additionalProperties" => {
                out.insert(key.clone(), schema(value));
            }
            "anyOf" | "oneOf" | "allOf" => {
                out.insert(
                    key.clone(),
                    Value::Array(value.as_array().into_iter().flatten().map(schema).collect()),
                );
            }
            _ => {}
        }
    }
    sanitize(&Value::Object(out))
}

pub fn operations(root: &Value, pricing_key: Option<&str>) -> Result<Vec<Value>> {
    if root.get("paths").is_none() {
        return root
            .get("operations")
            .and_then(Value::as_array)
            .cloned()
            .context("expected OpenAPI paths or operations digest");
    }
    let mut ops = Vec::new();
    for (path, item) in root["paths"]
        .as_object()
        .context("paths must be an object")?
    {
        for (method, op) in item.as_object().context("invalid path item")? {
            if !["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                continue;
            }
            let mut params = Vec::new();
            for p in op["parameters"].as_array().into_iter().flatten() {
                let p = resolve(root, p, &mut BTreeSet::new());
                if !p["name"].is_string() {
                    continue;
                }
                let s = schema(&resolve(
                    root,
                    p.get("schema").unwrap_or(&json!({})),
                    &mut BTreeSet::new(),
                ));
                params.push(json!({"name": p["name"], "in": p["in"],
                    "required": p["required"].as_bool().unwrap_or(false),
                    "description": p.get("description").filter(|v| !v.is_null()).unwrap_or(&s["description"]),
                    "schema": s}));
            }
            let body = op.get("requestBody").filter(|v| !v.is_null()).map(|rb| {
                let rb = resolve(root, rb, &mut BTreeSet::new());
                json!({"required": rb["required"].as_bool().unwrap_or(false),
                    "schema": schema(rb.pointer("/content/application~1json/schema").unwrap_or(&json!({})))})
            });
            ops.push(json!({"path":path, "method":method, "summary":op["summary"],
                "description":op["description"], "tags":op.get("tags").cloned().unwrap_or(json!([])),
                "pricing":pricing_key.and_then(|k| op.get(k)), "params":params, "body":body}));
        }
    }
    Ok(ops)
}

/// Counts from the source before selection, including tags on excluded operations.
pub fn tag_counts(root: &Value) -> Result<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for op in operations(root, None)? {
        let tags: BTreeSet<_> = op["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if tags.is_empty() {
            *counts.entry("(untagged)".into()).or_default() += 1;
        }
        for tag in tags {
            *counts.entry(tag.to_owned()).or_default() += 1;
        }
    }
    Ok(counts)
}

fn money(value: &Value) -> Option<String> {
    if !(value.is_string() || value.is_u64() || value.is_i64()) {
        return None;
    }
    let value = arg_text(value).trim().to_owned();
    if !Regex::new(r"^\d+(?:\.\d+)?$")
        .expect("constant regex")
        .is_match(&value)
    {
        return None;
    }
    Some(if let Some((a, b)) = value.split_once('.') {
        let b = b.trim_end_matches('0');
        format!("{a}.{b:0<2}")
    } else {
        value
    })
}

fn price(op: &Value) -> (String, bool) {
    let p = &op["pricing"];
    if p.is_null() || p == &json!({}) {
        return (DEFAULT_PRICE.into(), false);
    }
    if p["authMode"] == "free" && p.get("price").is_none() {
        return ("Free — no payment required (vendor spec).".into(), true);
    }
    let block = p.get("price").filter(|v| v.is_object()).unwrap_or(p);
    let show = |amount: String| match block["currency"].as_str() {
        Some(c) if c.eq_ignore_ascii_case("USD") || c.eq_ignore_ascii_case("USDC") => {
            format!("${amount}")
        }
        Some(c) => format!("{amount} {c}"),
        None => amount,
    };
    let line = if let Some(amount) = money(&block["amount"]) {
        let dynamic = block["mode"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("dynamic"));
        format!(
            "Price: {} per call (vendor spec{}).",
            show(amount),
            if dynamic { ", dynamic" } else { "" }
        )
    } else if let (Some(lo), Some(hi)) = (money(&block["min"]), money(&block["max"])) {
        format!(
            "Price: {}–{} per call, scaling with usage (vendor spec).",
            show(lo),
            show(hi)
        )
    } else {
        format!("Pricing: {p} (vendor spec extension).")
    };
    (line, true)
}

pub fn build_tools(cfg: &Config, root: &Value, prefix: &str) -> Result<Vec<ToolSpec>> {
    build_tools_with_prices(cfg, root, prefix, &BTreeMap::new())
}

pub fn build_tools_with_prices(
    cfg: &Config,
    root: &Value,
    prefix: &str,
    prices: &BTreeMap<(String, String), String>,
) -> Result<Vec<ToolSpec>> {
    let mut ops = operations(root, cfg.pricing_key.as_deref())?;
    for op in &ops {
        ensure!(
            op["path"].is_string() && op["method"].is_string(),
            "invalid operation"
        );
    }
    ops.sort_by_key(|o| {
        (
            o["path"].as_str().unwrap().to_owned(),
            o["method"].as_str().unwrap().to_owned(),
        )
    });
    let mut selected = Vec::new();
    let mut counts = BTreeMap::<String, usize>::new();
    for op in ops {
        let path = op["path"].as_str().unwrap();
        let operation = format!(
            "{} {path}",
            op["method"].as_str().unwrap().to_ascii_uppercase()
        );
        if !cfg.include_operations.is_empty() && !cfg.include_operations.contains(&operation) {
            continue;
        }
        if anchor(path, &cfg.exclude).is_some() {
            continue;
        }
        let anchored = anchor(path, &cfg.include);
        if !cfg.include.is_empty() && anchored.is_none() {
            continue;
        }
        let tags: BTreeSet<_> = op["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if cfg.exclude_tags.iter().any(|t| tags.contains(t.as_str())) {
            continue;
        }
        if !cfg.tags.is_empty() && !cfg.tags.iter().any(|t| tags.contains(t.as_str())) {
            continue;
        }
        let suffix = slug(path.strip_prefix(anchored.unwrap_or("")).unwrap_or(path));
        let base = format!(
            "{prefix}_{}",
            if suffix.is_empty() { "root" } else { &suffix }
        );
        *counts.entry(base.clone()).or_default() += 1;
        selected.push((op, base));
    }
    ensure!(!selected.is_empty(), "no operations matched");
    let mut seen = BTreeMap::<String, usize>::new();
    let mut tools = Vec::new();
    let re = Regex::new(r"\$\d[\d,]*(?:\.\d+)?").expect("constant regex");
    for (op, base) in selected {
        let method = op["method"].as_str().unwrap();
        let name = if counts[&base] > 1 {
            format!("{base}_{}", method.to_lowercase())
        } else {
            base
        };
        let count = seen.entry(name.clone()).or_default();
        *count += 1;
        let name = if *count > 1 {
            format!("{name}_{count}")
        } else {
            name
        };
        let mut props = Map::new();
        let mut routes = BTreeMap::new();
        let mut required = BTreeSet::new();
        let mut body_names = BTreeMap::new();
        for p in op["params"].as_array().into_iter().flatten() {
            if p["in"] == "header" {
                continue;
            }
            let Some(key) = p["name"].as_str() else {
                continue;
            };
            let mut prop = p["schema"].as_object().cloned().unwrap_or_default();
            prop.entry("type").or_insert(json!("string"));
            if p["description"].as_str().is_some_and(|s| !s.is_empty()) {
                prop.entry("description")
                    .or_insert(p["description"].clone());
            }
            props.insert(key.to_string(), Value::Object(prop));
            routes.insert(
                key.to_string(),
                if p["in"] == "path" { "path" } else { "query" }.into(),
            );
            if p["required"] == true {
                required.insert(key.to_string());
            }
        }
        let has_body = !op["body"].is_null();
        let body = &op["body"]["schema"];
        for (key, value) in body["properties"].as_object().into_iter().flatten() {
            let name = if props.contains_key(key) {
                format!("{key}_body")
            } else {
                key.clone()
            };
            ensure!(
                !props.contains_key(&name),
                "unresolvable body/query collision: {key}"
            );
            props.insert(
                name.clone(),
                if value.is_object() {
                    value.clone()
                } else {
                    json!({"type":"string"})
                },
            );
            body_names.insert(name.clone(), key.clone());
            routes.insert(name.clone(), "body".into());
            if body["required"]
                .as_array()
                .is_some_and(|r| r.contains(&json!(key)))
            {
                required.insert(name);
            }
        }
        let mut input_schema = json!({"type":"object", "properties":props});
        if !required.is_empty() {
            input_schema["required"] = json!(required);
        }
        let text = op["description"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| op["summary"].as_str().filter(|s| !s.is_empty()))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("{} {}", method.to_uppercase(), op["path"].as_str().unwrap())
            });
        let (mut price, vendor) = price(&op);
        if !vendor
            && let Some(line) = prices.get(&(
                method.to_uppercase(),
                op["path"].as_str().unwrap().to_owned(),
            ))
        {
            price = line.clone();
        }
        let tokens: Vec<_> = re.find_iter(&price).map(|m| m.as_str()).collect();
        let mut description =
            if vendor && !tokens.is_empty() && tokens.iter().all(|s| text.contains(s)) {
                text
            } else {
                format!("{text} {price}")
            };
        if let Some(max) = cfg.max_description_chars {
            description = description.chars().take(max).collect();
        }
        tools.push(ToolSpec {
            name,
            description,
            method: method.to_uppercase(),
            path: op["path"].as_str().unwrap().into(),
            input_schema: sanitize(&input_schema),
            param_routes: routes,
            has_body,
            help_url: None,
            body_names,
        });
    }
    if let Some(url) = &cfg.help_url {
        tools.push(ToolSpec { name: format!("{prefix}_help"), description: format!(
            "Extended documentation for all {prefix}_* tools: API-wide usage guidance, pricing notes, and workflows published by the vendor (llms.txt). Takes no arguments and returns the full document. Call this before other {prefix}_* tools when unsure how to use them."),
            method:"GET".into(), path:url.clone(), input_schema:json!({"type":"object","properties":{}}),
            param_routes:BTreeMap::new(), has_body:false, help_url:Some(url.clone()), body_names:BTreeMap::new() });
    }
    for tool in &mut tools {
        if let Some(ap) = cfg.additional_properties {
            tool.input_schema["additionalProperties"] = json!(ap);
        }
        if let Some(patch) = cfg.overrides.get(&tool.name) {
            if let Some(desc) = patch["description"].as_str() {
                tool.description = desc.into();
            }
            for (key, value) in patch["params"].as_object().into_iter().flatten() {
                if value.is_string() && tool.input_schema["properties"][key].is_object() {
                    tool.input_schema["properties"][key]["description"] = value.clone();
                }
            }
        }
    }
    let names: BTreeSet<_> = tools.iter().map(|t| &t.name).collect();
    if names.len() != tools.len() {
        bail!("duplicate generated tool names");
    }
    Ok(tools)
}
