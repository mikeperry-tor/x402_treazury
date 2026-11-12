//! Rejected source content must not become an indefinitely reusable snapshot.
use super::*;
use crate::http_cache::{Entry, Metadata, Slot};
use axum::{Json, Router, response::IntoResponse, routing::get};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn ordinary_rejected_catalogs_recover_on_fresh_startup_and_preserve_previous_bytes() {
    let document = Arc::new(Mutex::new(json!({"error":"maintenance"})));
    let calls = Arc::new(AtomicUsize::new(0));
    let (address, server) = crate::test_tls::serve(Router::new().route(
        "/spec",
        get({
            let document = document.clone();
            let calls = calls.clone();
            move || {
                let document = document.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(document.lock().unwrap().clone())
                }
            }
        }),
    ))
    .await;
    let config: SourceConfig = toml::from_str(&format!(
        "spec='https://{address}/spec'\nbase_url='https://{address}'\n"
    ))
    .unwrap();
    // Fresh Downloads owners model restart: their per-startup acquisition cells
    // cannot conceal persistence of the rejected response.
    for (purpose, defer) in [
        (CatalogPurpose::Inspection, false),
        (CatalogPurpose::Serving, false),
        (CatalogPurpose::Serving, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment.toml");
        *document.lock().unwrap() = json!({"error":"maintenance"});
        let before = calls.load(Ordering::SeqCst);
        let downloads = || Downloads {
            purpose,
            defer,
            cache_directory: Some(directory.path().join("http-cache")),
            ..Default::default()
        };
        let load = || async {
            let downloads = downloads();
            let cfg = downloads.configure("api", &config, &path, false).await;
            match load_one("api", &config, cfg, &downloads).await? {
                SourceLoad::Ready(source) => Ok(source),
                SourceLoad::Pending(pending) => {
                    pending
                        .resume(
                            "api",
                            Arc::new(crate::network::DiscoveryCapacity::new(2)),
                            Default::default(),
                        )
                        .await
                }
            }
        };
        let cfg = downloads()
            .configure("api", &config, &path, false)
            .await
            .unwrap();
        assert!(!cfg.catalog_cache_force_refresh);
        let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
        assert!(load().await.is_err());
        assert!(
            slot.read().await.is_none(),
            "semantic rejection must not be cached"
        );

        *document.lock().unwrap() = json!({"paths":{"/read":{"get":{}}}});
        let source = load().await.unwrap();
        assert_eq!(source.tools.len(), 1);
        assert_eq!(source.tools[0].path, "/read");
        assert!(source.tools[0].catalog_hints.is_some());
        let previous = slot.read().await.unwrap();
        assert_eq!(
            previous.metadata.expires,
            crate::http_cache::INDEFINITE_EXPIRY
        );
        assert_eq!(calls.load(Ordering::SeqCst), before + 2);
        // Another restart reuses the accepted snapshot.
        assert_eq!(load().await.unwrap().tools[0].path, "/read");
        assert_eq!(calls.load(Ordering::SeqCst), before + 2);

        let mut expired = previous.clone();
        expired.metadata.expires = 0;
        slot.write(Some(expired)).await;
        *document.lock().unwrap() = json!({"error":"maintenance"});
        assert!(load().await.is_err());
        let retained = slot.read().await.unwrap();
        assert_eq!(retained.data, previous.data);
        assert_eq!(
            retained.metadata.expires, 0,
            "rejection must not renew expiry"
        );
        *document.lock().unwrap() = json!({"paths":{"/recovered":{"get":{}}}});
        assert_eq!(load().await.unwrap().tools[0].path, "/recovered");
        assert_eq!(calls.load(Ordering::SeqCst), before + 4);
    }
    server.abort();
}

#[tokio::test]
async fn rejected_revalidated_catalog_does_not_renew_cached_content() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (address, server) = crate::test_tls::serve(Router::new().route(
        "/spec",
        get({
            let calls = calls.clone();
            move |headers: axum::http::HeaderMap| {
                let calls = calls.clone();
                async move {
                    assert_eq!(headers["if-none-match"], "\"old\"");
                    calls.fetch_add(1, Ordering::SeqCst);
                    (axum::http::StatusCode::NOT_MODIFIED, [("etag", "\"old\"")]).into_response()
                }
            }
        }),
    ))
    .await;
    let directory = tempfile::tempdir().unwrap();
    let config: SourceConfig = toml::from_str(&format!(
        "spec='https://{address}/spec'\nbase_url='https://{address}'\n"
    ))
    .unwrap();
    let path = directory.path().join("deployment.toml");
    let downloads = Downloads {
        cache_directory: Some(directory.path().join("http-cache")),
        ..Default::default()
    };
    let cfg = downloads
        .configure("api", &config, &path, false)
        .await
        .unwrap();
    let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let headers = reqwest::header::HeaderMap::from_iter([(
        reqwest::header::ETAG,
        "\"old\"".parse().unwrap(),
    )]);
    let mut metadata = Metadata::from_catalog_headers(
        &headers,
        Duration::ZERO,
        None,
        cfg.catalog_cache_ttl_seconds,
    )
    .unwrap();
    metadata.expires = 0;
    // Model an expired semantically invalid snapshot left by the old loader.
    let bytes = serde_json::to_vec(&json!({"error":"maintenance"})).unwrap();
    slot.write(Some(Entry {
        hints: None,
        metadata,
        data: bytes.clone(),
    }))
    .await;
    assert!(load_one("api", &config, Ok(cfg), &downloads).await.is_err());
    let retained = slot.read().await.unwrap();
    assert_eq!(retained.data, bytes);
    assert_eq!(retained.metadata.expires, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.abort();
}
