//! Caller names and exact JSON-body shape, without a serializer framework.
use super::{HeaderParameter, contracts::Unsupported, headers};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyArgument {
    pub name: String,
    pub required: bool,
}

pub(super) struct Names {
    reserved: BTreeSet<String>,
    used: BTreeSet<String>,
}
impl Names {
    fn new(params: &[Value], body: Option<&FlatBody<'_>>) -> Self {
        let mut reserved: BTreeSet<_> = params
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_owned))
            .collect();
        if let Some(body) = body {
            reserved.extend(body.properties.keys().map(|key| (*key).to_owned()));
        }
        Self {
            reserved,
            used: BTreeSet::new(),
        }
    }
    pub(super) fn argument(&mut self, wire: &str, location: &str) -> String {
        if self.used.insert(wire.into()) {
            return wire.into();
        }
        let base = format!("{wire}_{location}");
        let mut candidate = base.clone();
        let mut suffix = 2usize;
        while self.reserved.contains(&candidate) || self.used.contains(&candidate) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        self.used.insert(candidate.clone());
        candidate
    }
}

// These maps express one argument identity; update them together at insertion.
// Headers retain their existing specialized serialization/managed-key handling.
struct ArgumentFields {
    props: Map<String, Value>,
    routes: BTreeMap<String, String>,
    wire_names: BTreeMap<String, String>,
    required: BTreeSet<String>,
    names: Names,
}
impl ArgumentFields {
    fn new(names: Names) -> Self {
        Self {
            props: Map::new(),
            routes: BTreeMap::new(),
            wire_names: BTreeMap::new(),
            required: BTreeSet::new(),
            names,
        }
    }
    fn insert(&mut self, wire: &str, location: &str, property: Value, mandatory: bool) -> String {
        let name = self.names.argument(wire, location);
        self.props.insert(name.clone(), property);
        self.routes.insert(name.clone(), location.into());
        if name != wire {
            self.wire_names.insert(name.clone(), wire.into());
        }
        if mandatory {
            self.required.insert(name.clone());
        }
        name
    }
}

