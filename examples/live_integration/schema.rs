//! Deliberately strict offline JSON Schema subset: unknown assertions fail qualification.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
pub fn validate(schema: &Value, value: &Value) -> Result<()> {
    preflight(schema, 0, &mut 100_000)?;
    validate_numbers(value, 0, &mut 100_000)?;
    let mut remaining = 100_000;
    let result = check(schema, value, 0, &mut remaining);
    ensure!(
        remaining > 0,
        "argument validation exceeded 100000-operation qualification limit"
    );
    result
}
// Validate every branch before matching data. Unsupported assertions inside an absent
// property, an alternative, or `not` must never be mistaken for a normal mismatch.
fn preflight(schema: &Value, depth: usize, remaining: &mut usize) -> Result<()> {
    ensure!(
        depth < 64 && *remaining > 0,
        "schema exceeds 64-level or 100000-node qualification limit"
    );
    *remaining -= 1;
    if schema.is_boolean() {
        return Ok(());
    }
    let object = schema
        .as_object()
        .context("schema must be object or boolean")?;
    for (key, v) in object {
        match key.as_str() {
            "type" => {
                let types = v.as_array().map_or_else(|| vec![v], |a| a.iter().collect());
                ensure!(!types.is_empty(), "empty schema type list");
                for t in types {
                    ensure!(
                        matches!(
                            t.as_str(),
                            Some(
                                "null"
                                    | "object"
                                    | "array"
                                    | "string"
                                    | "boolean"
                                    | "integer"
                                    | "number"
                            )
                        ),
                        "invalid schema type"
                    );
                }
            }
            "properties" => {
                for s in v.as_object().context("invalid properties")?.values() {
                    preflight(s, depth + 1, remaining)?;
                }
            }
            "required" => {
                for name in v.as_array().context("invalid required")? {
                    ensure!(name.is_string(), "invalid required name");
                }
            }
            "additionalProperties" | "items" | "not" => preflight(v, depth + 1, remaining)?,
            "allOf" | "anyOf" | "oneOf" => {
                let branches = v.as_array().context("invalid schema alternatives")?;
                ensure!(!branches.is_empty(), "empty schema alternatives");
                for s in branches {
                    preflight(s, depth + 1, remaining)?;
                }
            }
            "enum" => ensure!(
                !v.as_array().context("invalid enum")?.is_empty(),
                "empty enum"
            ),
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => {
                exact_number(v)?;
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" => {
                v.as_u64().context("invalid nonnegative schema bound")?;
            }
            "pattern" => {
                regex::Regex::new(v.as_str().context("invalid pattern")?)?;
            }
            "$schema" => ensure!(
                matches!(
                    v.as_str(),
                    Some(
                        "http://json-schema.org/draft-07/schema#"
                            | "https://json-schema.org/draft/2020-12/schema"
                    )
                ),
                "unsupported schema dialect"
            ),
            "const" | "description" | "title" | "default" | "examples" | "deprecated"
            | "readOnly" | "writeOnly" => {}
            _ => anyhow::bail!("unsupported schema keyword {key}; cannot certify arguments"),
        }
    }
    Ok(())
}
fn exact_number(v: &Value) -> Result<f64> {
    let n = v.as_f64().context("invalid numeric schema value")?;
    ensure!(
        n.is_finite() && n.abs() <= 9_007_199_254_740_991.0,
        "number exceeds exact supported qualification range (2^53-1)"
    );
    Ok(n)
}
fn validate_numbers(value: &Value, depth: usize, remaining: &mut usize) -> Result<()> {
    ensure!(
        depth < 64 && *remaining > 0,
        "arguments exceed 64-level or 100000-node qualification limit"
    );
    *remaining -= 1;
    match value {
        Value::Number(_) => {
            exact_number(value)?;
        }
        Value::Array(a) => {
            for v in a {
                validate_numbers(v, depth + 1, remaining)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                validate_numbers(v, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn check(schema: &Value, value: &Value, depth: usize, remaining: &mut usize) -> Result<()> {
    ensure!(
        *remaining > 0,
        "argument validation operation limit exceeded"
    );
    *remaining -= 1;
    ensure!(depth < 64, "schema validation exceeds 64-level limit");
    if let Some(allowed) = schema.as_bool() {
        ensure!(allowed, "schema forbids value");
        return Ok(());
    }
    let object = schema
        .as_object()
        .context("schema must be object or boolean")?;
    for key in object.keys() {
        ensure!(
            [
                "type",
                "properties",
                "required",
                "additionalProperties",
                "enum",
                "const",
                "items",
                "minimum",
                "maximum",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "minLength",
                "maxLength",
                "minItems",
                "maxItems",
                "pattern",
                "allOf",
                "anyOf",
                "oneOf",
                "not",
                "description",
                "title",
                "default",
                "examples",
                "deprecated",
                "readOnly",
                "writeOnly",
                "$schema"
            ]
            .contains(&key.as_str()),
            "unsupported schema keyword {key}; cannot certify arguments"
        );
    }
    if let Some(types) = object.get("type") {
        let matches = |t: &Value| match t.as_str() {
            Some("null") => value.is_null(),
            Some("object") => value.is_object(),
            Some("array") => value.is_array(),
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            _ => false,
        };
        ensure!(
            types
                .as_array()
                .map_or_else(|| matches(types), |ts| ts.iter().any(matches)),
            "argument type mismatch"
        );
    }
    if let Some(v) = object.get("enum") {
        ensure!(
            v.as_array().context("invalid enum")?.contains(value),
            "argument not in enum"
        );
    }
    if let Some(v) = object.get("const") {
        ensure!(v == value, "argument differs from const");
    }
    if let Some(map) = value.as_object() {
        for r in object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            ensure!(
                map.contains_key(r.as_str().context("invalid required name")?),
                "missing required argument"
            );
        }
        for (name, v) in map {
            if let Some(s) = schema.get("properties").and_then(|p| p.get(name)) {
                check(s, v, depth + 1, remaining)?;
            } else if let Some(s) = schema.get("additionalProperties") {
                check(s, v, depth + 1, remaining)?;
            }
        }
    }
    if let Some(items) = value.as_array() {
        bounds(items.len(), schema, "minItems", "maxItems")?;
        if let Some(s) = schema.get("items") {
            for v in items {
                check(s, v, depth + 1, remaining)?;
            }
        }
    }
    if let Some(text) = value.as_str() {
        bounds(text.chars().count(), schema, "minLength", "maxLength")?;
        if let Some(p) = schema.get("pattern") {
            ensure!(
                regex::Regex::new(p.as_str().context("invalid pattern")?)?.is_match(text),
                "argument pattern mismatch"
            );
        }
    }
    if value.is_number() {
        let n = exact_number(value)?;
        for (key, exclusive, lower) in [
            ("minimum", false, true),
            ("maximum", false, false),
            ("exclusiveMinimum", true, true),
            ("exclusiveMaximum", true, false),
        ] {
            if let Some(limit) = schema.get(key) {
                let limit = exact_number(limit)?;
                ensure!(
                    if lower {
                        if exclusive { n > limit } else { n >= limit }
                    } else if exclusive {
                        n < limit
                    } else {
                        n <= limit
                    },
                    "numeric bound violated"
                );
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = schema.get(key) {
            let branches = branches.as_array().context("invalid schema alternatives")?;
            let count = branches
                .iter()
                .filter(|b| check(b, value, depth + 1, remaining).is_ok())
                .count();
            ensure!(
                match key {
                    "allOf" => count == branches.len(),
                    "anyOf" => count > 0,
                    _ => count == 1,
                },
                "schema alternatives did not match"
            );
        }
    }
    if let Some(s) = schema.get("not") {
        ensure!(
            check(s, value, depth + 1, remaining).is_err(),
            "schema not constraint violated"
        );
    }
    Ok(())
}
fn bounds(n: usize, s: &Value, min: &str, max: &str) -> Result<()> {
    if let Some(v) = s.get(min) {
        ensure!(
            n as u64 >= v.as_u64().context("invalid minimum length")?,
            "argument below minimum length"
        );
    }
    if let Some(v) = s.get(max) {
        ensure!(
            n as u64 <= v.as_u64().context("invalid maximum length")?,
            "argument exceeds maximum length"
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nested_arguments_and_unknown_assertions_fail_closed() {
        let s = json!({"type":"object","required":["n"],"properties":{"n":{"type":"integer","minimum":2}},"additionalProperties":false});
        assert!(validate(&s, &json!({"n":2})).is_ok());
        for v in [
            json!({}),
            json!({"n":1}),
            json!({"n":"2"}),
            json!({"n":2,"extra":1}),
        ] {
            assert!(validate(&s, &v).is_err());
        }
        assert!(validate(&json!({"format":"uri"}), &json!("x")).is_err());
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn hidden_unsupported_or_malformed_branches_never_certify() {
        for s in [
            json!({"anyOf":[true,{"format":"uri"}]}),
            json!({"oneOf":[true,{"format":"uri"}]}),
            json!({"not":{"format":"uri"}}),
            json!({"properties":{"absent":{"format":"uri"}}}),
            json!({"required":"bad"}),
            json!({"properties":[]}),
            json!({"minimum":9007199254740993u64}),
            json!({"anyOf":[]}),
            json!({"type":"typo"}),
        ] {
            assert!(validate(&s, &json!({})).is_err(), "{s}");
        }
        assert!(validate(&json!({"type":"integer"}), &json!(9007199254740993u64)).is_err());
    }
}
