use super::*;
use crate::{
    catalog::{self, Config},
    pricing::{CachePath, PricingCache},
};
use axum::{
    Router,
    extract::{Request, State},
    response::{IntoResponse, Response},
    routing::any,
};
use std::sync::{Arc, Mutex};

fn headers(values: &[(&str, &str)]) -> HeaderMap {
    values
        .iter()
        .map(|(k, v)| {
            (
                k.parse::<reqwest::header::HeaderName>().unwrap(),
                v.parse().unwrap(),
            )
        })
        .collect()
}
#[test]
fn private_http_freshness_and_validators() {
    let m = |h: &[(&str, &str)]| Metadata::from_headers(&headers(h), Duration::ZERO, None);
    assert!(m(&[]).is_none());
    assert!(m(&[("cache-control", "extension=\"foo,max-age=600\"")]).is_none());
    assert!(m(&[("cache-control", "max-age=\"600")]).is_none());
    assert!(m(&[("cache-control", "max-age=\"600\"")]).unwrap().fresh());
    for directive in ["no-store, max-age=60", "max-age=oops"] {
        assert!(m(&[("cache-control", directive)]).is_none());
    }
    assert!(!m(&[("etag", "\"v1\"")]).unwrap().fresh());
    assert!(
        !m(&[("cache-control", "no-cache, max-age=600")])
            .unwrap()
            .fresh()
    );
    assert!(
        !m(&[("cache-control", "max-age=60"), ("age", "60")])
            .unwrap()
            .fresh()
    );
    assert!(
        !m(&[
            ("cache-control", "max-age=60"),
            ("date", "Sun, 06 Nov 1994 08:49:37 GMT")
        ])
        .unwrap()
        .fresh()
    );
    assert!(m(&[("cache-control", "max-age=60")]).unwrap().fresh());
    assert!(
        m(&[("cache-control", "max-age=60"), ("vary", "accept-encoding")])
            .unwrap()
            .fresh()
    );
    assert!(
        m(&[("cache-control", "max-age=60"), ("set-cookie", "private")])
            .unwrap()
            .fresh()
    );
    let expires = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
    assert!(m(&[("expires", &expires)]).unwrap().fresh());
    let mut rollback = m(&[("cache-control", "max-age=60")]).unwrap();
    rollback.stored += 10;
    assert!(!rollback.fresh());
}

