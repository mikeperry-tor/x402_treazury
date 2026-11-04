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
    HeaderName,
    HeaderReserved,
    HeaderMissingName,
    HeaderDuplicate,
    HeaderSerialization,
    IdempotencyKeySchema,
    HeaderArgument,
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
            Self::InputReference => {
                "An input schema reference cannot be represented as a tool input"
            }
            Self::SchemaReferenceSiblings => {
                "Input schema reference siblings cannot be preserved by the current resolver"
            }
            Self::HeaderContract => "The declared HTTP header contract is not supported",
            Self::HeaderName => "An API header name is invalid",
            Self::HeaderReserved => {
                "API header is reserved for HTTP transport, authentication or payment"
            }
            Self::HeaderMissingName => "The catalog contains a header parameter without a name",
            Self::HeaderDuplicate => "The catalog declares the same HTTP header more than once",
            Self::HeaderSerialization => {
                "The catalog uses unsupported HTTP header serialization; expected OpenAPI simple style"
            }
            Self::IdempotencyKeySchema => {
                "The catalog's Idempotency-Key schema does not allow an automatically generated UUID"
            }
            Self::HeaderArgument => {
                "The catalog's HTTP header argument conflicts with another input argument"
            }
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

// Charge only the bounded fields actually inspected by the exact proof.
// Never scan rejected opaque payloads or repeat a separate projection pass.
fn proof_node(budget: &mut expansion::Budget, node: &Value, depth: usize) -> Result<()> {
    if let Some(text) = node.as_str() {
        budget.reference(text, depth)
    } else {
        budget.schema_node(node, depth)
    }
}
fn self_reference(
    node: &Value,
    reference: &str,
    depth: usize,
    budget: &mut expansion::Budget,
) -> Result<bool> {
    let Some(fields) = node.as_object().filter(|fields| fields.len() == 1) else {
        return Ok(false);
    };
    proof_node(budget, node, depth)?;
    let Some(value) = fields.get("$ref").filter(|value| value.is_string()) else {
        return Ok(false);
    };
    if value.as_str().map(str::len) != Some(reference.len()) {
        return Ok(false);
    }
    proof_node(budget, value, depth + 1)?;
    Ok(value.as_str() == Some(reference))
}
// Six disjoint JSON domains with unrestricted recursive members accept every
// finite JSON value. Only this exact shape is proved; other cycles stay errors.
fn unrestricted_json(
    node: &Value,
    reference: &str,
    depth: usize,
    budget: &mut expansion::Budget,
) -> Result<bool> {
    let Some(fields) = node.as_object().filter(|fields| fields.len() <= 2) else {
        return Ok(false);
    };
    let Some(arms) = fields
        .get("oneOf")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 6)
    else {
        return Ok(false);
    };
    if fields.len() != 1 + usize::from(fields.get("description").is_some_and(Value::is_string)) {
        return Ok(false);
    }
    proof_node(budget, node, depth)?;
    proof_node(budget, &fields["oneOf"], depth + 1)?;
    let mut domains = 0u8;
    for arm in arms {
        let Some(properties) = arm.as_object().filter(|v| v.len() <= 3) else {
            return Ok(false);
        };
        proof_node(budget, arm, depth + 2)?;
        let Some(kind) = properties.get("type").filter(|value| value.is_string()) else {
            return Ok(false);
        };
        if !kind
            .as_str()
            .is_some_and(|kind| (4..=7).contains(&kind.len()))
        {
            return Ok(false);
        }
        proof_node(budget, kind, depth + 3)?;
        let domain = match kind.as_str() {
            Some("null") if properties.len() == 1 => 1,
            Some("boolean") if properties.len() == 1 => 2,
            Some("number") if properties.len() == 1 => 4,
            Some("string") if properties.len() == 1 => 8,
            Some("array") if properties.len() == 2 => {
                if !self_reference(&arm["items"], reference, depth + 3, budget)? {
                    return Ok(false);
                }
                16
            }
            Some("object") if properties.len() == 3 => {
                if !self_reference(&arm["additionalProperties"], reference, depth + 3, budget)? {
                    return Ok(false);
                }
                let names = &arm["propertyNames"];
                if !names.as_object().is_some_and(|v| v.len() == 1) {
                    return Ok(false);
                }
                proof_node(budget, names, depth + 3)?;
                let Some(kind) = names.get("type").filter(|v| v.is_string()) else {
                    return Ok(false);
                };
                if kind.as_str().map(str::len) != Some("string".len()) {
                    return Ok(false);
                }
                proof_node(budget, kind, depth + 4)?;
                if kind.as_str() != Some("string") {
                    return Ok(false);
                }
                32
            }
            _ => return Ok(false),
        };
        if domains & domain != 0 {
            return Ok(false);
        }
        domains |= domain;
    }
    // Six validated arms, six possible bits, and no duplicates imply all domains.
    Ok(true)
}
fn standard_dialect(value: &Value) -> bool {
    matches!(
        value.as_str(),
        Some(
            "https://spec.openapis.org/oas/3.1/dialect/base"
                | "https://json-schema.org/draft/2020-12/schema"
        )
    )
}
#[derive(Clone, Copy)]
struct SchemaContext {
    resource: bool,
    standard: bool,
}
impl SchemaContext {
    fn root(root: &Value) -> Self {
        Self {
            resource: false,
            standard: root.get("jsonSchemaDialect").is_none_or(standard_dialect),
        }
    }
    fn enter(self, node: &Value) -> Self {
        Self {
            resource: self.resource || node.get("$id").is_some_and(Value::is_string),
            standard: self.standard
                && node.get("$schema").is_none_or(standard_dialect)
                && node.get("$vocabulary").is_none(),
        }
    }
    fn merge(self, other: Self) -> Self {
        Self {
            resource: self.resource || other.resource,
            standard: self.standard && other.standard,
        }
    }
}
// JSON Schema2020-12 basic annotations collect alongside the target; they do
// not override its assertions or apply defaults. Only inspect the bounded bag
// here: annotation payloads remain opaque and are charged before output copying.
fn reference_annotations<'a>(
    root: &Value,
    node: &'a Value,
    context: SchemaContext,
) -> Option<&'a serde_json::Map<String, Value>> {
    if !context.standard
        || root["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.0"))
    {
        return None;
    }
    let fields = node.as_object()?;
    if fields.len() < 2 {
        return None;
    }
    fields
        .iter()
        .all(|(key, value)| match key.as_str() {
            "$ref" | "default" => true,
            "title" | "description" => value.is_string(),
            "examples" => value.is_array(),
            "deprecated" | "readOnly" | "writeOnly" => value.is_boolean(),
            _ => false,
        })
        .then_some(fields)
}

