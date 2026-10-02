//! Development-only OpenAPI request fixture writer; no wallet or paid transport.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    about = "Create a deterministic OpenAPI request fixture, omitting response documentation"
)]
struct Args {
    #[arg(long)]
    network_config: Option<std::path::PathBuf>,
    /// Local JSON path or HTTP(S) URL.
    source: String,
    /// Destination JSON file. Replaces an existing fixture after successful parsing.
    output: PathBuf,
}

fn snapshot(mut spec: Value) -> Result<Vec<u8>> {
    let paths = spec
        .get_mut("paths")
        .and_then(Value::as_object_mut)
        .context("expected an OpenAPI document with a paths object")?;
    for item in paths.values_mut() {
        let item = item
            .as_object_mut()
            .context("OpenAPI path item must be an object")?;
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ] {
            if let Some(operation) = item.get_mut(method) {
                operation
                    .as_object_mut()
                    .context("OpenAPI operation must be an object")?
                    .remove("responses");
            }
        }
    }
    if let Some(components) = spec.get_mut("components").and_then(Value::as_object_mut) {
        components.remove("responses");
    }
    spec.sort_all_objects();
    let json = serde_json::to_string(&spec)?;
    // ASCII escaping keeps fixtures compatible with the existing Python-generated
    // snapshots, including UTF-16 surrogate pairs for non-BMP characters.
    let mut output = String::with_capacity(json.len() + 1);
    for c in json.chars() {
        if c.is_ascii() {
            output.push(c);
        } else {
            use std::fmt::Write;
            for unit in c.encode_utf16(&mut [0; 2]) {
                write!(output, "\\u{unit:04x}").expect("writing to String");
            }
        }
    }
    output.push('\n');
    Ok(output.into_bytes())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(path) = &args.network_config {
        x402_treazure::network::install(x402_treazure::network::NetworkPolicy::load(path)?)?;
    }
    let raw = if args.source.starts_with("https://") || args.source.starts_with("http://") {
        let http =
            x402_treazure::network::discovery(&args.source, std::time::Duration::from_secs(120))?;
        http.get(&args.source)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec()
    } else {
        ensure!(
            !args.source.contains("://"),
            "source must be a local path or HTTP(S) URL"
        );
        tokio::fs::read(&args.source)
            .await
            .context("reading source spec")?
    };
    let bytes = snapshot(serde_json::from_slice(&raw).context("parsing source JSON")?)?;
    // Complete all parsing/serialization before opening the destination.
    tokio::fs::write(&args.output, bytes)
        .await
        .context("writing fixture")?;
    eprintln!("Wrote {}", args.output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removes_only_response_documentation_and_preserves_request_metadata() {
        let input = json!({"openapi":"3.0.3", "paths":{"/items":{
            "x-guide":{"responses":"keep extension content"},
            "post":{"responses":{"200":{"description":"large"}},
                "tags":["Items"],"description":"Café 😀",
                "requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Item"}}}},
                "x-price":{"amount":"0.01"}}}},
            "components":{"responses":{"Large":{}},"schemas":{"Item":{"type":"object","properties":{"responses":{"type":"string"}}}}}});
        let mut expected = input.clone();
        expected["paths"]["/items"]["post"]
            .as_object_mut()
            .unwrap()
            .remove("responses");
        expected["components"]
            .as_object_mut()
            .unwrap()
            .remove("responses");
        let bytes = snapshot(input).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("Caf\\u00e9 \\ud83d\\ude00"));
        assert!(text.ends_with('\n'));
        assert_eq!(snapshot(expected).unwrap(), bytes);
        assert!(snapshot(json!({"operations":[]})).is_err());
    }

    #[test]
    fn committed_socialfetch_fixture_is_byte_stable() {
        let bytes = include_bytes!("../tests/fixtures/socialfetch_openapi.json");
        assert_eq!(
            snapshot(serde_json::from_slice(bytes).unwrap()).unwrap(),
            bytes
        );
    }
}