#[derive(Default)]
struct Origin {
    calls: Vec<HeaderMap>,
    mode: usize,
}
async fn handler(State(state): State<Arc<Mutex<Origin>>>, request: Request) -> Response {
    assert_eq!(request.method(), "GET");
    assert!(!request.headers().contains_key("payment-signature"));
    let mut state = state.lock().unwrap();
    state.calls.push(request.headers().clone());
    let date = httpdate::fmt_http_date(SystemTime::now());
    match state.mode {
        0 => (
            axum::http::StatusCode::OK,
            [("cache-control", "max-age=600"), ("etag", "\"one\"")],
            "{\"value\":1}",
        )
            .into_response(),
        1 => (
            axum::http::StatusCode::NOT_MODIFIED,
            [
                ("cache-control", "max-age=600"),
                ("date", date.as_str()),
                ("etag", "\"one\""),
            ],
            "",
        )
            .into_response(),
        2 => (
            axum::http::StatusCode::OK,
            [("cache-control", "no-store")],
            "{\"value\":2}",
        )
            .into_response(),
        3 => (axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
        4 => (
            axum::http::StatusCode::OK,
            [("cache-control", "max-age=600")],
            "invalid JSON",
        )
            .into_response(),
        5..=7 => {
            use base64::Engine;
            let challenge = base64::engine::general_purpose::STANDARD
                .encode(r#"{"accepts":[{"amount":"1000","asset":"USDC"}]}"#);
            let cache = match state.mode {
                5 => "max-age=600",
                6 => "no-store",
                _ => "no-cache",
            };
            (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                [
                    ("cache-control", cache),
                    ("payment-required", challenge.as_str()),
                ],
                "",
            )
                .into_response()
        }
        9 => (axum::http::StatusCode::OK, "{\"value\":9}").into_response(),
        10 => (
            axum::http::StatusCode::OK,
            [
                ("cache-control", "private, max-age=600"),
                ("vary", "Accept-Encoding, Cookie, User-Agent"),
                ("set-cookie", "private-cookie=secret"),
            ],
            "{\"value\":10}",
        )
            .into_response(),
        11 => (
            axum::http::StatusCode::OK,
            [("cache-control", "no-cache"), ("etag", "\"one\"")],
            "{\"value\":11}",
        )
            .into_response(),
        12 => (
            axum::http::StatusCode::OK,
            [("cache-control", "max-age=600"), ("vary", "*")],
            "{\"value\":12}",
        )
            .into_response(),
        _ => (
            axum::http::StatusCode::OK,
            [("cache-control", "max-age=600")],
            "x".repeat(2048),
        )
            .into_response(),
    }
}
async fn fixture() -> (String, Arc<Mutex<Origin>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/catalog", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(Origin::default()));
    let app = Router::new()
        .fallback(any(handler))
        .with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, state, task)
}
fn config(dir: &Path) -> Config {
    Config {
        http_cache_directory: Some(dir.join("http-cache")),
        ..Default::default()
    }
}
fn client(url: &str) -> reqwest::Client {
    crate::network::provider_discovery(url, Duration::from_secs(5), Default::default()).unwrap()
}
async fn catalog(url: &str, cfg: &Config) -> Result<serde_json::Value> {
    catalog::load_json_cached(
        url,
        &client(url),
        cfg.max_spec_bytes,
        Slot::new(cfg, url, "catalog", cfg.max_spec_bytes),
    )
    .await
}
#[tokio::test]
async fn catalogs_survive_reopen_revalidate_replace_and_never_serve_stale_errors() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let (url, origin, task) = fixture().await;
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    assert_eq!(origin.lock().unwrap().calls.len(), 1);
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let expire = |slot: Slot| async move {
        let mut entry = slot.read().await.unwrap();
        entry.metadata.expires = 0;
        slot.write(Some(entry)).await;
    };
    expire(slot.clone()).await;
    origin.lock().unwrap().mode = 1;
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    assert_eq!(
        origin.lock().unwrap().calls.last().unwrap()["if-none-match"],
        "\"one\""
    );
    assert!(slot.read().await.unwrap().metadata.fresh());
    expire(slot.clone()).await;
    origin.lock().unwrap().mode = 3;
    let error = catalog(&url, &cfg).await.unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<reqwest::Error>()
            .unwrap()
            .status()
            .unwrap()
            .as_u16(),
        503
    );
    assert!(slot.read().await.is_none());
    origin.lock().unwrap().mode = 0;
    catalog(&url, &cfg).await.unwrap();
    expire(slot.clone()).await;
    origin.lock().unwrap().mode = 2;
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 2);
    assert!(slot.read().await.unwrap().metadata.fresh());
    expire(slot.clone()).await;
    origin.lock().unwrap().mode = 4;
    assert!(catalog(&url, &cfg).await.is_err());
    assert!(slot.read().await.is_none());
    task.abort();
}
#[tokio::test]
async fn pricing_persists_only_fresh_estimates_and_never_challenges() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let (url, origin, task) = fixture().await;
    origin.lock().unwrap().mode = 5;
    let root = serde_json::json!({"operations":[{"method":"GET","path":"/catalog"}]});
    let tools = catalog::build_tools(&cfg, &root, "test").unwrap();
    let base = url.strip_suffix("/catalog").unwrap();
    let first = PricingCache::default()
        .discover_observed(&cfg, &root, &tools, base)
        .await
        .unwrap();
    assert_eq!(first.evidence.available_prices, 1);
    let second = PricingCache::default()
        .discover_observed(&cfg, &root, &tools, base)
        .await
        .unwrap();
    assert_eq!(second.prices, first.prices);
    assert_eq!(second.evidence.cache[&CachePath::Disk], 1);
    assert_eq!(origin.lock().unwrap().calls.len(), 1);
    let slot = Slot::new(&cfg, &url, "pricing", 64 * 1024).unwrap();
    let mut entry = slot.read().await.unwrap();
    assert_eq!(
        String::from_utf8(entry.data.clone()).unwrap(),
        "Cost: ~$0.001/call [x402 probe]."
    );
    assert!(
        !serde_json::to_string(&entry.metadata)
            .unwrap()
            .contains("payment-required")
    );
    entry.metadata.stored -= 30;
    slot.write(Some(entry.clone())).await;
    let process = PricingCache::default();
    process
        .discover_observed(&cfg, &root, &tools, base)
        .await
        .unwrap();
    let mut shorter = cfg.clone();
    shorter.probe_ttl_seconds = 10.0;
    let expired = process
        .discover_observed(&shorter, &root, &tools, base)
        .await
        .unwrap();
    assert!(expired.prices.is_empty());
    assert_eq!(expired.evidence.expired, 1);
    assert_eq!(origin.lock().unwrap().calls.len(), 1);
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    for mode in [6, 7] {
        origin.lock().unwrap().mode = mode;
        PricingCache::default()
            .discover_observed(&cfg, &root, &tools, base)
            .await
            .unwrap();
        assert!(slot.read().await.is_none());
        assert!(
            !origin
                .lock()
                .unwrap()
                .calls
                .last()
                .unwrap()
                .contains_key("if-none-match")
        );
    }
    task.abort();
}
#[tokio::test]
async fn opt_out_isolation_missing_state_and_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    let (url, origin, task) = fixture().await;
    catalog(&url, &cfg).await.unwrap();
    cfg.allow_http1 = true;
    catalog(&url, &cfg).await.unwrap();
    cfg.http_cache_enabled = false;
    catalog(&url, &cfg).await.unwrap();
    catalog(&url, &cfg).await.unwrap();
    assert_eq!(origin.lock().unwrap().calls.len(), 4);
    cfg.http_cache_enabled = true;
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let mut entry = slot.read().await.unwrap();
    entry.data = b"broken".to_vec();
    slot.write(Some(entry)).await;
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    cfg.http_cache_directory = Some(dir.path().join("missing/http-cache"));
    catalog(&url, &cfg).await.unwrap();
    assert!(!dir.path().join("missing").exists());
    task.abort();
}

