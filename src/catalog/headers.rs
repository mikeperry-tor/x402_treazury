//! Declared business headers and per-invocation idempotency, without replay policy.
use super::diagnostics::Problem;
use super::{HeaderParameter, arg_text};
use anyhow::{Context, Result, ensure};
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn capacity_error(_: reqwest::header::MaxSizeReached) -> anyhow::Error {
    tracing::warn!("HTTP request headers exceed the HTTP library's representation capacity");
    anyhow::Error::new(Problem(
        "HTTP request headers exceed HTTP representation capacity; reduce request headers",
    ))
}

pub(super) fn ignored(name: &str) -> bool {
    matches!(name, "accept" | "content-type" | "authorization")
}

pub(super) fn payment(name: &str) -> bool {
    matches!(
        name,
        "payment-signature"
            | "x-payment"
            | "payment-required"
            | "payment-response"
            | "x-payment-response"
    )
}

/// These fields belong to HTTP framing, credentials or the payment implementation.
/// Apply the same boundary when a Rust caller constructs a routed request directly.
pub(crate) fn business_name(name: &str) -> Result<HeaderName> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .context(Problem("An API header name is invalid"))?;
    ensure!(
        !ignored(name.as_str())
            && !payment(name.as_str())
            && !name.as_str().starts_with("proxy-")
            && !matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "connection"
                    | "accept-encoding"
                    | "keep-alive"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
                    | "cookie"
                    | "cookie2"
            ),
        Problem("API header is reserved for HTTP transport, authentication or payment")
    );
    Ok(name)
}

fn automatic_key_schema(schema: &Value) -> Result<()> {
    // No general schema validator or vendor-specific key generator is needed.
    // UUID strings cover the common declaration; fail visibly for other contracts.
    ensure!(
        schema.is_null() || schema.is_object(),
        "unsupported Idempotency-Key schema"
    );
    for (key, value) in schema.as_object().into_iter().flatten() {
        let compatible = match key.as_str() {
            "type" => value == "string",
            "format" => value == "uuid",
            "minLength" => value.as_u64().is_some_and(|n| n <= 36),
            "maxLength" => value.as_u64().is_some_and(|n| n >= 36),
            "title" | "description" | "example" | "examples" | "default" | "deprecated"
            | "readOnly" | "writeOnly" | "nullable" => true,
            _ => key.starts_with("x-"),
        };
        ensure!(
            compatible,
            "Idempotency-Key schema is incompatible with automatic UUID keys"
        );
    }
    Ok(())
}

pub(super) fn original_key_schema<'a>(
    root: &'a Value,
    parameter: &'a Value,
    budget: &mut super::expansion::Budget,
) -> Result<&'a Value> {
    // Preserve authored Reference Object behavior, including schema siblings.
    let parameter = follow_reference(root, parameter, false, budget)?;
    follow_reference(root, &parameter["schema"], true, budget)
}

fn follow_reference<'a>(
    root: &'a Value,
    mut node: &'a Value,
    preserve_siblings: bool,
    budget: &mut super::expansion::Budget,
) -> Result<&'a Value> {
    let mut seen = BTreeSet::new();
    let mut depth = 0;
    while let Some(reference) = node["$ref"].as_str() {
        if preserve_siblings && node.as_object().is_some_and(|object| object.len() != 1) {
            break;
        }
        if !reference.starts_with("#/") {
            break;
        }
        depth += 1;
        budget.reference(reference, depth)?;
        if !seen.insert(reference) {
            break;
        }
        let Some(target) = root.pointer(&reference[1..]) else {
            break;
        };
        node = target;
    }
    Ok(node)
}

pub(super) fn collect(
    params: &Value,
    props: &mut Map<String, Value>,
    required: &mut BTreeSet<String>,
    names: &mut super::input::Names,
) -> Result<BTreeMap<String, HeaderParameter>> {
    let mut headers = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for parameter in params
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["in"] == "header")
    {
        let name = parameter["name"].as_str().context(Problem(
            "The catalog contains a header parameter without a name",
        ))?;
        let lower = name.to_ascii_lowercase();
        // These declarations document the protocol, not caller-supplied input.
        // Routing still rejects them through business_name, including forged metadata.
        if ignored(&lower) || payment(&lower) {
            continue;
        }
        let wire = business_name(name)?;
        ensure!(
            seen.insert(wire.to_string()),
            Problem("The catalog declares the same HTTP header more than once")
        );
        ensure!(
            parameter.get("content").is_none()
                && parameter.get("style").is_none_or(|style| style == "simple")
                && parameter.get("explode").is_none_or(Value::is_boolean),
            Problem(
                "The catalog uses unsupported HTTP header serialization; expected OpenAPI simple style"
            )
        );
        let managed = wire == "idempotency-key";
        if managed {
            automatic_key_schema(&parameter["schema"]).context(Problem(
                "The catalog's Idempotency-Key schema does not allow an automatically generated UUID",
            ))?;
        }
        let argument = names.argument(name, "header");
        ensure!(
            !props.contains_key(&argument) && !headers.contains_key(&argument),
            Problem("The catalog's HTTP header argument conflicts with another input argument")
        );
        let is_required = parameter["required"] == true;
        if !managed {
            let schema =
                super::input::parameter_schema(parameter, "Use a scalar or a flat array/object.");
            props.insert(argument.clone(), schema);
            if is_required {
                required.insert(argument.clone());
            }
        }
        headers.insert(
            argument,
            HeaderParameter {
                name: name.into(),
                required: is_required,
                explode: parameter["explode"] == true,
            },
        );
    }
    Ok(headers)
}

fn scalar(value: &Value) -> Result<String> {
    ensure!(
        value.is_string() || value.is_number() || value.is_boolean(),
        "API header values must be scalars or flat arrays/objects"
    );
    Ok(arg_text(value))
}

fn simple(value: &Value, explode: bool) -> Result<String> {
    match value {
        Value::Array(values) => values
            .iter()
            .map(scalar)
            .collect::<Result<Vec<_>>>()
            .map(|v| v.join(",")),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                Ok(format!(
                    "{key}{}{value}",
                    if explode { "=" } else { "," },
                    value = scalar(value)?
                ))
            })
            .collect::<Result<Vec<_>>>()
            .map(|v| v.join(",")),
        _ => scalar(value),
    }
}

pub(super) fn route(
    parameters: &BTreeMap<String, HeaderParameter>,
    args: &Map<String, Value>,
) -> Result<BTreeMap<String, String>> {
    let mut headers = BTreeMap::new();
    for (argument, parameter) in parameters {
        let name = business_name(&parameter.name)?;
        let managed = name == "idempotency-key";
        let value = if managed {
            ensure!(
                !args.contains_key(argument),
                "Idempotency-Key is generated by Treazury for each invocation"
            );
            uuid::Uuid::new_v4().to_string()
        } else {
            let Some(value) = args.get(argument).filter(|v| !v.is_null()) else {
                ensure!(
                    !parameter.required,
                    "missing required API header argument: {argument}"
                );
                continue;
            };
            simple(value, parameter.explode)?
        };
        HeaderValue::from_str(&value).context("invalid API header value")?;
        ensure!(
            headers.insert(name.to_string(), value).is_none(),
            "duplicate API header parameter"
        );
    }
    Ok(headers)
}
