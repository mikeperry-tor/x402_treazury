//! Stage complete vendor copies only. Acceptance uses ordinary reviewed patches.
use super::*;
use serde_json::{Value, json};

// Only reference-bearing OpenAPI objects and schema-valued keywords are visited.
// Maps name objects; their keys (including "$ref") are never interpreted as fields.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Object {
    Document,
    Components,
    Paths,
    PathItem,
    Operation,
    Responses,
    Response,
    RequestBody,
    Parameter,
    MediaType,
    Encoding,
    Callback,
    Schema,
    Leaf,
}
#[derive(Clone, Copy)]
enum Position {
    Node(Object),
    Collection(Object),
}
fn references(root: &Value) -> Result<()> {
    visit(
        root,
        root,
        Position::Node(Object::Document),
        &mut Default::default(),
    )
}
fn visit(
    root: &Value,
    node: &Value,
    context: Position,
    visited: &mut std::collections::BTreeSet<(String, Object)>,
) -> Result<()> {
    let kind = match context {
        Position::Collection(kind) => {
            for child in node
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
                .chain(node.as_array().into_iter().flatten())
            {
                visit(root, child, Position::Node(kind), visited)?;
            }
            return Ok(());
        }
        Position::Node(kind) => kind,
    };
    let Some(map) = node.as_object() else {
        return Ok(());
    };
    use Object::*;
    if matches!(
        kind,
        PathItem | Response | RequestBody | Parameter | Callback | Schema | Leaf
    ) && let Some(reference) = map.get("$ref")
    {
        let reference = reference.as_str().context("non-string reference")?;
        let fragment = reference
            .strip_prefix('#')
            .context("external reference requires authored closure")?;
        let pointer = percent_encoding::percent_decode_str(fragment).decode_utf8()?;
        let target = root
            .pointer(&pointer)
            .context("unresolved local reference")?;
        // A reference gives even an extension-hosted target its semantic context.
        // Include the context so a prior opaque/leaf visit cannot hide schema refs.
        if visited.insert((pointer.into_owned(), kind)) {
            visit(root, target, Position::Node(kind), visited)?;
        }
    }
    for (key, child) in map {
        if let Some(context) = child_context(kind, key) {
            visit(root, child, context, visited)?;
        }
    }
    Ok(())
}
fn child_context(kind: Object, key: &str) -> Option<Position> {
    use Object::*;
    use Position::{Collection, Node};
    Some(match (kind, key) {
        (Document, "paths") => Node(Paths),
        (Document, "webhooks") => Collection(PathItem),
        (Document, "components") => Node(Components),
        (Paths, path) if path.starts_with('/') => Node(PathItem),
        (Responses, field) if !field.starts_with("x-") => Node(Response),
        (Callback, expression) if !expression.starts_with("x-") => Node(PathItem),
        (Components, "schemas") => Collection(Schema),
        (Components, "responses") => Collection(Response),
        (Components, "parameters" | "headers") => Collection(Parameter),
        (Components, "requestBodies") => Collection(RequestBody),
        (Components, "examples" | "links" | "securitySchemes") => Collection(Leaf),
        (Components, "callbacks") => Collection(Callback),
        (Components, "pathItems") => Collection(PathItem),
        (PathItem, "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace") => {
            Node(Operation)
        }
        (PathItem | Operation, "parameters") => Collection(Parameter),
        (Operation, "requestBody") => Node(RequestBody),
        (Operation, "responses") => Node(Responses),
        (Operation, "callbacks") => Collection(Callback),
        (Response | RequestBody | Parameter, "content") => Collection(MediaType),
        (Response, "headers") => Collection(Parameter),
        (Response, "links") => Collection(Leaf),
        (Parameter | MediaType, "schema") => Node(Schema),
        (Parameter | MediaType, "examples") => Collection(Leaf),
        (MediaType, "encoding") => Collection(Encoding),
        (Encoding, "headers") => Collection(Parameter),
        (
            Schema,
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas"
            | "dependencies",
        ) => Collection(Schema),
        (Schema, "allOf" | "anyOf" | "oneOf" | "prefixItems") => Collection(Schema),
        (
            Schema,
            "items"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contains"
            | "additionalProperties"
            | "additionalItems"
            | "unevaluatedProperties"
            | "propertyNames"
            | "unevaluatedItems"
            | "contentSchema",
        ) => Node(Schema),
        _ => return None,
    })
}
fn candidate(run: &Path, provider: &Provider) -> Result<Option<Vec<u8>>> {
    let Some(evidence) = &provider.evidence else {
        return Ok(None);
    };
    if evidence.intent != Intent::VendorCopy {
        return Ok(None);
    }
    ensure!(
        provider.inputs.iter().all(|r| r.hash.is_some()),
        "incomplete evidence"
    );
    ensure!(
        provider.asset.is_some(),
        "vendor-copy intent requires a local asset"
    );
    let schemas: Vec<_> = provider
        .inputs
        .iter()
        .filter(|r| r.purpose == "schema")
        .collect();
    ensure!(
        schemas.len() == 1,
        "vendor-copy requires exactly one complete schema authority"
    );
    let raw = bytes(run, schemas[0])?;
    let root: Value = serde_json::from_slice(&raw)?;
    ensure!(
        root["paths"].is_object() && root["openapi"].is_string(),
        "complete OpenAPI document required"
    );
    references(&root)?;
    let cfg = provider.config.as_ref().context("missing config")?;
    let prefix = cfg.prefix.clone().map(Ok).unwrap_or_else(|| {
        x402_treazury::catalog::default_prefix(cfg.base_url.as_deref().context("missing base URL")?)
    })?;
    x402_treazury::catalog::build_tools(cfg, &root, &prefix)?;
    Ok(Some(raw))
}
pub(super) fn stage(run: &Path, provider: &Provider, index: usize) -> Result<Value> {
    match candidate(run, provider) {
        Ok(Some(raw)) => {
            let name = format!("candidate-{index}.json");
            std::fs::write(run.join(&name), &raw)?;
            Ok(
                json!({"outcome":"candidate ready; human review pending","file":name,"sha256":hash(&raw),"target":provider.asset.as_ref().map(|r|&r.source),"expected_preimage":provider.asset.as_ref().and_then(|r|r.hash.as_ref()),"accepted":false}),
            )
        }
        Ok(None) => Ok(
            json!({"outcome":"authored review needed; no automatic asset replacement","accepted":false}),
        ),
        Err(_) => Ok(
            json!({"outcome":"authored review needed: incomplete evidence, compilation or reference closure","accepted":false}),
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_reference_keys_are_data_but_real_example_and_schema_refs_need_closure() {
        let mut root = json!({"openapi":"3.1.0","paths":{"/one":{"get":{"responses":{"200":{
            "description":"literal response data", "content":{"application/json":{
                "example":{"$ref":"https://example.com/item"},
                "examples":{"$ref":{"value":{"$ref":"literal payload"}}},
                "schema":{"type":"object","properties":{"$ref":{"type":"string"},"example":{"$ref":"#/components/schemas/Value"}}}
            }}, "x-evidence":{"$ref":"opaque extension"}
        }}}}}, "components":{"schemas":{"Value":{
            "type":"string", "default":{"$ref":"literal default"}, "enum":[{"$ref":"literal enum"}],
            "const":{"$ref":"literal const"}, "examples":[{"$ref":"literal schema example"}],
            "x-extension":{"$ref":"opaque extension"}
        }}},"x-evidence":{"$ref":"opaque document extension"}});
        assert!(references(&root).is_ok());
        let schema = "/components/schemas/Value";
        for nested in [
            json!({"allOf":[{"$ref":"#/missing"}]}),
            json!({"properties":{"example":{"$ref":"https://example.com/schema"}}}),
            json!({"$defs":{"$ref":{"$ref":"#/missing"}}}),
            json!({"if":{"type":"string"},"then":{"$ref":"#/missing"}}),
            json!({"$ref":{"type":"string"}}),
        ] {
            *root.pointer_mut(schema).unwrap() = nested;
            assert!(references(&root).is_err());
        }
        *root.pointer_mut(schema).unwrap() = json!({"type":"string"});
        let examples = root
            .pointer_mut("/paths/~1one/get/responses/200/content/application~1json/examples")
            .unwrap();
        *examples = json!({"actual":{"$ref":"https://example.com/example"}});
        assert!(references(&root).is_err());
        *root
            .pointer_mut("/paths/~1one/get/responses/200/content/application~1json/examples")
            .unwrap() = json!({"actual":{"$ref":"#/missing"}});
        assert!(references(&root).is_err());
    }
    #[test]
    fn referenced_extension_targets_are_checked_in_context_and_cycles_terminate() {
        let mut root = json!({"openapi":"3.1.0", "paths":{"/one":{"get":{"responses":{"200":{
            "content":{"application/json":{"schema":{"$ref":"#/x-schema"}}}
        }}}}}, "x-schema":{"properties":{"child":{"$ref":"#/x-other"}}},
        "x-other":{"$ref":"#/x-schema"},
        "x-unused":{"$ref":"https://example.com/opaque-data"}});
        assert!(references(&root).is_ok());
        root["x-other"] = json!({"$ref":"https://example.com/schema"});
        assert!(references(&root).is_err());
        root["x-other"] = json!({"$ref":"#/missing"});
        assert!(references(&root).is_err());
        // Visiting the same target as an Example object first must not suppress
        // its later traversal as a Schema object.
        root["components"]["examples"]["first"] = json!({"$ref":"#/x-schema"});
        assert!(references(&root).is_err());
        root["x-other"] = json!({"type":"object", "default":{"$ref":"literal default"}});
        assert!(references(&root).is_ok());
    }
    #[test]
    fn full_copy_retains_responses_and_rejects_unclosed_references() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        let mut root = json!({"openapi":"3.0.0","paths":{"/one":{"get":{"responses":{"200":{"description":"complete response","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Result"}}}}}}}},"components":{"schemas":{"Result":{"type":"string"}}}});
        root["paths"]["/one"]["get"]["responses"]["200"]["content"]["application/json"]["example"] =
            json!({"$ref":"https://example.com/literal-response-value"});
        let raw = serde_json::to_vec(&root).unwrap();
        let read = Read {
            source: "https://example.com/openapi.json".into(),
            purpose: "schema".into(),
            hash: Some(store(run, &raw).unwrap()),
            outcome: "captured".into(),
        };
        let mut provider = Provider {
            id: "synthetic".into(),
            path: "synthetic.toml".into(),
            config: Some(Config {
                prefix: Some("demo".into()),
                ..Default::default()
            }),
            inputs: vec![read.clone()],
            asset: Some(read),
            evidence: Some(Evidence {
                intent: Intent::VendorCopy,
                guidance: "guide".into(),
                sources: vec![],
            }),
            outcome: "captured".into(),
        };
        assert_eq!(candidate(run, &provider).unwrap(), Some(raw.clone()));
        let result = stage(run, &provider, 0).unwrap();
        assert_eq!(
            std::fs::read(run.join(result["file"].as_str().unwrap())).unwrap(),
            raw
        );
        assert_eq!(result["accepted"], false);
        let target = run.join("tracked-schema.json");
        std::fs::write(&target, b"intervening user edit").unwrap();
        provider.asset.as_mut().unwrap().source = target.to_str().unwrap().into();
        let stale = stage(run, &provider, 1).unwrap();
        assert_ne!(
            stale["expected_preimage"],
            hash(&std::fs::read(&target).unwrap())
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"intervening user edit");
        provider.evidence.as_mut().unwrap().intent = Intent::Authored;
        assert!(candidate(run, &provider).unwrap().is_none());
        root["components"]["schemas"] = json!({});
        assert!(references(&root).is_err());
        root["paths"]["/one"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"] = json!("https://example.com/external.json");
        assert!(references(&root).is_err());
        let recursive = json!({"components":{"schemas":{"Node":{"properties":{"child":{"$ref":"#/components/schemas/Node"}}}}}});
        assert!(references(&recursive).is_ok());
    }
}
