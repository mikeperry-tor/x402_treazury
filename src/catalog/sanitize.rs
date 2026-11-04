//! Preserve the public infallible projection without recursive traversal.
use serde_json::{Map, Value};

pub(super) fn sanitize(root: &Value) -> Value {
    let mut output = Value::Null;
    let mut pending = vec![(root, &mut output)];
    while let Some((source, destination)) = pending.pop() {
        match source {
            Value::Object(fields) => {
                let selected: Vec<_> = fields
                    .iter()
                    .filter_map(|(key, child)| {
                        if matches!(key.as_str(), "exclusiveMinimum" | "exclusiveMaximum") {
                            let bound = if child == &Value::Bool(true) {
                                fields.get(if key == "exclusiveMinimum" {
                                    "minimum"
                                } else {
                                    "maximum"
                                })
                            } else {
                                Some(child)
                            };
                            return bound
                                .filter(|value| value.is_number())
                                .map(|value| (key, value));
                        }
                        Some((key, child))
                    })
                    .collect();
                *destination = Value::Object(
                    selected
                        .iter()
                        .map(|(key, _)| ((*key).clone(), Value::Null))
                        .collect::<Map<_, _>>(),
                );
                // Both iterators have the source map's order. Allocate each
                // container once, then fill disjoint borrowed child slots.
                if let Value::Object(slots) = destination {
                    pending.extend(
                        selected
                            .into_iter()
                            .zip(slots.values_mut())
                            .map(|((_, child), slot)| (child, slot)),
                    );
                }
            }
            Value::Array(children) => {
                *destination = Value::Array(vec![Value::Null; children.len()]);
                if let Value::Array(slots) = destination {
                    pending.extend(children.iter().zip(slots.iter_mut()));
                }
            }
            _ => *destination = source.clone(),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_deep_sanitization_uses_no_recursive_traversal() {
        let mut input =
            serde_json::json!({"minimum":2,"exclusiveMinimum":true,"exclusiveMaximum":false});
        for _ in 0..2048 {
            input = Value::Array(vec![input]);
        }
        let output = sanitize(&input);
        let mut leaf = &output;
        for _ in 0..2048 {
            leaf = &leaf[0];
        }
        assert_eq!(*leaf, serde_json::json!({"minimum":2,"exclusiveMinimum":2}));
        // Artificial direct Value depth is also the caller's destruction concern.
        for mut value in [input, output] {
            while let Value::Array(mut children) = value {
                value = children.pop().unwrap();
            }
        }
    }
}
