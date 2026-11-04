//! Small synthetic regressions for public and cache admission boundaries.
use super::*;

fn chain(length: usize) -> Value {
    let mut definitions = Map::new();
    for index in 0..length {
        definitions.insert(
            index.to_string(),
            json!({"$ref":format!("#/defs/{}", index + 1)}),
        );
    }
    definitions.insert(length.to_string(), json!({"type":"string"}));
    json!({"defs":definitions,"paths":{"/test":{"get":{"parameters":[
        {"name":"value","in":"query","schema":{"$ref":"#/defs/0"}}
    ]}}}})
}

#[test]
fn direct_value_entrypoints_reject_reference_depth_before_conversion() {
    let document = chain(70);
    for result in [
        operations(&document, None).map(|_| ()),
        build_tools(&Config::default(), &document, "test").map(|_| ()),
        tool_names_before_filtering(&Config::default(), &document, "test").map(|_| ()),
        tag_counts(&document).map(|_| ()),
    ] {
        assert!(result.unwrap_err().is::<expansion::ComplexityLimit>());
    }
    let mut unused = chain(70);
    unused["paths"]["/test"]["get"]["parameters"] = json!([]);
    // Unused definitions are not hypothetically expanded for authored sources.
    assert_eq!(operations(&unused, None).unwrap().len(), 1);
}

#[test]
fn authored_reference_fallback_and_import_reference_policy_remain_distinct() {
    for node in [
        json!({"$ref":"#/missing"}),
        json!({"$ref":"https://example.com/schema"}),
        json!({"$ref":"#/cycle"}),
    ] {
        let root = json!({"cycle":{"$ref":"#/cycle"}});
        let mut budget = expansion::Budget::default();
        assert_eq!(resolve(&root, &node, &mut budget).unwrap(), node);
        assert!(expansion::import(&json!({"cycle":node})).is_err());
    }
    let root = json!({"s":{"type":"string"}});
    let node = json!({"$ref":"#/s","allOf":[{"$ref":"#/missing"}]});
    assert_eq!(
        resolve(&root, &node, &mut expansion::Budget::default()).unwrap(),
        root["s"]
    );
}

#[test]
fn composed_repeated_expansion_is_rejected_without_allocating_its_result() {
    let mut definitions = Map::new();
    definitions.insert("0".into(), json!({"type":"string"}));
    for index in 1..20 {
        let reference = format!("#/defs/{}", index - 1);
        definitions.insert(
            index.to_string(),
            json!({"allOf":[{"$ref":reference},{"$ref":reference}]}),
        );
    }
    let root = json!({"defs":definitions});
    assert!(
        resolve(
            &root,
            &json!({"$ref":"#/defs/19"}),
            &mut expansion::Budget::default()
        )
        .unwrap_err()
        .is::<expansion::ComplexityLimit>()
    );
}

#[tokio::test]
async fn local_and_cache_admission_reject_expansion_and_304_cannot_refresh_it() {
    use crate::http_cache::{Entry, Metadata, Slot};
    use reqwest::header::{CACHE_CONTROL, ETAG, HeaderMap};
    let directory = tempfile::tempdir().unwrap();
    let document = serde_json::to_vec(&chain(70)).unwrap();
    let file = directory.path().join("catalog.json");
    tokio::fs::write(&file, &document).await.unwrap();
    assert!(
        load_json(
            file.to_str().unwrap(),
            &crate::test_tls::client("https://api.example.com"),
        )
            .await
            .unwrap_err()
            .is::<expansion::ComplexityLimit>()
    );
    let cfg = Config {
        http_cache_directory: Some(directory.path().join("cache")),
        ..Default::default()
    };
    let slot = Slot::new(
        &cfg,
        "https://example.com/spec",
        "catalog",
        cfg.max_spec_bytes,
    )
    .unwrap();
    let headers = HeaderMap::from_iter([
        (CACHE_CONTROL, "max-age=600".parse().unwrap()),
        (ETAG, "\"original\"".parse().unwrap()),
    ]);
    let entry = Entry {
        metadata: Metadata::from_headers(&headers, std::time::Duration::ZERO, None).unwrap(),
        data: document,
    };
    slot.write(Some(entry.clone())).await;
    assert!(read_cached_catalog(Some(&slot)).await.is_none());
    assert!(slot.read().await.is_none());
    // Revalidation must not replace the last complete entry with rejected bytes.
    let good = Entry {
        data: b"{}".to_vec(),
        ..entry.clone()
    };
    slot.write(Some(good)).await;
    assert!(
        revalidate_cached_catalog(&headers, Some(&entry), Some(&slot))
            .await
            .unwrap_err()
            .is::<expansion::ComplexityLimit>()
    );
    assert_eq!(slot.read().await.unwrap().data, b"{}");
}

#[test]
fn parse_rejection_retains_safe_resource_explanation() {
    let error = parse_document(&serde_json::to_vec(&chain(70)).unwrap()).unwrap_err();
    let failure = diagnostics::Failure::from_error(&error, "fetch_parse");
    assert!(failure.reason.contains("structural"));
    assert!(!failure.reason.contains("#/defs"));
    assert_eq!(parse_document(b"{}").unwrap(), json!({}));
}

#[test]
fn original_idempotency_schema_lookup_shares_depth_and_copy_guard() {
    let mut root = chain(70);
    root["paths"]["/test"]["get"]["parameters"][0]["name"] = json!("Idempotency-Key");
    root["paths"]["/test"]["get"]["parameters"][0]["in"] = json!("header");
    let parameter = &root["paths"]["/test"]["get"]["parameters"][0];
    assert!(
        headers::original_key_schema(&root, parameter, &mut expansion::Budget::default())
            .unwrap_err()
            .is::<expansion::ComplexityLimit>()
    );
    let root = json!({"s":{"type":"string","format":"uuid"}});
    let parameter = json!({"schema":{"$ref":"#/s","maxLength":12}});
    assert_eq!(
        headers::original_key_schema(&root, &parameter, &mut expansion::Budget::default()).unwrap(),
        &parameter["schema"]
    );
}
