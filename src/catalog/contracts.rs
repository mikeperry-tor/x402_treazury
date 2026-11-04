//! Fixed operation compatibility results, separate from fatal catalog failures.
use super::{ToolSpec, expansion};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unsupported {
    CookieParameter,
    ParameterSerialization,
    RequestMedia,
    InputReference,
    InputSchema,
    SchemaReferenceSiblings,
    HeaderContract,
}
impl Unsupported {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::CookieParameter => "Cookie parameters are not supported",
            Self::ParameterSerialization => {
                "The declared parameter serialization cannot be represented"
            }
            Self::RequestMedia => "No offered request representation supports UTF-8 JSON encoding",
            Self::InputSchema => "An input schema is not an object or boolean JSON schema",
            Self::InputReference => "An input schema reference cannot be resolved for a tool input",
            Self::SchemaReferenceSiblings => {
                "Input schema reference siblings cannot be preserved by the current resolver"
            }
            Self::HeaderContract => "The declared HTTP header contract is not supported",
        }
    }
}
impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.explanation())
    }
}
impl std::error::Error for Unsupported {}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OperationIssue {
    pub method: String,
    pub path: String,
    pub reason: Unsupported,
}
#[derive(Clone, Debug, Default)]
pub struct CatalogBuild {
    pub tools: Vec<ToolSpec>,
    pub issues: Vec<OperationIssue>,
}
#[derive(Debug)]
pub struct IncompleteCatalog(pub Vec<OperationIssue>);
impl std::fmt::Display for IncompleteCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Catalog omits {} unsupported API operations",
            self.0.len()
        )?;
        for issue in &self.0 {
            write!(f, "; {} {}: {}", issue.method, issue.path, issue.reason)?;
        }
        Ok(())
    }
}
impl std::error::Error for IncompleteCatalog {}
impl CatalogBuild {
    pub fn complete(self) -> Result<Vec<ToolSpec>> {
        if !self.issues.is_empty() {
            return Err(IncompleteCatalog(self.issues).into());
        }
        Ok(self.tools)
    }
}
pub fn omission_instructions(issues: &[OperationIssue]) -> Option<String> {
    if issues.is_empty() {
        return None;
    }
    let reasons: BTreeSet<_> = issues
        .iter()
        .map(|issue| issue.reason.explanation())
        .collect();
    Some(format!(
        "{} API operations are omitted: {}. Inspect catalog or registration details for methods and paths.",
        issues.len(),
        reasons.into_iter().collect::<Vec<_>>().join("; ")
    ))
}
pub fn append_omissions(text: &mut Option<String>, source: &str, issues: &[OperationIssue]) {
    if let Some(summary) = omission_instructions(issues) {
        let text = text.get_or_insert_default();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("[{source}] {summary}"));
    }
}

/// Existing Reference Object semantics; schema provenance is inspected separately.
pub(super) fn reference<'a>(
    root: &'a Value,
    mut node: &'a Value,
    budget: &mut expansion::Budget,
) -> Result<&'a Value> {
    let mut active = BTreeSet::new();
    let mut depth = 0;
    while let Some(value) = node.get("$ref") {
        let reference = value
            .as_str()
            .filter(|reference| reference.starts_with("#/"))
            .ok_or(Unsupported::InputReference)?;
        depth += 1;
        budget.reference(reference, depth)?;
        if !active.insert(reference) {
            return Err(Unsupported::InputReference.into());
        }
        node = root
            .pointer(&reference[1..])
            .ok_or(Unsupported::InputReference)?;
    }
    Ok(node)
}