// Header and path/query argument construction share one schema-metadata policy.
// Call only on catalog-normalized parameters after the shared copy/ref preflight.
pub(super) fn parameter_schema(parameter: &Value, codec_note: &str) -> Value {
    let mut schema = parameter
        .get("schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"string"}));
    if let Some(fields) = schema.as_object_mut() {
        if let Some(description) = parameter
            .get("description")
            .filter(|value| value.as_str().is_some_and(|text| !text.is_empty()))
        {
            fields
                .entry("description")
                .or_insert_with(|| description.clone());
        }
        if !fields.contains_key("type") {
            let description = fields
                .get("description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map_or_else(
                    || codec_note.to_owned(),
                    |text| format!("{text} {codec_note}"),
                );
            fields.insert("description".into(), Value::String(description));
        }
    }
    schema
}

pub(super) struct OperationInput {
    pub schema: Value,
    pub routes: BTreeMap<String, String>,
    pub wire_names: BTreeMap<String, String>,
    pub header_params: BTreeMap<String, HeaderParameter>,
    pub has_body: bool,
    pub body_closed: bool,
    pub whole_body: Option<BodyArgument>,
    pub json_media_type: Option<String>,
}
fn issue(value: &Value) -> Result<()> {
    if let Some(reason) = value.get("issue") {
        return Err(serde_json::from_value::<Unsupported>(reason.clone())?.into());
    }
    Ok(())
}
// A bounded wire-family exception, not a schema evaluator. Keep the authored
// alternatives/default unchanged and let the existing array codec guards apply.
fn string_or_string_array(schema: &Value) -> bool {
    if schema.get("type").is_some()
        || ["allOf", "oneOf", "not", "if", "then", "else", "prefixItems"]
            .iter()
            .any(|key| schema.get(*key).is_some())
    {
        return false;
    }
    let Some(arms) = schema.get("anyOf").and_then(Value::as_array) else {
        return false;
    };
    let [left, right] = arms.as_slice() else {
        return false;
    };
    let string = |node: &Value| {
        node.as_object().is_some_and(|fields| {
            fields.len() == 1 && fields.get("type").and_then(Value::as_str) == Some("string")
        })
    };
    let array = |node: &Value| {
        node.as_object().is_some_and(|fields| {
            fields.len() == 2
                && fields.get("type").and_then(Value::as_str) == Some("array")
                && fields.get("items").is_some_and(string)
        })
    };
    (string(left) && array(right)) || (array(left) && string(right))
}

fn types_within(types: &Value, allowed: &[&str]) -> bool {
    match types {
        Value::String(kind) => allowed.contains(&kind.as_str()),
        Value::Array(kinds) => {
            !kinds.is_empty()
                && kinds
                    .iter()
                    .all(|kind| kind.as_str().is_some_and(|kind| allowed.contains(&kind)))
        }
        _ => false,
    }
}

/// A narrow explicit-type incompatibility proof, not schema satisfiability
/// analysis. Scalar alternatives and tuple prefixes can still be encodable.
pub(super) fn unsupported_scalar_shape(schema: &Value) -> bool {
    types_within(&schema["type"], &["null"])
        || (types_within(&schema["type"], &["array", "null"])
            && schema.get("prefixItems").is_none()
            && types_within(&schema["items"]["type"], &["object", "array", "null"]))
}

fn parameter(p: &Value) -> Result<()> {
    issue(p)?;
    let location = p["in"].as_str().unwrap_or("");
    if location == "cookie" {
        return Err(Unsupported::CookieParameter.into());
    }
    if location == "header" {
        return Ok(());
    }
    if !matches!(location, "path" | "query") || p.get("content").is_some() {
        return Err(Unsupported::ParameterSerialization.into());
    }
    ensure!(
        !unsupported_scalar_shape(&p["schema"]),
        Unsupported::ParameterSerialization
    );
    let types = &p["schema"]["type"];
    let contains = |kind: &str| {
        types == kind
            || types
                .as_array()
                .is_some_and(|types| types.iter().any(|value| value == kind))
    };
    let object = contains("object");
    let query_union = location == "query" && string_or_string_array(&p["schema"]);
    let array = contains("array") || query_union;
    // Other compositions still need an explicit root wire type. Do not infer
    // through arbitrary alternatives or intersect the contract with type:string.
    if types.is_null()
        && !query_union
        && ["allOf", "anyOf", "oneOf", "not", "if", "then", "else"]
            .iter()
            .any(|key| p["schema"].get(*key).is_some())
    {
        return Err(Unsupported::ParameterSerialization.into());
    }
    let style = p
        .get("style")
        .and_then(Value::as_str)
        .unwrap_or(if location == "path" { "simple" } else { "form" });
    let representable = if location == "path" {
        style == "simple" && !array && !object
    } else {
        style == "form"
            && !object
            && (!array || p.get("explode").is_none_or(|value| value == true))
            && p.get("allowReserved").is_none_or(|value| value == false)
    };
    ensure!(representable, Unsupported::ParameterSerialization);
    Ok(())
}

// Borrow a proof-friendly object form. No intermediate JSON copy or guessed
// constraint rewrite: unfamiliar roots remain intact as the whole body schema.
struct FlatBody<'a> {
    properties: BTreeMap<&'a str, &'a Value>,
    required: BTreeSet<&'a str>,
    closed: bool,
    alternatives: BTreeMap<&'static str, Vec<Vec<&'a str>>>,
    metadata: &'a Map<String, Value>,
}
fn flattened(body: &Value) -> Option<FlatBody<'_>> {
    let object = body.as_object()?;
    let known_object = object.get("type").is_some_and(|kind| kind == "object");
    if !known_object
        || !object.keys().all(|key| {
            matches!(
                key.as_str(),
                "type"
                    | "properties"
                    | "required"
                    | "additionalProperties"
                    | "description"
                    | "title"
                    | "allOf"
                    | "oneOf"
                    | "anyOf"
            )
        })
    {
        return None;
    }
    let closed = match object.get("additionalProperties") {
        None | Some(Value::Bool(true)) => false,
        Some(Value::Bool(false)) => true,
        _ => return None,
    };
    let mut properties = BTreeMap::new();
    if let Some(value) = object.get("properties") {
        properties.extend(
            value
                .as_object()?
                .iter()
                .map(|(key, value)| (key.as_str(), value)),
        );
    }
    let required = match object.get("required") {
        None => BTreeSet::new(),
        Some(value) => value
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<BTreeSet<_>>>()?,
    };
    if required.iter().any(|key| !properties.contains_key(key)) {
        return None;
    }
    let mut alternatives = BTreeMap::new();
    for keyword in ["oneOf", "anyOf", "allOf"] {
        if let Some(value) = object.get(keyword) {
            let mut branches = Vec::new();
            for branch in value.as_array()? {
                let branch = branch.as_object()?;
                if branch.len() != 1 {
                    return None;
                }
                let keys = branch
                    .get("required")?
                    .as_array()?
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()?;
                if keys.iter().any(|key| !properties.contains_key(key)) {
                    return None;
                }
                branches.push(keys);
            }
            alternatives.insert(keyword, branches);
        }
    }
    Some(FlatBody {
        properties,
        required,
        closed,
        alternatives,
        metadata: object,
    })
}