#[tokio::test]
async fn origin_limit_has_consumer_error_and_sanitized_log() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.max_spec_bytes = 1024;
    let (url, origin, task) = fixture().await;
    origin.lock().unwrap().mode = 8;
    let error = catalog(&url, &cfg).await.unwrap_err();
    assert!(format!("{error:#}").contains("max_spec_bytes=1024"));
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(logs.contains("download limit exceeded"));
    assert!(logs.contains("limit_bytes=1024"));
    assert!(!logs.contains(&url));
    assert!(
        Slot::new(&cfg, &url, "catalog", 1024)
            .unwrap()
            .read()
            .await
            .is_none()
    );
    task.abort();
}

#[tokio::test]
async fn deployment_uses_existing_state_only_and_qualification_bypasses() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/spec", listener.local_addr().unwrap());
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = count.clone();
    let app = Router::new().fallback(any(move || {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ([("cache-control", "max-age=600")], r#"{"servers":[{"url":"https://api.example"}],"operations":[{"method":"GET","path":"/read"}]}"#)
        }
    }));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        format!(
            r#"
version=1
[treasury]
state_dir="state"
daily_treasury_spend_limit_zec="0.01"
max_refund_shielding_fee_zec="0.001"
[wallets.w]
mode="static"
private_key_env="UNUSED_TEST_KEY"
max_api_payment_usdc="0.01"
[sources.api]
spec="{url}"
probe_pricing=false
[servers.test]
listen="127.0.0.1:0"
bearer_token_env="UNUSED_TEST_TOKEN"
wallet="w"
sources=["api"]
"#
        ),
    )
    .unwrap();
    crate::deployment::Deployment::load(&path).await.unwrap();
    assert!(!dir.path().join("state").exists());
    std::fs::create_dir(dir.path().join("state")).unwrap();
    crate::deployment::Deployment::load(&path).await.unwrap();
    crate::deployment::Deployment::load(&path).await.unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        dir.path()
            .join("state/http-cache/discovery-v1.sqlite")
            .is_file()
    );
    let (inspected, observations) = crate::deployment::Deployment::inspect_catalogs(&path).await;
    inspected.unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);
    // Treasury keys and database were never opened or created by catalog loading.
    assert!(!dir.path().join("state/wallet.key").exists());
    assert!(!dir.path().join("state/state.sqlite").exists());
    task.abort();
}

