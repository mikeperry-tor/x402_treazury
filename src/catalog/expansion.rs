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

    /// Charge an iterative reference-only consumer before it compares or
    /// parses a local JSON pointer (for example original Idempotency-Key schema
    /// lookup). It shares the conversion budget rather than a second counter.
    pub(crate) fn reference(&mut self, reference: &str, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.bytes(reference.len(), depth)
    }

    /// Charge the actual schema-provenance walk without expanding its target
    /// before a known unsupported sibling can reject that operation.
    pub(crate) fn schema_node(&mut self, node: &Value, depth: usize) -> Result<()> {
        self.node(depth)?;
        if let Value::Object(fields) = node {
            for key in fields.keys() {
                self.bytes(key.len(), depth)?;
            }
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
        // Covers both preflight/projection pointer parses. Only three long
        // pointer traversals fit, so the second target cannot be visited.
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES
                - ("allOf".len() + 4 * "$ref".len() + 4 * "#/alias".len() + 3 * pointer.len()),
        };
        let error = super::super::contracts::schema(&root, &schema, &mut budget).unwrap_err();
        assert!(error.is::<ComplexityLimit>());
        assert_eq!(budget.nodes, 15);
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
