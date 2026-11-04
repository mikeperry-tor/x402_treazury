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
    Expand,
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

    /// Mirror resolve exactly: a successfully followed reference replaces the
    /// whole object; unresolved, external or active-cycle references clone the
    /// original object without resolving its children. Invalid non-string $ref
    /// fields remain ordinary object members. Siblings are not extra expansions.
    pub(crate) fn expansion(&mut self, root: &Value, node: &Value) -> Result<()> {
        self.inspect(root, node, Mode::Expand)
    }

    pub(crate) fn copying(&mut self, node: &Value) -> Result<()> {
        self.inspect(node, node, Mode::Clone)
    }

    /// Charge an iterative reference-only consumer before it compares or
    /// parses a local JSON pointer (for example original Idempotency-Key schema
    /// lookup). It shares the conversion budget rather than a second counter.
    pub(crate) fn reference(&mut self, reference: &str, depth: usize) -> Result<()> {
        self.node(depth)?;
        self.bytes(reference.len(), depth)
    }

    fn inspect<'a>(&mut self, root: &'a Value, node: &'a Value, mode: Mode) -> Result<()> {
        let mut frames = vec![Frame::Visit(node, 0, mode)];
        let mut active = BTreeSet::new();
        while let Some(frame) = frames.pop() {
            // Cursor advancement schedules one child at a time. Actual value
            // work is outside this match, so all policies use one flat path.
            let (node, depth, mut mode) = match frame {
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
            if matches!(mode, Mode::Expand)
                && let Some(reference) = map.get("$ref").and_then(Value::as_str)
            {
                if reference.starts_with("#/") {
                    self.bytes(reference.len(), depth)?;
                }
                if reference.starts_with("#/") && active.insert(reference) {
                    if let Some(target) = root.pointer(&reference[1..]) {
                        frames.push(Frame::LeaveReference(reference));
                        frames.push(Frame::Visit(target, depth + 1, Mode::Expand));
                        continue;
                    }
                    active.remove(reference);
                }
                mode = Mode::Clone;
            }
            frames.push(Frame::Object(map.iter(), depth, mode));
        }
        Ok(())
    }
}

fn reference_rejected(code: &'static str, message: &'static str) -> anyhow::Error {
    anyhow::anyhow!(code).context(super::diagnostics::Problem(message))
}

/// Dynamic imports retain their stricter reference-admission contract. The
/// traversal and resource accounting are shared with authored catalog handling.
pub(crate) fn import(root: &Value) -> Result<()> {
    Budget::default().inspect(root, root, Mode::Import)
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
    fn reference_fallback_does_not_expand_ignored_siblings_or_cycle_leaves() {
        let root = json!({
            "a": {"type":"string"},
            "loop": {"$ref":"#/loop", "unused":{"$ref":"#/missing"}}
        });
        let mut direct = Budget::default();
        direct
            .expansion(&root, &json!({"$ref":"#/a", "ignored": {"type":"number"}}))
            .unwrap();
        assert_eq!(direct.nodes, 3); // wrapper, target object, target string
        assert_eq!(direct.bytes, "#/a".len() + "type".len() + "string".len());
        let mut cycle = Budget::default();
        cycle.expansion(&root, &root["loop"]).unwrap();
        // The repeated reference clones its remaining subtree without following
        // the nested missing ref; preserving the authored resolver contract.
        assert!(cycle.nodes > 1);
    }

    #[test]
    fn repeated_long_pointer_traversal_is_charged_before_target_lookup() {
        let key = "long-key".repeat(16);
        let pointer = format!("#/{key}");
        let mut fields = serde_json::Map::new();
        fields.insert(key, Value::Null);
        fields.insert("alias".into(), json!({"$ref":pointer.clone()}));
        let root = Value::Object(fields);
        let node = json!([{"$ref":"#/alias"},{"$ref":"#/alias"}]);
        // Simulate earlier work at the real byte boundary without allocating a
        // giant fixture. One long target lookup fits; the second must fail
        // before visiting its tiny null target, not only when copying strings.
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - (2 * "#/alias".len() + pointer.len()),
        };
        assert!(budget.expansion(&root, &node).is_err());
        assert_eq!(budget.nodes, 6);
    }

    #[test]
    fn repeated_targets_are_charged_each_time_and_arithmetic_fails_visibly() {
        let root = json!({"s":{"type":"string"}});
        let node = json!([{"$ref":"#/s"},{"$ref":"#/s"}]);
        let mut budget = Budget::default();
        budget.expansion(&root, &node).unwrap();
        assert_eq!(budget.nodes, 7);
        assert_eq!(
            budget.bytes,
            2 * ("#/s".len() + "type".len() + "string".len())
        );
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
    }
    #[test]
    fn exact_existing_boundaries_use_small_inputs() {
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
        let mut budget = Budget {
            nodes: 0,
            bytes: BYTES - 2,
        };
        budget.reference("#/", DEPTH).unwrap();
        assert!(budget.reference("#/", DEPTH + 1).is_err());
    }
}
