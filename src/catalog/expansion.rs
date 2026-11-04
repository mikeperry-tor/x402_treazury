//! Bound catalog-owned copying and expansion before recursive consumers run.
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeSet;

const DEPTH: usize = 64;
const NODES: usize = 500_000;
const BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Budget {
    nodes: usize,
    bytes: usize,
}

#[derive(Clone, Copy)]
enum Mode {
    Clone,
    Import,
}

enum Frame<'a> {
    Visit(&'a Value, usize, Mode),
    LeaveReference(&'a str),
    Object(serde_json::map::Iter<'a>, usize, Mode),
    Array(std::slice::Iter<'a, Value>, usize, Mode),
}

#[derive(Debug)]
pub(crate) struct ComplexityLimit {
    pub depth: usize,
    pub nodes: usize,
    pub bytes: usize,
}
impl std::fmt::Display for ComplexityLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "document_complexity_limit: depth {}, nodes {}, traversal/copy bytes {}; bounds 64, 500000, 67108864",
            self.depth, self.nodes, self.bytes
        )
    }
}
impl std::error::Error for ComplexityLimit {}

fn rejected(depth: usize, nodes: usize, bytes: usize) -> anyhow::Error {
    tracing::warn!(
        category = "document_complexity_limit",
        resource = if depth > DEPTH {
            "depth"
        } else if nodes > NODES {
            "nodes"
        } else {
            "traversal_copy_bytes"
        },
        depth,
        nodes,
        traversal_copy_bytes = bytes,
        depth_limit = DEPTH,
        node_limit = NODES,
        traversal_copy_byte_limit = BYTES,
        "Catalog rejected because its structural expansion or copy budget was exceeded; no partial catalog was accepted"
    );
    anyhow::Error::new(ComplexityLimit { depth, nodes, bytes })
        .context(super::diagnostics::Problem(
            "Catalog document exceeds structural safety bounds: depth 64, 500000 nodes or 67108864 traversal/copy bytes (document_complexity_limit)",
        ))
}