#[derive(Clone, Copy)]
enum Position {
    Schema,
    Map,
    List,
    Opaque,
}
fn position(key: &str, value: &Value) -> Position {
    match key {
        "properties" | "patternProperties" | "dependentSchemas" => Position::Map,
        "allOf" | "oneOf" | "anyOf" | "prefixItems" => Position::List,
        "items" if value.is_array() => Position::List,
        "items"
        | "additionalItems"
        | "contains"
        | "not"
        | "if"
        | "then"
        | "else"
        | "additionalProperties"
        | "unevaluatedProperties"
        | "unevaluatedItems"
        | "propertyNames" => Position::Schema,
        _ => Position::Opaque,
    }
}
fn exclusive<'a>(node: &'a Value, key: &str, value: &'a Value) -> Option<&'a Value> {
    if !matches!(key, "exclusiveMinimum" | "exclusiveMaximum") {
        return Some(value);
    }
    let bound = if value == &Value::Bool(true) {
        node.get(if key == "exclusiveMinimum" {
            "minimum"
        } else {
            "maximum"
        })
    } else {
        Some(value)
    };
    bound.filter(|value| value.is_number())
}
fn nullable<'a>(root: &Value, node: &'a Value) -> Option<&'a str> {
    (root["openapi"]
        .as_str()
        .is_some_and(|version| version.starts_with("3.0"))
        && node["nullable"] == true)
        .then(|| node["type"].as_str())
        .flatten()
}

