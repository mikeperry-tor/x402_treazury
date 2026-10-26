//! Direct, documented API calls under endpoint-local source-management authority.
use super::{Manager, import};
use crate::{catalog::RoutedRequest, output::ToolOutput};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub const NAME: &str = "x402_treazury_raw_call";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    method: String,
    url: String,
    #[serde(default)]
    query: BTreeMap<String, Value>,
    #[serde(default)]
    body: Value,
    #[serde(default)]
    headers: Value,
}

pub fn schema() -> Value {
    let scalar = json!({"type":["string","number","boolean"]});
    json!({"type":"object","properties":{
        "method":{"type":"string","description":"Documented HTTP method, such as GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS. Use the provider's exact method."},
        "url":{"type":"string","format":"uri","description":"Any documented public HTTPS x402 API URL. No URL credentials or fragments. Existing query parameters are preserved."},
        "query":{"type":"object","description":"Optional query parameters, appended to the URL and URL-encoded. Scalars become strings; arrays repeat the parameter name. Do not repeat parameters already in the URL unless the API requires it.","additionalProperties":{"oneOf":[scalar.clone(),{"type":"array","items":scalar}]}},
        "headers":{"type":"object","additionalProperties":{"type":"string"},"description":"Optional documented business headers, Authorization or Idempotency-Key. Values are private: no paid relay or wallet handoff. Idempotency-Key is caller-owned and grants no replay guarantee. Secrets can remain in client history and may be echoed in responses."},
        "body":{"description":"Optional JSON request body, sent as application/json exactly as supplied. Omit when the documented request has no body; explicit null sends JSON null."}
    },"required":["method","url"],"additionalProperties":false})
}

fn request(args: &Map<String, Value>) -> Result<(RoutedRequest, ManualRawHeaders)> {
    let input: Arguments = serde_json::from_value(Value::Object(args.clone()))?;
    let method =
        reqwest::Method::from_bytes(input.method.as_bytes()).context("invalid HTTP method")?;
    let url = import::endpoint(&input.url)?;
    let scalar = |v: &Value| v.is_string() || v.is_number() || v.is_boolean();
    for value in input.query.values() {
        ensure!(
            scalar(value) || value.as_array().is_some_and(|vs| vs.iter().all(scalar)),
            "query values must be strings, numbers, booleans or arrays of these scalars"
        );
    }
    let headers = ManualRawHeaders::parse(args.contains_key("headers").then_some(&input.headers))?;
    Ok((
        RoutedRequest {
            headers: Default::default(),
            method: method.to_string(),
            url: url.into(),
            query: input.query,
            body: args.contains_key("body").then_some(input.body),
        },
        headers,
    ))
}

/// Validated invocation-local headers. Never serialize through RoutedRequest.
#[derive(Default, Debug)]
pub(crate) struct ManualRawHeaders(reqwest::header::HeaderMap);

impl ManualRawHeaders {
    pub(crate) fn parse(input: Option<&Value>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(input) = input {
            let values = input
                .as_object()
                .context("headers must be an object of strings")?;
            for (name, value) in values {
                let name = if name.eq_ignore_ascii_case("authorization") {
                    reqwest::header::AUTHORIZATION
                } else {
                    crate::catalog::headers::business_name(name)?
                };
                ensure!(!headers.contains_key(&name), "duplicate manual HTTP header");
                let value = value
                    .as_str()
                    .context("manual HTTP header values must be strings")?;
                let mut value = reqwest::header::HeaderValue::from_str(value)
                    .context("invalid manual HTTP header value")?;
                value.set_sensitive(true);
                headers
                    .try_insert(name, value)
                    .map_err(crate::catalog::headers::capacity_error)?;
            }
        }
        Ok(Self(headers))
    }

    pub(crate) fn is_private(&self) -> bool {
        !self.0.is_empty()
    }

    pub(crate) fn apply(self, request: &mut reqwest::Request) -> Result<()> {
        for (name, value) in &self.0 {
            request
                .headers_mut()
                .try_insert(name.clone(), value.clone())
                .map_err(crate::catalog::headers::capacity_error)?;
        }
        Ok(())
    }
}