impl Budget {
    fn node(&mut self, depth: usize) -> Result<()> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| rejected(depth, usize::MAX, self.bytes))?;
        if depth > DEPTH || self.nodes > NODES {
            return Err(rejected(depth, self.nodes, self.bytes));
        }
        Ok(())
    }

    fn bytes(&mut self, count: usize, depth: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(count)
            .ok_or_else(|| rejected(depth, self.nodes, usize::MAX))?;
        if self.bytes > BYTES {
            return Err(rejected(depth, self.nodes, self.bytes));
        }
        Ok(())
    }

    pub(crate) fn copying(&mut self, node: &Value) -> Result<()> {
        self.inspect(node, node, Mode::Clone, 0)
    }

    /// Charge borrowed text before parsing or allocating its owned copy.
    pub(crate) fn copying_text(&mut self, text: &str, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.bytes(text.len(), depth)
    }

    /// Charge an iterative reference-only consumer before it compares or
    /// parses a local JSON pointer (for example original Idempotency-Key schema
    /// lookup). It shares the conversion budget rather than a second counter.
    pub(crate) fn reference(&mut self, reference: &str, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.bytes(reference.len(), depth)
    }

    /// Media precedence can revisit ranges and parameter names for each
    /// candidate. Charge comparison work before inspecting either string.
    pub(crate) fn comparison(&mut self, left: &str, right: &str) -> Result<()> {
        self.node(0)?;
        self.bytes(left.len(), 0)?;
        self.bytes(right.len(), 0)
    }

    /// Charge a schema projection before allocating output or descending; depth
    /// bounds recursive projection before a long reference chain can grow its stack.
    pub(crate) fn schema_node(&mut self, node: &Value, depth: usize) -> Result<()> {
        self.node(depth)?;
        if let Value::Object(fields) = node {
            for key in fields.keys() {
                self.bytes(key.len(), depth)?;
            }
        }
        Ok(())
    }

    /// Charge a generated object/array node and its fixed keys without first
    /// allocating a dummy JSON value. Shares the existing traversal/copy budget.
    pub(crate) fn generated_node(&mut self, keys: &[&str], depth: usize) -> Result<()> {
        self.node(depth)?;
        for key in keys {
            self.bytes(key.len(), depth)?;
        }
        Ok(())
    }

    pub(crate) fn copying_at(&mut self, node: &Value, depth: usize) -> Result<()> {
        self.inspect(node, node, Mode::Clone, depth)
    }

    fn inspect<'a>(
        &mut self,
        root: &'a Value,
        node: &'a Value,
        mode: Mode,
        initial_depth: usize,
    ) -> Result<()> {
        let mut frames = vec![Frame::Visit(node, initial_depth, mode)];
        let mut active = BTreeSet::new();
        while let Some(frame) = frames.pop() {
            // Cursor advancement schedules one child at a time. Actual value
            // work is outside this match, so all policies use one flat path.
            let (node, depth, mode) = match frame {
                Frame::Visit(node, depth, mode) => (node, depth, mode),
                Frame::LeaveReference(reference) => {
                    active.remove(reference);
                    continue;
                }
                Frame::Object(mut fields, depth, mode) => {
                    if let Some((key, child)) = fields.next() {
                        self.bytes(key.len(), depth)?;
                        frames.push(Frame::Object(fields, depth, mode));
                        frames.push(Frame::Visit(child, depth + 1, mode));
                    }
                    continue;
                }
                Frame::Array(mut values, depth, mode) => {
                    if let Some(child) = values.next() {
                        frames.push(Frame::Array(values, depth, mode));
                        frames.push(Frame::Visit(child, depth + 1, mode));
                    }
                    continue;
                }
            };
            self.node(depth)?;
            let map = match node {
                Value::Object(map) => map,
                Value::Array(values) => {
                    frames.push(Frame::Array(values.iter(), depth, mode));
                    continue;
                }
                Value::String(text) => {
                    self.bytes(text.len(), depth)?;
                    continue;
                }
                _ => continue,
            };
            if matches!(mode, Mode::Import)
                && let Some(value) = map.get("$ref")
            {
                let reference = value.as_str().filter(|reference| reference.starts_with("#/"))
                    .ok_or_else(|| reference_rejected("external_reference_unsupported",
                        "Source specification rejected because a reference is not a local JSON pointer"))?;
                // Lookup and active-set comparisons read pointer bytes even
                // when the target is tiny. Source siblings count separately.
                self.bytes(reference.len(), depth)?;
                let target = root.pointer(&reference[1..]).ok_or_else(|| {
                    reference_rejected(
                        "unresolved_reference",
                        "Source specification rejected because a local reference has no target",
                    )
                })?;
                if !active.insert(reference) {
                    return Err(reference_rejected(
                        "cyclic_reference_unsupported",
                        "Source specification rejected because its references contain a cycle",
                    ));
                }
                // Strict import follows the target, leaves its reference scope,
                // then inspects the original object's children and siblings.
                frames.push(Frame::Object(map.iter(), depth, mode));
                frames.push(Frame::LeaveReference(reference));
                frames.push(Frame::Visit(target, depth + 1, mode));
                continue;
            }
            frames.push(Frame::Object(map.iter(), depth, mode));
        }
        Ok(())
    }
}

fn reference_rejected(code: &'static str, message: &'static str) -> anyhow::Error {
    anyhow::anyhow!(message).context(super::diagnostics::Problem(code))
}

/// Dynamic imports retain their stricter reference-admission contract. The
/// traversal and resource accounting are shared with authored catalog handling.
pub(crate) fn import(root: &Value) -> Result<()> {
    Budget::default().inspect(root, root, Mode::Import, 0)
}