enum SchemaWork<'a> {
    Visit(&'a Value, usize, bool),
    Leave(&'a str),
    Fields(&'a Value, serde_json::map::Iter<'a>, usize, bool),
    Map(serde_json::map::Iter<'a>, usize, bool),
    List(std::slice::Iter<'a, Value>, usize, bool),
}
/// Validate actual schema expansion/copy work before allocating the projection.
/// Cursor frames retain O(depth) pending work, not every uncharged breadth child.
/// Opaque annotations/unused definitions copy unchanged; refs charge both passes.
pub(super) fn schema_provenance(
    root: &Value,
    node: &Value,
    budget: &mut expansion::Budget,
) -> Result<()> {
    let schema_siblings = root["openapi"]
        .as_str()
        .is_none_or(|version| !version.starts_with("3.0"));
    let mut pending = vec![SchemaWork::Visit(node, 0, false)];
    let mut active = BTreeSet::new();
    while let Some(work) = pending.pop() {
        let (node, depth, resource) = match work {
            SchemaWork::Leave(reference) => {
                active.remove(reference);
                continue;
            }
            SchemaWork::Map(mut fields, depth, resource) => {
                if let Some((_, value)) = fields.next() {
                    pending.push(SchemaWork::Map(fields, depth, resource));
                    pending.push(SchemaWork::Visit(value, depth, resource));
                }
                continue;
            }
            SchemaWork::List(mut values, depth, resource) => {
                if let Some(value) = values.next() {
                    pending.push(SchemaWork::List(values, depth, resource));
                    pending.push(SchemaWork::Visit(value, depth, resource));
                }
                continue;
            }
            SchemaWork::Fields(node, mut fields, depth, resource) => {
                let Some((key, value)) = fields.next() else {
                    continue;
                };
                pending.push(SchemaWork::Fields(node, fields, depth, resource));
                let Some(value) = exclusive(node, key, value) else {
                    continue;
                };
                if key == "type" && nullable(root, node).is_some() {
                    budget.schema_node(&Value::Null, depth + 1)?;
                    budget.copying_at(value, depth + 2)?;
                    budget.copying_at(&Value::String("null".into()), depth + 2)?;
                    continue;
                }
                match position(key, value) {
                    Position::Map => {
                        let values = value.as_object().ok_or(Unsupported::InputSchema)?;
                        budget.schema_node(value, depth + 1)?;
                        pending.push(SchemaWork::Map(values.iter(), depth + 2, resource));
                    }
                    Position::List => {
                        let values = value.as_array().ok_or(Unsupported::InputSchema)?;
                        budget.schema_node(value, depth + 1)?;
                        pending.push(SchemaWork::List(values.iter(), depth + 2, resource));
                    }
                    Position::Schema => pending.push(SchemaWork::Visit(value, depth + 1, resource)),
                    Position::Opaque => budget.copying_at(value, depth + 1)?,
                }
                continue;
            }
            SchemaWork::Visit(node, depth, resource) => (node, depth, resource),
        };
        budget.schema_node(node, depth)?;
        if node.is_boolean() {
            continue;
        }
        let fields = node.as_object().ok_or(Unsupported::InputSchema)?;
        // Only string-valued IDs on actual schema nodes establish resource
        // context. Property/definition names and opaque data are not schemas.
        let resource = resource || fields.get("$id").is_some_and(Value::is_string);
        if let Some(value) = fields.get("$ref") {
            if resource {
                return Err(Unsupported::InputReference.into());
            }
            if schema_siblings && fields.len() != 1 {
                return Err(Unsupported::SchemaReferenceSiblings.into());
            }
            let reference = value
                .as_str()
                .filter(|value| value.starts_with("#/"))
                .ok_or(Unsupported::InputReference)?;
            budget.reference(reference, depth + 1)?;
            budget.reference(reference, depth + 1)?;
            if !active.insert(reference) {
                return Err(Unsupported::InputReference.into());
            }
            let (target, target_resource) = schema_target(root, reference)?;
            pending.push(SchemaWork::Leave(reference));
            pending.push(SchemaWork::Visit(
                target,
                depth + 1,
                resource || target_resource,
            ));
            continue;
        }
        pending.push(SchemaWork::Fields(node, fields.iter(), depth, resource));
    }
    Ok(())
}

pub(super) fn schema(root: &Value, node: &Value, budget: &mut expansion::Budget) -> Result<Value> {
    schema_provenance(root, node, budget)?;
    projected(root, node)
}
// Walk only this used pointer. Serde's one-token pointer operation retains its
// exact escaping/index semantics; no URI resolver or unused-resource scan.
// The caller charges the whole pointer before either preflight/projection walk.
fn schema_target<'a>(root: &'a Value, reference: &str) -> Result<(&'a Value, bool)> {
    let pointer = reference
        .strip_prefix("#/")
        .ok_or(Unsupported::InputReference)?;
    let mut fields = pointer.split('/').peekable();
    let mut node = root;
    let mut resource = false;
    while let Some(field) = fields.next() {
        node = node
            .pointer(&format!("/{field}"))
            .ok_or(Unsupported::InputReference)?;
        // Carry lexical context only: a selected no-ref target is already
        // known exactly and needs no base-dependent resolution. The catalog
        // root is metadata; the final target's own ID is handled by provenance.
        resource |= fields.peek().is_some() && node.get("$id").is_some_and(Value::is_string);
    }
    Ok((node, resource))
}

// Preflight proved finite references and depth before recursion, pointer lookups
// and opaque Value::clone. Its same budget includes both pointer parses.
fn projected(root: &Value, node: &Value) -> Result<Value> {
    let Some(fields) = node.as_object() else {
        return Ok(node.clone());
    };
    if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
        let (target, _) = schema_target(root, reference)?;
        return projected(root, target);
    }
    let mut output = serde_json::Map::new();
    for (key, value) in fields {
        let Some(value) = exclusive(node, key, value) else {
            continue;
        };
        let value = match position(key, value) {
            Position::Map => Value::Object(
                value
                    .as_object()
                    .ok_or(Unsupported::InputSchema)?
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), projected(root, value)?)))
                    .collect::<Result<_>>()?,
            ),
            Position::List => Value::Array(
                value
                    .as_array()
                    .ok_or(Unsupported::InputSchema)?
                    .iter()
                    .map(|value| projected(root, value))
                    .collect::<Result<_>>()?,
            ),
            Position::Schema => projected(root, value)?,
            Position::Opaque => value.clone(),
        };
        output.insert(key.clone(), value);
    }
    if nullable(root, node).is_some()
        && let Some(kind) = output.remove("type")
    {
        output.insert(
            "type".into(),
            Value::Array(vec![kind, Value::String("null".into())]),
        );
    }
    Ok(Value::Object(output))
}