impl Manager {
    pub async fn invoke_raw(&self, owner: &str, args: &Map<String, Value>) -> Result<ToolOutput> {
        ensure!(self.policy(owner).raw, "raw_url_calls_disabled");
        let (route, headers) = request(args)?;
        self.source_client(owner)?
            .with_source_identity("builtin:raw")
            .public_destinations()
            .execute_manual_raw(route, headers)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_headers_validate_without_value_disclosure() {
        for input in [
            json!(null),
            json!("secret"),
            json!({"x-label":17}),
            json!({"x-label":["secret"]}),
            json!({"x-label":{"secret":true}}),
            json!({"x-label":"secret\r\ninjected: yes"}),
            json!({"bad name":"secret"}),
            json!({"X-Label":"secret","x-label":"other"}),
            json!({"Authorization":"secret", "AUTHORIZATION":"other"}),
            json!({"Authorization":"secret\r\nextra: yes"}),
            json!({"Authorization": ["secret"]}),
        ] {
            let error = ManualRawHeaders::parse(Some(&input)).unwrap_err();
            assert!(!format!("{error:#?}").contains("secret"));
        }
        for name in [
            "Accept",
            "Content-Type",
            "Host",
            "Proxy-Authorization",
            "Connection",
            "Content-Length",
            "Accept-Encoding",
            "Keep-Alive",
            "TE",
            "Trailer",
            "Transfer-Encoding",
            "Upgrade",
            "Cookie",
            "Cookie2",
            "Payment-Signature",
            "X-Payment",
            "Payment-Required",
            "Payment-Response",
            "X-Payment-Response",
        ] {
            for name in [name.to_string(), name.to_ascii_uppercase()] {
                assert!(ManualRawHeaders::parse(Some(&json!({name:"secret"}))).is_err());
            }
        }
    }

    #[test]
    fn manual_headers_stay_sensitive_outside_serializable_routes() {
        let input = json!({"method":"POST","url":"https://api.example.com/action","headers":{"X-Label":"private-label", "X-Api-Key":"private-token", "Idempotency-Key":"private-key", "Authorization":"private-auth"}});
        let (route, headers) = request(input.as_object().unwrap()).unwrap();
        assert!(headers.is_private());
        let debug = format!("{route:?} {headers:?}");
        let serialized = serde_json::to_string(&route).unwrap();
        let mut request = reqwest::Request::new(reqwest::Method::POST, route.url.parse().unwrap());
        headers.apply(&mut request).unwrap();
        for value in [
            "private-label",
            "private-token",
            "private-key",
            "private-auth",
        ] {
            assert!(!debug.contains(value));
            assert!(!serialized.contains(value));
            assert!(!format!("{request:?}").contains(value));
        }
        assert!(request.headers().values().all(|v| v.is_sensitive()));
        assert_eq!(request.headers()["idempotency-key"], "private-key");
    }

    #[test]
    fn manual_header_parsing_rejects_unrepresentable_sets() {
        let mut represented = reqwest::header::HeaderMap::new();
        let mut values = Map::new();
        let mut index = 0;
        loop {
            let name = format!("x-field-{index}");
            let parsed = reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap();
            if represented
                .try_insert(parsed, reqwest::header::HeaderValue::from_static("secret"))
                .is_err()
            {
                break;
            }
            values.insert(name, json!("secret"));
            index += 1;
        }
        ManualRawHeaders::parse(Some(&Value::Object(values.clone()))).unwrap();
        values.insert(format!("x-field-{index}"), json!("secret"));
        let error = ManualRawHeaders::parse(Some(&Value::Object(values))).unwrap_err();
        assert!(!format!("{error:#?}").contains("secret"));
    }

    #[test]
    fn omitted_and_empty_headers_preserve_body_presence() {
        for headers in [None, Some(json!({}))] {
            for body in [None, Some(Value::Null)] {
                let mut args = json!({"method":"POST","url":"https://api.example.com/action"});
                if let Some(headers) = &headers {
                    args["headers"] = headers.clone();
                }
                if let Some(body) = &body {
                    args["body"] = body.clone();
                }
                let (route, headers) = request(args.as_object().unwrap()).unwrap();
                assert!(!headers.is_private());
                assert!(route.headers.is_empty());
                assert_eq!(route.body, body);
            }
        }
        // MCP arguments already are serde_json::Map: exact duplicate keys have
        // collapsed before raw validation, while case variants remain detectable.
        let args: Value = serde_json::from_str(r#"{"method":"GET","url":"https://api.example.com","headers":{"x-label":"first","x-label":"last"}}"#).unwrap();
        let (_, headers) = request(args.as_object().unwrap()).unwrap();
        assert_eq!(headers.0["x-label"], "last");
    }
}