pub(super) fn prepare(op: &Value) -> Result<OperationInput> {
    let params = op["params"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut identities = BTreeSet::new();
    for p in params {
        parameter(p)?;
        if matches!(p["in"].as_str(), Some("path" | "query")) {
            ensure!(
                identities.insert((p["in"].as_str(), p["name"].as_str())),
                Unsupported::ParameterSerialization
            );
        }
    }
    let body = &op["body"];
    issue(body)?;
    let has_body = !body.is_null();
    let flat = if has_body && body["required"] == true {
        flattened(&body["schema"])
    } else {
        None
    };
    let external = params.iter().any(|p| {
        matches!(p["in"].as_str(), Some("path" | "query"))
            || p["in"] == "header"
                && p["name"].as_str().is_some_and(|name| {
                    !headers::ignored(&name.to_ascii_lowercase())
                        && !headers::payment(&name.to_ascii_lowercase())
                })
    });
    let flat = flat.filter(|schema| !external || schema.closed);
    let mut args = ArgumentFields::new(Names::new(params, flat.as_ref()));
    for location in ["path", "query"] {
        for p in params.iter().filter(|p| p["in"] == location) {
            let Some(wire) = p["name"].as_str() else {
                continue;
            };
            let note = if location == "path" {
                "Use a scalar value."
            } else {
                "Use a scalar value or an array of scalars."
            };
            let prop = parameter_schema(p, note);
            args.insert(wire, location, prop, p["required"] == true);
        }
    }
    let mut body_arguments = BTreeMap::new();
    let whole_body = if let Some(flat) = &flat {
        for (wire, property) in &flat.properties {
            let name = args.insert(
                wire,
                "body",
                (**property).clone(),
                flat.required.contains(wire),
            );
            body_arguments.insert((*wire).to_owned(), name);
        }
        None
    } else if has_body {
        let name = args.names.argument("body", "body");
        args.props.insert(name.clone(), body["schema"].clone());
        if body["required"] == true {
            args.required.insert(name.clone());
        }
        Some(BodyArgument {
            name,
            required: body["required"] == true,
        })
    } else {
        None
    };
    let header_params = headers::collect(
        &op["params"],
        &mut args.props,
        &mut args.required,
        &mut args.names,
    )
    .map_err(|error| {
        if error.is::<Unsupported>() {
            error
        } else {
            anyhow::Error::new(Unsupported::HeaderContract)
        }
    })?;
    let mut schema = Value::Object(Map::from_iter([
        ("type".into(), json!("object")),
        ("properties".into(), Value::Object(args.props)),
    ]));
    if !args.required.is_empty() {
        schema["required"] = json!(args.required);
    }
    if let Some(flat) = &flat {
        for key in ["description", "title"] {
            if let Some(value) = flat.metadata.get(key) {
                schema[key] = value.clone();
            }
        }
    }
    let body_closed = flat.as_ref().is_some_and(|body| body.closed);
    if whole_body.is_some() || body_closed {
        schema["additionalProperties"] = json!(false);
    }
    if let Some(flat) = &flat {
        for (keyword, branches) in &flat.alternatives {
            schema[*keyword]=Value::Array(branches.iter().map(|required|json!({"required":required.iter().map(|wire|body_arguments[*wire].clone()).collect::<Vec<_>>()})).collect());
        }
    }
    Ok(OperationInput {
        schema,
        routes: args.routes,
        wire_names: args.wire_names,
        header_params,
        has_body,
        body_closed,
        whole_body,
        json_media_type: body
            .get("media_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