/// Raw Value APIs must validate physical depth before any Value::clone, schema
/// projection or metadata copy. This does not follow unused component/response
/// references; actual expansion is charged at the resolver call boundary.
pub(crate) fn shape(root: &Value) -> Result<()> {
    Budget::default().copying(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shared_request_bodies_charge_invalid_and_unsupported_media_before_parsing() {
        for name in [
            format!("invalid media {}", "x".repeat(128)),
            format!("application/xml; p=\"{}\"", "x".repeat(128)),
        ] {
            let source = json!({"content":{(name.clone()):{"schema":true}}});
            shape(&source).unwrap();
            let root = json!({"body":source});
            let reference = json!({"$ref":"#/body"});
            let mut short = Budget {
                nodes: 0,
                bytes: BYTES - "#/body".len() - name.len() + 1,
            };
            assert!(
                super::super::media::body(&root, &reference, &mut short)
                    .unwrap_err()
                    .is::<ComplexityLimit>()
            );
            assert_eq!(short.nodes, 2, "reject before parsing the used media name");

            // Let the first conversion finish, then leave insufficient space
            // for the next expansion of the same small physical body.
            let mut measured = Budget::default();
            let unsupported = super::super::media::body(&root, &reference, &mut measured).unwrap();
            assert_eq!(
                unsupported["issue"],
                json!(super::super::Unsupported::RequestMedia)
            );
            let mut shared = Budget {
                nodes: 0,
                bytes: BYTES - measured.bytes * 2 + 1,
            };
            super::super::media::body(&root, &reference, &mut shared).unwrap();
            assert!(
                super::super::media::body(&root, &reference, &mut shared)
                    .unwrap_err()
                    .is::<ComplexityLimit>()
            );
        }
    }

    #[test]
    fn wildcard_media_candidates_charge_synthesis_and_keep_selected_parameters() {
        let source = json!({"content":{"application/*; profile=kept":{"schema":true}}});
        let mut measured = Budget::default();
        let body = super::super::media::body(&json!({}), &source, &mut measured).unwrap();
        assert_eq!(body["media_type"], "application/json; profile=kept");
        assert_eq!(body["schema"], true);
        let name = "application/*; profile=kept";
        let mut short = Budget {
            nodes: 0,
            bytes: BYTES - name.len() - "application/json".len(),
        };
        assert!(
            super::super::media::body(&json!({}), &source, &mut short)
                .unwrap_err()
                .is::<ComplexityLimit>()
        );
        assert_eq!(
            short.nodes, 3,
            "reject wildcard suffix copy before allocation"
        );
    }

    #[test]
    fn structural_budget_rejections_warn_with_safe_bounds_even_when_errors_are_caught() {
        if crate::test_log_capture::run_in_child(
            "catalog::expansion::tests::structural_budget_rejections_warn_with_safe_bounds_even_when_errors_are_caught",
        ) {
            return;
        }
        let logs = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(logs.reopen().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let private = json!("private-catalog-token");
            let mut bytes = Budget {
                nodes: 0,
                bytes: BYTES,
            };
            assert!(bytes.copying(&private).is_err());
            let mut nodes = Budget {
                nodes: NODES,
                bytes: 0,
            };
            assert!(nodes.copying(&private).is_err());
            assert!(Budget::default().copying_at(&private, DEPTH + 1).is_err());
            let mut overflow = Budget {
                nodes: 0,
                bytes: usize::MAX,
            };
            assert!(overflow.copying(&private).is_err());
            let mut overflow = Budget {
                nodes: usize::MAX,
                bytes: 0,
            };
            assert!(overflow.copying(&private).is_err());
        });
        let text = std::fs::read_to_string(logs.path()).unwrap();
        assert_eq!(text.lines().count(), 5, "{text}");
        for line in text.lines() {
            assert!(
                line.contains("WARN") && line.contains("document_complexity_limit"),
                "{line}"
            );
            assert!(
                line.contains("Catalog rejected")
                    && line.contains("traversal_copy_byte_limit=67108864"),
                "{line}"
            );
        }
        for resource in ["depth", "nodes", "traversal_copy_bytes"] {
            assert!(text.contains(&format!("resource=\"{resource}\"")), "{text}");
        }
        assert!(!text.contains("private-catalog-token"));
    }

    #[test]
    fn openapi_metadata_copies_share_the_conversion_budget_across_methods() {
        let root = json!({});
        let op = json!({"summary":"read", "description":{"opaque":"text"},
            "tags":["tag"], "x-price":{"amount":"1"}});
        let expected = json!({"path":"/shared", "method":"get", "summary":"read",
            "description":{"opaque":"text"}, "tags":["tag"],
            "pricing":{"amount":"1"}, "credit_pricing":{"amount":"1"},
            "params":[], "body":null});
        let mut copied = Budget::default();
        copied.copying(&expected).unwrap();
        let mut exact = Budget {
            nodes: NODES - copied.nodes,
            bytes: BYTES - copied.bytes,
        };
        let convert = |budget: &mut Budget| {
            super::super::openapi_operation(
                &root,
                "/shared",
                "get",
                &op,
                Some("x-price"),
                Some("x-price"),
                budget,
            )
        };
        assert_eq!(convert(&mut exact).unwrap(), expected);
        assert_eq!((exact.nodes, exact.bytes), (NODES, BYTES));
        for (nodes, bytes) in [(0, BYTES - copied.bytes + 1), (NODES - copied.nodes + 1, 0)] {
            assert!(
                convert(&mut Budget { nodes, bytes })
                    .unwrap_err()
                    .is::<ComplexityLimit>()
            );
        }
        // Physical input owns one path; each method emits another owned copy.
        let mut repeated = Budget {
            nodes: 0,
            bytes: BYTES - copied.bytes * 2 + 1,
        };
        convert(&mut repeated).unwrap();
        assert!(
            super::super::openapi_operation(
                &root,
                "/shared",
                "put",
                &op,
                Some("x-price"),
                Some("x-price"),
                &mut repeated,
            )
            .unwrap_err()
            .is::<ComplexityLimit>()
        );
    }

    #[test]
    fn parameter_issues_and_fallback_descriptions_charge_their_actual_copies() {
        let root = json!({});
        for parameter in [
            json!({"name":"x".repeat(128), "in":"cookie"}),
            json!({"name":"x".repeat(128), "in":"query", "$ref":"#/missing"}),
            json!({"name":"x".repeat(128), "in":"query", "schema":null}),
        ] {
            let mut budget = Budget {
                nodes: 0,
                bytes: BYTES - 64,
            };
            assert!(
                super::super::openapi_parameter(&root, &parameter, &mut budget)
                    .unwrap_err()
                    .is::<ComplexityLimit>()
            );
        }
        let parameter = json!({"name":"x".repeat(128), "in":"query", "$ref":"#/missing"});
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - 64,
        };
        assert!(
            super::super::digest_parameter(&root, &parameter, &mut budget)
                .unwrap_err()
                .is::<ComplexityLimit>()
        );

        let schema = json!({"type":"string", "description":"x".repeat(128)});
        let parameter = json!({"name":"q", "in":"query", "schema":schema});
        let expected = json!({"name":"q", "in":"query", "required":false,
            "description":schema["description"], "schema":schema});
        let mut copied = Budget::default();
        copied.copying(&expected).unwrap();
        let mut exact = Budget {
            nodes: NODES - copied.nodes,
            bytes: BYTES - copied.bytes,
        };
        assert_eq!(
            super::super::openapi_parameter(&root, &parameter, &mut exact).unwrap(),
            Some(expected)
        );
        assert_eq!((exact.nodes, exact.bytes), (NODES, BYTES));
        let mut short = Budget {
            nodes: 0,
            bytes: BYTES - copied.bytes + 1,
        };
        assert!(
            super::super::openapi_parameter(&root, &parameter, &mut short)
                .unwrap_err()
                .is::<ComplexityLimit>()
        );
    }

    #[test]
    fn description_reference_output_is_charged_before_copy_and_at_its_real_depth() {
        let root = json!({"openapi":"3.1.0","s":{"type":"string"}});
        let schema = json!({"$ref":"#/s","description":"x".repeat(128)});
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - 64,
        };
        let error = super::super::contracts::schema(&root, &schema, &mut budget).unwrap_err();
        assert!(error.is::<ComplexityLimit>());
        let mut nested = Value::Null;
        for _ in 0..62 {
            nested = json!([nested]);
        }
        let root = json!({"openapi":"3.1.0","s":{"type":"string","default":nested}});
        super::super::contracts::schema(&root, &json!({"$ref":"#/s"}), &mut Budget::default())
            .unwrap();
        let error = super::super::contracts::schema(
            &root,
            &json!({"$ref":"#/s","description":"here"}),
            &mut Budget::default(),
        )
        .unwrap_err();
        assert!(error.is::<ComplexityLimit>());
    }
    #[test]
    fn reference_annotation_payloads_are_charged_before_copy_at_output_depth() {
        let root = json!({"openapi":"3.1.0","s":{"type":"string"}});
        for (key, value) in [
            ("default", json!({"opaque":"x".repeat(128)})),
            ("examples", json!([{"opaque":"x".repeat(128)}])),
        ] {
            let mut schema = json!({"$ref":"#/s"});
            schema[key] = value;
            let mut budget = Budget {
                nodes: 0,
                bytes: BYTES - 64,
            };
            let error = super::super::contracts::schema(&root, &schema, &mut budget).unwrap_err();
            assert!(error.is::<ComplexityLimit>(), "{key}");
        }
        let mut budget = Budget {
            nodes: NODES - 7,
            bytes: 0,
        };
        assert_eq!(
            super::super::contracts::schema(&root, &json!({"$ref":"#/s"}), &mut budget).unwrap(),
            root["s"]
        );
        let mut budget = Budget {
            nodes: NODES - 7,
            bytes: 0,
        };
        let error = super::super::contracts::schema(
            &root,
            &json!({"$ref":"#/s","default":null}),
            &mut budget,
        )
        .unwrap_err();
        assert!(
            error.is::<ComplexityLimit>(),
            "generated wrapper nodes count"
        );
        let mut nested = Value::Null;
        for _ in 0..62 {
            nested = json!([nested]);
        }
        let schema = json!({"$ref":"#/s","default":nested});
        shape(&schema).unwrap();
        let error =
            super::super::contracts::schema(&root, &schema, &mut Budget::default()).unwrap_err();
        assert!(
            error.is::<ComplexityLimit>(),
            "generated annotation bag increases actual copy depth"
        );
    }
    #[test]
    fn unrestricted_proof_charges_inspected_pointer_strings_before_output() {
        let pointer = format!("#/{}", "long".repeat(32));
        let definition = json!({"oneOf":[{"type":"null"},{"type":"boolean"},{"type":"number"},{"type":"string"},
            {"type":"array","items":{"$ref":pointer}},
            {"type":"object","additionalProperties":{"$ref":pointer},"propertyNames":{"type":"string"}}]});
        let mut root = json!({});
        root[pointer.strip_prefix("#/").unwrap()] = definition;
        let input = json!({"$ref":pointer});
        super::super::contracts::schema(&root, &input, &mut Budget::default()).unwrap();
        let mut budget = Budget {
            nodes: 0,
            // The outer pointer and two inspected SELF pointers cannot all fit.
            bytes: BYTES - (2 * pointer.len()),
        };
        let error = super::super::contracts::schema(&root, &input, &mut budget).unwrap_err();
        assert!(
            error.is::<ComplexityLimit>(),
            "each inspected proof pointer must be charged"
        );
    }
    #[test]
    fn schema_resource_context_is_checked_only_after_pointer_work_is_charged() {
        let root = json!({"Container":{"$id":"https://example.com/base","X":{"$ref":"#/Y"}},"Y":{"type":"string"}});
        let schema = json!({"$ref":"#/Container/X"});
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - "$ref".len() - "#/Container/X".len() + 1,
        };
        let error = super::super::contracts::schema(&root, &schema, &mut budget).unwrap_err();
        assert!(error.is::<ComplexityLimit>());
        assert_eq!(
            budget.nodes, 2,
            "pointer work must fail before target traversal/context lookup"
        );
        let error =
            super::super::contracts::schema(&root, &schema, &mut Budget::default()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<super::super::Unsupported>(),
            Some(&super::super::Unsupported::InputReference)
        );
    }
    #[test]
    fn real_schema_projection_charges_repeated_long_pointers_before_lookup() {
        let key = "long-key".repeat(16);
        let pointer = format!("#/{key}");
        let mut fields = serde_json::Map::new();
        fields.insert(key, Value::Bool(true));
        fields.insert("alias".into(), json!({"$ref":pointer.clone()}));
        let root = Value::Object(fields);
        let schema = json!({"allOf":[{"$ref":"#/alias"},{"$ref":"#/alias"}]});
        // Each used pointer is parsed once. One long pointer fits, so the
        // repeated second target cannot be visited.
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - ("allOf".len() + 4 * "$ref".len() + 2 * "#/alias".len() + pointer.len()),
        };
        let error = super::super::contracts::schema(&root, &schema, &mut budget).unwrap_err();
        assert!(error.is::<ComplexityLimit>());
        assert_eq!(budget.nodes, 11);
        let mut exact = Budget {
            nodes: 0,
            bytes: BYTES
                - ("allOf".len() + 4 * "$ref".len() + 2 * "#/alias".len() + 2 * pointer.len()),
        };
        assert_eq!(
            super::super::contracts::schema(&root, &schema, &mut exact).unwrap(),
            json!({"allOf":[true,true]})
        );
        assert_eq!(exact.bytes, BYTES, "charge each actual pointer lookup once");
    }
    #[test]
    fn real_schema_projection_accounts_each_repeated_target() {
        let root = json!({"s":{"type":"string"}});
        let schema = json!({"allOf":[{"$ref":"#/s"},{"$ref":"#/s"}]});
        let projected =
            super::super::contracts::schema(&root, &schema, &mut Budget::default()).unwrap();
        assert_eq!(
            projected,
            json!({"allOf":[{"type":"string"},{"type":"string"}]})
        );
        let mut budget = Budget {
            nodes: NODES - 3,
            bytes: 0,
        };
        assert!(
            super::super::contracts::schema(&root, &schema, &mut budget)
                .unwrap_err()
                .is::<ComplexityLimit>()
        );
    }
    #[test]
    fn exact_existing_boundaries_and_overflow_use_small_inputs() {
        let mut budget = Budget {
            nodes: NODES - 1,
            bytes: BYTES - 1,
        };
        budget.copying(&json!("x")).unwrap();
        assert!(budget.copying(&Value::Null).is_err());
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - 1,
        };
        assert!(budget.copying(&json!("xx")).is_err());
        let mut nested = Value::Null;
        for _ in 0..DEPTH {
            nested = Value::Array(vec![nested]);
        }
        shape(&nested).unwrap();
        assert!(shape(&Value::Array(vec![nested])).is_err());
        let mut overflow = Budget {
            nodes: usize::MAX,
            bytes: 0,
        };
        assert!(overflow.copying(&Value::Null).is_err());
        let mut overflow = Budget {
            nodes: 0,
            bytes: usize::MAX,
        };
        assert!(overflow.copying(&json!("x")).is_err());
        let mut depth = Budget::default();
        assert!(depth.copying_at(&json!([null]), DEPTH).is_err());
    }
}