#[tokio::test]
async fn cache_bounds_evict_and_report_without_losing_complete_catalog() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.max_spec_bytes = 1024;
    let (url, _, task) = fixture().await;
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    catalog(&url, &cfg).await.unwrap();
    let entry = slot.read().await.unwrap();
    let db = open(&slot.directory).unwrap();
    db.execute(
        "UPDATE entries SET data=zeroblob(1025) WHERE key=?1",
        [&slot.key],
    )
    .unwrap();
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    db.execute("DELETE FROM entries", []).unwrap();
    db.execute("WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<4096) INSERT INTO entries SELECT printf('%04d',n),'{}',x'00',0 FROM ids", []).unwrap();
    slot.write(Some(entry)).await;
    let count: i64 = db
        .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, ENTRIES as i64);
    assert!(slot.read().await.is_some());
    let logs = std::fs::read_to_string(log.path()).unwrap();
    assert!(logs.contains("entry exceeds resource limit"));
    assert!(logs.contains("limit_bytes=1024"));
    assert!(logs.contains("oldest disposable entry evicted"));
    assert!(logs.contains("limit_entries=4096"));
    assert!(!logs.contains(&url));
    task.abort();
}

#[tokio::test]
async fn last_modified_and_conflicting_validator_handling() {
    let metadata = Metadata::from_headers(
        &headers(&[("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT")]),
        Duration::ZERO,
        None,
    )
    .unwrap();
    let url = "https://example.com/catalog";
    let request = metadata.conditional(client(url).get(url)).build().unwrap();
    assert_eq!(
        request.headers()["if-modified-since"],
        "Sun, 06 Nov 1994 08:49:37 GMT"
    );
    let metadata =
        Metadata::from_headers(&headers(&[("etag", "\"one\"")]), Duration::ZERO, None).unwrap();
    assert!(!metadata.matches_validation(&headers(&[("etag", "\"two\"")])));
    assert!(metadata.matches_validation(&HeaderMap::new()));
}

#[tokio::test]
async fn explicit_direct_entries_have_separate_provenance_and_require_freshness() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let url = "https://example.com/spec";
    let mut warming = cfg.clone();
    warming.http_cache_direct_warm_target = Some(crate::network::global().policy.clone());
    let direct = Slot::new(&warming, url, "catalog", 1024).unwrap();
    let normal = Slot::new(&cfg, url, "catalog", 1024).unwrap();
    assert_ne!(direct.key, normal.key);
    let metadata = Metadata::from_headers(
        &headers(&[("cache-control", "max-age=600"), ("etag", "\"direct\"")]),
        Duration::ZERO,
        None,
    )
    .unwrap();
    let mut entry = Entry {
        metadata,
        data: b"{}".to_vec(),
    };
    direct.write(Some(entry.clone())).await;
    assert!(normal.read_primary().await.is_none());
    let mut reused = normal.read().await.unwrap();
    assert!(!reused.metadata.has_validator());
    // Even if freshness expires during parsing, the direct validator cannot leak.
    reused.metadata.expires = 0;
    let request = reused
        .metadata
        .conditional(client(url).get(url))
        .build()
        .unwrap();
    assert!(!request.headers().contains_key("if-none-match"));
    let mut other = cfg.clone();
    other.allow_http1 = true;
    assert!(
        Slot::new(&other, url, "catalog", 1024)
            .unwrap()
            .read()
            .await
            .is_none()
    );
    entry.metadata.expires = 0;
    direct.write(Some(entry)).await;
    // A warm command may revalidate its own stale entry using direct egress;
    // a normal configured-network consumer must not receive that validator.
    assert!(direct.read().await.unwrap().metadata.has_validator());
    assert!(normal.read().await.is_none());
}