pub(super) fn schema(root: &Value, node: &Value, budget: &mut expansion::Budget) -> Result<Value> {
    projected(
        root,
        node,
        SchemaContext::root(root).enter(node),
        0,
        budget,
        &mut BTreeSet::new(),
    )
}

// Every descent charges its strictly increasing depth before looking up a ref or
// constructing output. The shared depth bound also bounds this recursion, even
// for long flat reference chains. Opaque copies are inspected before cloning.
fn projected<'a>(
    root: &'a Value,
    node: &'a Value,
    context: SchemaContext,
    depth: usize,
    budget: &mut expansion::Budget,
    active: &mut BTreeSet<&'a str>,
) -> Result<Value> {
    budget.schema_node(node, depth)?;
    if node.is_boolean() {
        return Ok(node.clone());
    }
    let fields = node.as_object().ok_or(Unsupported::InputSchema)?;
    if let Some(value) = fields.get("$ref") {
        if context.resource {
            return Err(Unsupported::InputReference.into());
        }
        let annotations = reference_annotations(root, node, context);
        if root["openapi"]
            .as_str()
            .is_none_or(|version| !version.starts_with("3.0"))
            && fields.len() != 1
            && annotations.is_none()
        {
            return Err(Unsupported::SchemaReferenceSiblings.into());
        }
        let reference = value
            .as_str()
            .filter(|value| value.starts_with("#/"))
            .ok_or(Unsupported::InputReference)?;
        budget.reference(reference, depth + 1)?;
        let (target, target_context) = schema_target(root, reference)?;
        let target_context = context.merge(target_context).enter(target);
        if annotations.is_some() && !target_context.standard {
            return Err(Unsupported::SchemaReferenceSiblings.into());
        }
        let target_depth = if let Some(annotations) = annotations {
            let keys: Vec<_> = annotations
                .keys()
                .filter(|key| *key != "$ref")
                .map(String::as_str)
                .collect();
            budget.generated_node(&["allOf"], depth)?;
            budget.generated_node(&[], depth + 1)?;
            budget.generated_node(&keys, depth + 2)?;
            for (key, value) in annotations {
                if key != "$ref" {
                    budget.copying_at(value, depth + 3)?;
                }
            }
            depth + 2
        } else {
            depth + 1
        };
        let target = if !target_context.resource
            && target_context.standard
            && unrestricted_json(target, reference, target_depth, budget)?
        {
            if let Some(description) = target.get("description") {
                budget.generated_node(&["description"], target_depth)?;
                budget.copying_at(description, target_depth + 1)?;
                Value::Object(serde_json::Map::from_iter([(
                    "description".into(),
                    description.clone(),
                )]))
            } else {
                budget.generated_node(&[], target_depth)?;
                Value::Object(serde_json::Map::new())
            }
        } else {
            if !active.insert(reference) {
                return Err(Unsupported::InputReference.into());
            }
            let result = projected(root, target, target_context, target_depth, budget, active);
            active.remove(reference);
            result?
        };
        return Ok(if let Some(annotations) = annotations {
            let annotations: serde_json::Map<_, _> = annotations
                .iter()
                .filter(|(key, _)| *key != "$ref")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            Value::Object(serde_json::Map::from_iter([(
                "allOf".into(),
                Value::Array(vec![target, Value::Object(annotations)]),
            )]))
        } else {
            target
        });
    }
    // OpenAPI3.0 ref replacement above still ignores unused siblings.
    if fields.contains_key("$dynamicRef") || fields.contains_key("$recursiveRef") {
        return Err(Unsupported::InputReference.into());
    }
    let mut output = serde_json::Map::new();
    for (key, value) in fields {
        let Some(value) = exclusive(node, key, value) else {
            continue;
        };
        let value = if key == "type" && nullable(root, node).is_some() {
            budget.generated_node(&[], depth + 1)?;
            budget.copying_at(value, depth + 2)?;
            budget.copying_at(&Value::String("null".into()), depth + 2)?;
            Value::Array(vec![value.clone(), Value::String("null".into())])
        } else {
            match position(key, value) {
                Position::Map => {
                    let values = value.as_object().ok_or(Unsupported::InputSchema)?;
                    budget.schema_node(value, depth + 1)?;
                    let mut properties = serde_json::Map::new();
                    for (name, value) in values {
                        let value = projected(
                            root,
                            value,
                            context.enter(value),
                            depth + 2,
                            budget,
                            active,
                        )?;
                        properties.insert(name.clone(), value);
                    }
                    Value::Object(properties)
                }
                Position::List => {
                    let values = value.as_array().ok_or(Unsupported::InputSchema)?;
                    budget.schema_node(value, depth + 1)?;
                    let mut children = Vec::new();
                    for value in values {
                        children.push(projected(
                            root,
                            value,
                            context.enter(value),
                            depth + 2,
                            budget,
                            active,
                        )?);
                    }
                    Value::Array(children)
                }
                Position::Schema => {
                    projected(root, value, context.enter(value), depth + 1, budget, active)?
                }
                Position::Opaque => {
                    budget.copying_at(value, depth + 1)?;
                    value.clone()
                }
            }
        };
        output.insert(key.clone(), value);
    }
    Ok(Value::Object(output))
}
// Walk only this used pointer. Serde's one-token pointer operation retains its
// exact escaping/index semantics; no URI resolver or unused-resource scan.
// The caller charges the whole pointer before lookup and lexical-context inspection.
fn schema_target<'a>(root: &'a Value, reference: &str) -> Result<(&'a Value, SchemaContext)> {
    let pointer = reference
        .strip_prefix("#/")
        .ok_or(Unsupported::InputReference)?;
    let mut fields = pointer.split('/').peekable();
    let mut node = root;
    let component_schemas = root
        .get("components")
        .and_then(|value| value.get("schemas"));
    let mut context = SchemaContext {
        resource: false,
        standard: true,
    };
    while let Some(field) = fields.next() {
        node = node
            .pointer(&format!("/{field}"))
            .ok_or(Unsupported::InputReference)?;
        // Carry lexical context only: a selected no-ref target is already
        // known exactly and needs no base-dependent resolution. The catalog
        // root is metadata; the caller derives the final target's context once.
        if fields.peek().is_some() {
            context.resource |= node.get("$id").is_some_and(Value::is_string);
            // This known map contains schema NAMES, even if an unused entry
            // has a malformed string value. Only selected schemas are checked.
            if component_schemas.is_none_or(|schemas| !std::ptr::eq(schemas, node))
                && let Some(dialect) = node.get("$schema").filter(|value| value.is_string())
            {
                context.standard &= standard_dialect(dialect);
            }
        }
        // Do not infer $vocabulary from ancestor maps: an unused component
        // may simply be named $vocabulary. Actual schema nodes use enter().
    }
    Ok((node, context))
}