#[tokio::test]
async fn catalog_cache_logs_source_hit_revalidation_and_rejected_policy_without_urls() {
    use tracing::instrument::WithSubscriber;
    let dir = tempfile::tempdir().unwrap();
    let (url, origin, task) = fixture().await;
    let mut cfg = config(dir.path());
    cfg.spec = format!("{url}?token=private-token");
    cfg.discovery_source = Some("fixture_source".into());
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    async {
        let http = client(&cfg.spec);
        catalog::load_json_discovery(&cfg, &http).await.unwrap();
        catalog::load_json_discovery(&cfg, &http).await.unwrap();
        let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
        let mut entry = slot.read().await.unwrap();
        entry.metadata.expires = 0;
        slot.write(Some(entry)).await;
        origin.lock().unwrap().mode = 1;
        catalog::load_json_discovery(&cfg, &http).await.unwrap();
        let mut entry = slot.read().await.unwrap();
        entry.metadata.expires = 0;
        slot.write(Some(entry)).await;
        origin.lock().unwrap().mode = 2;
        catalog::load_json_discovery(&cfg, &http).await.unwrap();
        assert!(slot.read().await.unwrap().metadata.fresh());
    }
    .with_subscriber(subscriber)
    .await;
    let logs = std::fs::read_to_string(log.path()).unwrap();
    for field in [
        "fixture_source",
        "cache=\"miss\"",
        "cache=\"stored\"",
        "cache=\"disk_hit\"",
        "cache=\"revalidate\"",
        "cache=\"revalidated\"",
        "origin_policy=\"no_store\"",
    ] {
        assert!(logs.contains(field), "missing {field}: {logs}");
    }
    assert!(!logs.contains(&url));
    assert!(!logs.contains("private-token"));
    assert_eq!(origin.lock().unwrap().calls.len(), 3);
    task.abort();
}

#[test]
fn cache_policy_labels_explain_conservative_rejections() {
    for (values, expected) in [
        (vec![], "no_cache_headers"),
        (vec![("vary", "*")], "vary_star"),
        (vec![("cache-control", "no-store")], "no_store"),
        (
            vec![("cache-control", "max-age=bad")],
            "unsupported_or_incomplete_cache_headers",
        ),
        (
            vec![
                ("cache-control", "max-age=0, must-revalidate"),
                ("etag", "private-validator"),
            ],
            "requires_revalidation",
        ),
    ] {
        let headers = headers(&values);
        let metadata = Metadata::from_headers(&headers, Duration::ZERO, None);
        assert_eq!(
            Metadata::policy_label(&headers, metadata.as_ref()),
            expected
        );
    }
}

#[test]
fn catalog_local_ttl_overrides_origin_freshness_and_logs_policy() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        for values in [
            vec![],
            vec![("cache-control", "no-store")],
            vec![("cache-control", "no-cache")],
            vec![("cache-control", "max-age=0, must-revalidate")],
            vec![("cache-control", "max-age=999999")],
            vec![("cache-control", "max-age=bad")],
            vec![("expires", "0"), ("age", "999999")],
        ] {
            let h = headers(&values);
            for ttl in [86400, 120] {
                let m = Metadata::from_catalog_headers(&h, Duration::ZERO, None, ttl).unwrap();
                assert!(m.local_override && m.fresh());
                assert_eq!(m.expires - m.stored, ttl);
                assert_eq!(Metadata::policy_label(&h, Some(&m)), "local_override");
            }
            assert!(Metadata::from_catalog_headers(&h, Duration::ZERO, None, 0).is_none());
        }
    });
    let logs = std::fs::read_to_string(log.path()).unwrap();
    for expected in [
        "INFO",
        "ttl_seconds=86400",
        "ttl_seconds=120",
        "origin_policy=\"no_store\"",
        "cache_policy=\"local_override\"",
    ] {
        assert!(logs.contains(expected), "missing {expected}: {logs}");
    }
    assert!(Metadata::from_headers(&HeaderMap::new(), Duration::ZERO, None).is_none());
    let delayed =
        Metadata::from_relay_headers(&HeaderMap::new(), Duration::from_secs(121), Some(120))
            .unwrap();
    assert!(
        !delayed.fresh(),
        "coalesced relay reuse must not renew lifetime"
    );
}

#[tokio::test]
async fn headerless_and_private_vary_catalogs_reuse_disk_without_cookies() {
    for mode in [2, 9, 10, 11, 12] {
        let dir = tempfile::tempdir().unwrap();
        let (url, origin, task) = fixture().await;
        origin.lock().unwrap().mode = mode;
        let mut cfg = config(dir.path());
        cfg.spec = url.clone();
        for _ in 0..2 {
            // New client and load: exercise persisted reuse, not a process cache.
            assert_eq!(
                catalog::load_json_discovery(&cfg, &client(&url))
                    .await
                    .unwrap()["value"],
                mode
            );
        }
        {
            let origin = origin.lock().unwrap();
            assert_eq!(origin.calls.len(), if mode == 12 { 2 } else { 1 });
            for h in &origin.calls {
                assert_eq!(h["accept-encoding"], "identity");
                assert_eq!(h["accept"], "*/*");
                assert!(!h.contains_key("cookie") && !h.contains_key("authorization"));
            }
        }
        let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
        let saved = slot.read().await;
        if mode == 12 {
            assert!(saved.is_none());
        } else {
            let saved = saved.unwrap();
            assert!(
                !serde_json::to_string(&saved.metadata)
                    .unwrap()
                    .contains("private-cookie")
            );
        }
        task.abort();
    }
}

#[test]
fn lists_extensions_private_variants_and_304_refresh_are_supported() {
    let mut h = headers(&[
        ("cache-control", "private, extension=\"ignored,max-age=0\""),
        ("vary", "Accept-Encoding"),
        ("set-cookie", "not-persisted"),
    ]);
    h.append("cache-control", "max-age=600".parse().unwrap());
    h.append("vary", "Cookie".parse().unwrap());
    let mut m = Metadata::from_catalog_headers(&h, Duration::ZERO, None, 86400).unwrap();
    assert!(m.fresh());
    assert!(!m.headers.contains_key("set-cookie"));
    m.expires = 0;
    let validation = headers(&[("vary", "Accept-Encoding, Cookie")]);
    assert!(m.matches_validation(&validation));
    let refreshed =
        Metadata::from_catalog_headers(&validation, Duration::ZERO, Some(&m), 86400).unwrap();
    assert!(refreshed.fresh() && refreshed.local_override);
    let restrictive = Metadata::from_catalog_headers(
        &headers(&[("cache-control", "max-age=60, max-age=0")]),
        Duration::ZERO,
        None,
        86400,
    )
    .unwrap();
    assert!(restrictive.fresh());
    assert!(
        Metadata::from_catalog_headers(
            &headers(&[("vary", "if-none-match")]),
            Duration::ZERO,
            None,
            86400
        )
        .is_none()
    );
    assert!(
        Metadata::from_relay_headers(&h, Duration::ZERO, Some(86400)).is_none(),
        "relay target request headers are unknown"
    );
}

#[tokio::test]
async fn catalog_ttl_zero_and_changes_are_isolated_from_pricing() {
    let dir = tempfile::tempdir().unwrap();
    let (url, origin, task) = fixture().await;
    let mut cfg = config(dir.path());
    catalog(&url, &cfg).await.unwrap();
    let pricing = Slot::new(&cfg, &url, "pricing", 1024).unwrap();
    cfg.catalog_cache_ttl_seconds = 120;
    catalog(&url, &cfg).await.unwrap();
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let metadata = slot.read().await.unwrap().metadata;
    assert_eq!(metadata.expires - metadata.stored, 120);
    cfg.catalog_cache_ttl_seconds = 0;
    assert!(Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).is_none());
    assert_eq!(
        pricing.key,
        Slot::new(&cfg, &url, "pricing", 1024).unwrap().key
    );
    catalog(&url, &cfg).await.unwrap();
    catalog(&url, &cfg).await.unwrap();
    assert_eq!(origin.lock().unwrap().calls.len(), 4);
    task.abort();
}
