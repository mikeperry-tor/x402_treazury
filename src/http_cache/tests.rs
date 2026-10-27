use super::*;
use crate::catalog::{self, Config};
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
        13 => (
            axum::http::StatusCode::NOT_MODIFIED,
            [("etag", "\"two\"")],
            "",
        )
            .into_response(),
        14 => (
            axum::http::StatusCode::CREATED,
            [("cache-control", "max-age=600"), ("etag", "\"created\"")],
            "{\"value\":14}",
        )
            .into_response(),
        15 => (
            axum::http::StatusCode::NOT_MODIFIED,
            [("etag", "\"one\""), ("vary", "*")],
            "",
        )
            .into_response(),
        16 => (axum::http::StatusCode::PARTIAL_CONTENT, "{\"value\":16}").into_response(),
        17 => (
            axum::http::StatusCode::OK,
            [("content-range", "bytes 0-11/100")],
            "{\"value\":17}",
        )
            .into_response(),
        18 => (axum::http::StatusCode::NO_CONTENT, "").into_response(),
        19 => (axum::http::StatusCode::FOUND, "{\"value\":19}").into_response(),
        20 => (
            axum::http::StatusCode::NOT_MODIFIED,
            [("content-range", "bytes 0-11/100")],
            "",
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
    let mut cfg = cfg.clone();
    cfg.spec = url.to_owned();
    catalog::load_json_discovery(&cfg).await
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
    {
        let state = origin.lock().unwrap();
        assert_eq!(state.calls.len(), 5);
        assert!(!state.calls[4].contains_key("if-none-match"));
        assert!(!state.calls[4].contains_key("if-modified-since"));
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&slot.read().await.unwrap().data).unwrap()["value"],
        1
    );
    assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 1);
    assert_eq!(origin.lock().unwrap().calls.len(), 5);
    cfg.http_cache_directory = Some(dir.path().join("missing/http-cache"));
    catalog(&url, &cfg).await.unwrap();
    assert!(!dir.path().join("missing").exists());
    task.abort();
}

#[tokio::test]
async fn origin_limit_has_consumer_error_and_sanitized_log() {
    if crate::test_log_capture::run_in_child(
        "http_cache::tests::origin_limit_has_consumer_error_and_sanitized_log",
    ) {
        return;
    }
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
    catalog(&url, &cfg).await.unwrap();
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let mut entry = slot.read().await.unwrap();
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    origin.lock().unwrap().mode = 8;
    let error = catalog(&url, &cfg).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<catalog::LoadStage>(),
        Some(&catalog::LoadStage::Body)
    );
    {
        let state = origin.lock().unwrap();
        assert_eq!(state.calls.len(), 2);
        assert_eq!(state.calls[1]["if-none-match"], "\"one\"");
    }
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
max_refund_shielding_fee_zec="0.001"
[wallets.w]
mode="static"
private_key_env="UNUSED_TEST_KEY"
max_api_payment_usdc="0.01"
[sources.api]
spec="{url}"
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
        .with_max_level(tracing::Level::DEBUG)
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
    assert!(
        logs.lines()
            .any(|line| line.contains("DEBUG") && line.contains("oldest disposable entry evicted"))
    );
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
    if crate::test_log_capture::run_in_child(
        "http_cache::tests::catalog_cache_logs_source_hit_revalidation_and_rejected_policy_without_urls",
    ) {
        return;
    }
    use tracing::instrument::WithSubscriber;
    for debug in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (url, origin, task) = fixture().await;
        let mut cfg = config(dir.path());
        cfg.spec = format!("{url}?token=private-token");
        cfg.discovery_source = Some("fixture_source".into());
        let log = tempfile::NamedTempFile::new().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(if debug {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            })
            .with_writer(log.reopen().unwrap())
            .finish();
        async {
            catalog::load_json_discovery(&cfg).await.unwrap();
            catalog::load_json_discovery(&cfg).await.unwrap();
            let slot = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
            let mut entry = slot.read().await.unwrap();
            entry.metadata.expires = 0;
            slot.write(Some(entry)).await;
            origin.lock().unwrap().mode = 1;
            catalog::load_json_discovery(&cfg).await.unwrap();
            let mut entry = slot.read().await.unwrap();
            entry.metadata.expires = 0;
            slot.write(Some(entry)).await;
            origin.lock().unwrap().mode = 2;
            catalog::load_json_discovery(&cfg).await.unwrap();
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
            assert_eq!(
                logs.contains(field),
                debug,
                "unexpected visibility for {field}: {logs}"
            );
        }
        assert!(!logs.contains("INFO") && !logs.contains("WARN"), "{logs}");
        assert!(!logs.contains(&url));
        assert!(!logs.contains("private-token"));
        assert_eq!(origin.lock().unwrap().calls.len(), 3);
        task.abort();
    }
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
    if crate::test_log_capture::run_in_child(
        "http_cache::tests::catalog_local_ttl_overrides_origin_freshness_and_logs_policy",
    ) {
        return;
    }
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
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
        "DEBUG",
        "ttl_seconds=86400",
        "ttl_seconds=120",
        "origin_policy=\"no_store\"",
        "cache_policy=\"local_override\"",
    ] {
        assert!(logs.contains(expected), "missing {expected}: {logs}");
    }
    assert!(!logs.contains("INFO"), "{logs}");
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
                catalog::load_json_discovery(&cfg).await.unwrap()["value"],
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
async fn catalog_ttl_changes_partition_cache_and_zero_disables_it() {
    let dir = tempfile::tempdir().unwrap();
    let (url, origin, task) = fixture().await;
    let mut cfg = config(dir.path());
    catalog(&url, &cfg).await.unwrap();
    let original = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    cfg.catalog_cache_ttl_seconds = 120;
    catalog(&url, &cfg).await.unwrap();
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    assert_ne!(
        original.key, slot.key,
        "catalog lifetime changes cannot reuse a different policy slot"
    );
    let metadata = slot.read().await.unwrap().metadata;
    assert_eq!(metadata.expires - metadata.stored, 120);
    cfg.catalog_cache_ttl_seconds = 0;
    assert!(Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).is_none());
    catalog(&url, &cfg).await.unwrap();
    catalog(&url, &cfg).await.unwrap();
    assert_eq!(origin.lock().unwrap().calls.len(), 4);
    task.abort();
}

#[tokio::test]
async fn unsolicited_and_conflicting_304_fail_once_at_headers() {
    for cache_enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config(dir.path());
        cfg.http_cache_enabled = cache_enabled;
        let (url, origin, task) = fixture().await;
        origin.lock().unwrap().mode = 1;
        let error = catalog(&url, &cfg).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<catalog::LoadStage>(),
            Some(&catalog::LoadStage::Headers)
        );
        assert!(format!("{error:#}").contains("unsolicited catalog HTTP 304"));
        {
            let state = origin.lock().unwrap();
            assert_eq!(state.calls.len(), 1);
            assert!(!state.calls[0].contains_key("if-none-match"));
            assert!(!state.calls[0].contains_key("if-modified-since"));
        }
        if let Some(slot) = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes) {
            assert!(slot.read().await.is_none());
        }
        task.abort();
    }
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let (url, origin, task) = fixture().await;
    catalog(&url, &cfg).await.unwrap();
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let mut entry = slot.read().await.unwrap();
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    origin.lock().unwrap().mode = 13;
    let error = catalog(&url, &cfg).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<catalog::LoadStage>(),
        Some(&catalog::LoadStage::Headers)
    );
    assert!(format!("{error:#}").contains("did not validate cached representation"));
    {
        let state = origin.lock().unwrap();
        assert_eq!(state.calls.len(), 2);
        assert_eq!(state.calls[1]["if-none-match"], "\"one\"");
    }
    assert!(slot.read().await.is_none());
    task.abort();
}

#[tokio::test]
async fn successful_uncacheable_responses_remove_previous_catalog() {
    for mode in [14, 15] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let (url, origin, task) = fixture().await;
        catalog(&url, &cfg).await.unwrap();
        let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
        let mut entry = slot.read().await.unwrap();
        entry.metadata.expires = 0;
        slot.write(Some(entry)).await;
        origin.lock().unwrap().mode = mode;
        assert_eq!(
            catalog(&url, &cfg).await.unwrap()["value"],
            if mode == 14 { 14 } else { 1 }
        );
        {
            let state = origin.lock().unwrap();
            assert_eq!(state.calls.len(), 2);
            assert_eq!(state.calls[1]["if-none-match"], "\"one\"");
        }
        assert!(slot.read().await.is_none());
        // The successfully returned document does not grant persistence. The
        // next load must return to origin without the removed validator.
        origin.lock().unwrap().mode = 14;
        assert_eq!(catalog(&url, &cfg).await.unwrap()["value"], 14);
        {
            let state = origin.lock().unwrap();
            assert_eq!(state.calls.len(), 3);
            assert!(!state.calls[2].contains_key("if-none-match"));
        }
        assert!(slot.read().await.is_none());
        task.abort();
    }
}

#[test]
fn metadata_duplicate_headers_merge_lists_but_reject_conflicting_singletons() {
    let mut h = headers(&[
        ("cache-control", "private, max-age=600"),
        ("vary", "Cookie"),
    ]);
    h.append("cache-control", "MAX-AGE = \"30\"".parse().unwrap());
    h.append("vary", "Accept-Encoding".parse().unwrap());
    h.append("etag", "\"same\"".parse().unwrap());
    h.append("etag", "\"same\"".parse().unwrap());
    let m = Metadata::from_headers(&h, Duration::ZERO, None).unwrap();
    assert_eq!(m.expires - m.stored, 29);
    assert_eq!(m.headers["vary"], "Cookie, Accept-Encoding");
    assert_eq!(m.headers["etag"], "\"same\"");
    for (name, first, second) in [
        ("etag", "\"one\"", "\"two\""),
        ("age", "1", "2"),
        (
            "date",
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:38 GMT",
        ),
        ("expires", "0", "1"),
        ("last-modified", "one", "two"),
    ] {
        let mut h = headers(&[("cache-control", "max-age=600"), (name, first)]);
        h.append(
            name.parse::<reqwest::header::HeaderName>().unwrap(),
            second.parse().unwrap(),
        );
        assert!(
            Metadata::from_headers(&h, Duration::ZERO, None).is_none(),
            "{name}"
        );
    }
}

#[test]
fn metadata_freshness_accounts_for_delay_age_and_explicit_precedence() {
    let h = headers(&[
        ("cache-control", "max-age=100"),
        ("age", "20"),
        ("expires", "0"),
    ]);
    let m = Metadata::from_headers(&h, Duration::from_secs(7), None).unwrap();
    assert_eq!(
        m.expires - m.stored,
        72,
        "max-age overrides Expires; age and response delay both count"
    );
    // A fixed future Date avoids wall-clock boundary races while proving that
    // Expires is relative to the response Date, not the receipt clock.
    let response_date = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(4_000_000_000));
    let expires = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(4_000_000_120));
    let dated = Metadata::from_headers(
        &headers(&[("date", &response_date), ("expires", &expires)]),
        Duration::from_secs(9),
        None,
    )
    .unwrap();
    assert_eq!(dated.expires - dated.stored, 110);
    let historical = Metadata::from_headers(
        &headers(&[
            ("date", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("expires", "Sun, 06 Nov 1994 08:50:37 GMT"),
        ]),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert_eq!(historical.expires, historical.stored);
    let exhausted = Metadata::from_headers(&h, Duration::from_secs(100), None).unwrap();
    assert_eq!(exhausted.expires, exhausted.stored);
    for (name, value) in [
        ("date", "invalid"),
        ("age", "-1"),
        ("age", "18446744073709551616"),
    ] {
        let h = headers(&[("cache-control", "max-age=100"), (name, value)]);
        assert!(Metadata::from_headers(&h, Duration::ZERO, None).is_none());
    }
    let h = headers(&[("expires", "invalid")]);
    let m = Metadata::from_headers(&h, Duration::ZERO, None).unwrap();
    assert_eq!(m.expires, m.stored, "invalid Expires grants no freshness");
    let h = headers(&[
        ("cache-control", "max-age=18446744073709551616"),
        ("age", "18446744073709551615"),
    ]);
    let m = Metadata::from_headers(&h, Duration::MAX, None).unwrap();
    assert_eq!(
        m.expires, m.stored,
        "overflow must saturate without renewing freshness"
    );
}

#[test]
fn metadata_validation_replaces_age_observation_and_preserves_omitted_policy() {
    let old = Metadata::from_headers(
        &headers(&[
            ("cache-control", "max-age=600"),
            ("date", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("age", "9999"),
            ("etag", "\"old\""),
            ("vary", "Cookie"),
        ]),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert!(!old.fresh());
    let refreshed = Metadata::from_headers(
        &headers(&[("etag", "\"new\"")]),
        Duration::from_secs(4),
        Some(&old),
    )
    .unwrap();
    assert!(!refreshed.headers.contains_key("age"));
    assert_ne!(refreshed.headers["date"], old.headers["date"]);
    assert_eq!(refreshed.headers["etag"], "\"new\"");
    assert_eq!(refreshed.headers["vary"], "Cookie");
    assert_eq!(refreshed.headers["cache-control"], "max-age=600");
    assert!(refreshed.fresh());
    assert!(refreshed.expires - refreshed.stored <= 595);
    let explicit_date = Metadata::from_headers(
        &headers(&[("date", "Sun, 06 Nov 1994 08:50:37 GMT")]),
        Duration::ZERO,
        Some(&old),
    )
    .unwrap();
    assert_eq!(
        explicit_date.headers["date"],
        "Sun, 06 Nov 1994 08:50:37 GMT"
    );
    assert_eq!(
        explicit_date.expires, explicit_date.stored,
        "an explicit validation Date overrides the synthesized receipt Date"
    );
    let replaced = Metadata::from_headers(
        &headers(&[("cache-control", "no-cache"), ("age", "10")]),
        Duration::ZERO,
        Some(&old),
    )
    .unwrap();
    assert_eq!(replaced.expires, 0);
    assert_eq!(replaced.headers["age"], "10");
    assert!(
        Metadata::from_headers(
            &headers(&[("cache-control", "no-store")]),
            Duration::ZERO,
            Some(&old)
        )
        .is_none()
    );
    assert_eq!(
        old.headers["age"], "9999",
        "validation does not mutate the old observation"
    );
}

#[test]
fn metadata_rejects_unsafe_variants_and_enforces_selected_header_bound() {
    for vary in [
        "*",
        "Accept, *",
        "If-None-Match",
        "if-modified-since",
        "bad name",
    ] {
        assert!(
            Metadata::from_headers(
                &headers(&[("cache-control", "max-age=60"), ("vary", vary)]),
                Duration::ZERO,
                None
            )
            .is_none(),
            "{vary}"
        );
    }
    let mut h = headers(&[
        ("cache-control", "max-age=60"),
        ("vary", " , Cookie, , User-Agent,"),
    ]);
    h.insert(
        "etag",
        reqwest::header::HeaderValue::from_bytes(&[0xff]).unwrap(),
    );
    assert!(Metadata::from_headers(&h, Duration::ZERO, None).is_none());
    h.remove("etag");
    assert!(Metadata::from_headers(&h, Duration::ZERO, None).is_some());
    let exact = "x".repeat(METADATA_LIMIT);
    let m = Metadata::from_headers(&headers(&[("etag", &exact)]), Duration::ZERO, None).unwrap();
    assert!(!m.fresh());
    let oversized = "x".repeat(METADATA_LIMIT + 1);
    assert!(
        Metadata::from_headers(&headers(&[("etag", &oversized)]), Duration::ZERO, None).is_none()
    );
    let h = headers(&[("cache-control", "max-age=60"), ("set-cookie", &oversized)]);
    let m = Metadata::from_headers(&h, Duration::ZERO, None).unwrap();
    assert!(
        !m.headers.contains_key("set-cookie"),
        "discarded headers neither persist nor consume metadata capacity"
    );
}

#[test]
fn metadata_directive_quoting_does_not_hide_real_policy() {
    for directive in [
        "max-age",
        "max-age=",
        "max-age=\"\"",
        "max-age=-1",
        "max-age=+1",
        "max-age=1.5",
        "max-age=\"60",
        "extension=\"unfinished\\",
    ] {
        assert!(
            Metadata::from_headers(
                &headers(&[("cache-control", directive)]),
                Duration::ZERO,
                None
            )
            .is_none(),
            "{directive}"
        );
    }
    let h = headers(&[(
        "cache-control",
        r#"private, s-maxage=0, extension="escaped\",no-store", max-age=60"#,
    )]);
    let m = Metadata::from_headers(&h, Duration::ZERO, None).unwrap();
    assert_eq!(m.expires - m.stored, 59);
    let h = headers(&[("cache-control", "max-age=60, NO-CACHE=\"etag\"")]);
    assert_eq!(
        Metadata::from_headers(&h, Duration::ZERO, None)
            .unwrap()
            .expires,
        0
    );
}

#[test]
fn retry_after_parses_seconds_and_http_dates_and_ignores_invalid_or_conflicting_headers() {
    let status_429 = reqwest::StatusCode::TOO_MANY_REQUESTS;
    let status_503 = reqwest::StatusCode::SERVICE_UNAVAILABLE;
    let status_403 = reqwest::StatusCode::FORBIDDEN;

    assert_eq!(
        crate::network::parse_retry_after(status_429, &headers(&[("retry-after", "2")])),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        crate::network::parse_retry_after(status_503, &headers(&[("retry-after", " 1 ")])),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        crate::network::parse_retry_after(status_403, &headers(&[("retry-after", "2")])),
        None
    );
    for invalid in ["0", "-1", "abc", "", "Sun, 06 Nov 1994 08:49:37 GMT"] {
        assert_eq!(
            crate::network::parse_retry_after(status_429, &headers(&[("retry-after", invalid)])),
            None,
            "{invalid}"
        );
    }
    let future_date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(5));
    let parsed_date =
        crate::network::parse_retry_after(status_503, &headers(&[("retry-after", &future_date)]))
            .unwrap();
    assert!(parsed_date > Duration::ZERO && parsed_date <= Duration::from_secs(5));

    let mut dup_same = headers(&[("retry-after", "1")]);
    dup_same.append("retry-after", " 1 ".parse().unwrap());
    assert_eq!(
        crate::network::parse_retry_after(status_429, &dup_same),
        Some(Duration::from_secs(1))
    );
    let mut dup_conflict = headers(&[("retry-after", "1")]);
    dup_conflict.append("retry-after", "2".parse().unwrap());
    assert_eq!(
        crate::network::parse_retry_after(status_429, &dup_conflict),
        None
    );
}

#[tokio::test]
async fn unfollowed_catalog_redirects_consume_complete_json_without_retries_or_disk_caching() {
    use crate::network::{IsolationId, Mode, NetworkContext, NetworkPolicy};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    for mode in [Mode::Direct, Mode::Tor] {
        for code in [
            axum::http::StatusCode::FOUND,
            axum::http::StatusCode::TEMPORARY_REDIRECT,
        ] {
            for valid in [true, false] {
                for candidate in [false, true] {
                    if candidate && mode == Mode::Direct {
                        continue;
                    }
                    let calls = Arc::new(AtomicUsize::new(0));
                    let followed = Arc::new(AtomicUsize::new(0));
                    let observed = calls.clone();
                    let followed_route = followed.clone();
                    let app = Router::new()
                        .route(
                            "/spec",
                            axum::routing::get(move || {
                                let observed = observed.clone();
                                async move {
                                    let ordinal = observed.fetch_add(1, Ordering::SeqCst);
                                    if candidate && ordinal == 0 {
                                        return axum::http::StatusCode::FORBIDDEN.into_response();
                                    }
                                    (
                                        code,
                                        [
                                            ("location", "/must-not-follow"),
                                            ("cache-control", "max-age=600"),
                                        ],
                                        if valid {
                                            "{\"value\":1}"
                                        } else {
                                            "invalid catalog"
                                        },
                                    )
                                        .into_response()
                                }
                            }),
                        )
                        .route(
                            "/must-not-follow",
                            axum::routing::get(move || {
                                followed_route.fetch_add(1, Ordering::SeqCst);
                                async { "{\"value\":999}" }
                            }),
                        );
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let address = listener.local_addr().unwrap();
                    let server = tokio::spawn(async move {
                        axum::serve(listener, app).await.unwrap();
                    });
                    let socks = crate::test_socks::Socks::start(
                        BTreeMap::from([("api.example.com".into(), address)]),
                        crate::test_socks::Fault::None,
                    )
                    .await;
                    let network = NetworkContext::new(NetworkPolicy {
                        mode: mode.clone(),
                        socks_endpoint: (mode == Mode::Tor).then_some(socks.address),
                        ..Default::default()
                    })
                    .unwrap();
                    let dir = tempfile::tempdir().unwrap();
                    let mut cfg = config(dir.path());
                    cfg.spec = if mode == Mode::Tor {
                        "http://api.example.com/spec".into()
                    } else {
                        format!("http://{address}/spec")
                    };
                    let identity = IsolationId::discovery(&cfg.spec).unwrap();
                    let before = network
                        .http_participating(&identity, &cfg.spec, None, false, cfg.transport())
                        .unwrap();
                    let result = catalog::load_json_discovery_with(&cfg, &network).await;
                    if valid {
                        assert_eq!(result.unwrap()["value"], 1);
                    } else {
                        assert_eq!(
                            result.unwrap_err().downcast_ref::<catalog::LoadStage>(),
                            Some(&catalog::LoadStage::Parse)
                        );
                    }
                    let after = network
                        .http_participating(&identity, &cfg.spec, None, false, cfg.transport())
                        .unwrap();
                    assert_eq!(
                        Arc::ptr_eq(&before, &after),
                        !candidate || !valid,
                        "only a complete valid candidate catalog promotes"
                    );
                    let expected = if candidate { 2 } else { 1 };
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        expected,
                        "redirect status never earns another request"
                    );
                    assert_eq!(followed.load(Ordering::SeqCst), 0);
                    assert!(
                        Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes)
                            .unwrap()
                            .read()
                            .await
                            .is_none(),
                        "redirect body must not become a persistent200 catalog"
                    );
                    assert_eq!(
                        socks.records.lock().unwrap().len(),
                        if mode == Mode::Tor { expected } else { 0 }
                    );
                    server.abort();
                }
            }
        }
    }
}

#[tokio::test]
async fn tor_catalog_retries_promote_only_valid_json_or_304_and_stop_on_local_or_content_errors() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let step = Arc::new(AtomicUsize::new(0));
    let step_for_route = step.clone();
    let app = Router::new().route(
        "/spec",
        axum::routing::get(move |req: Request| {
            let step = step_for_route.clone();
            async move {
                match step.fetch_add(1, Ordering::SeqCst) {
                    // Sequence 1: 429 (Retry-After: 1) on selected -> 200 valid JSON on candidate 1 -> promotes candidate 1.
                    0 => (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "1")],
                        "rate limited",
                    )
                        .into_response(),
                    1 => (
                        axum::http::StatusCode::OK,
                        [("cache-control", "max-age=600"), ("etag", "\"v1\"")],
                        "{\"value\":1}",
                    )
                        .into_response(),
                    // Sequence 2 (after expiring disk cache): 503 on promoted scope -> 304 Not Modified on candidate -> promotes candidate.
                    2 => {
                        assert_eq!(req.headers()["if-none-match"], "\"v1\"");
                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "busy").into_response()
                    }
                    3 => {
                        assert_eq!(req.headers()["if-none-match"], "\"v1\"");
                        (
                            axum::http::StatusCode::NOT_MODIFIED,
                            [("cache-control", "max-age=600"), ("etag", "\"v1\"")],
                            "",
                        )
                            .into_response()
                    }
                    // Sequence 3: 403 on selected -> 200 invalid JSON on candidate -> fails at parse without retry or promotion.
                    4 => (axum::http::StatusCode::FORBIDDEN, "exit blocked").into_response(),
                    5 => (axum::http::StatusCode::OK, "not json").into_response(),
                    // Sequence 4: 400 on selected -> 200 oversized body on candidate -> fails at body limit without retry or promotion.
                    6 => (axum::http::StatusCode::BAD_REQUEST, "bad exit").into_response(),
                    7 => (axum::http::StatusCode::OK, "x".repeat(2048)).into_response(),
                    // Sequence 5: 404 (ineligible status) -> fails immediately after 1 attempt.
                    _ => (axum::http::StatusCode::NOT_FOUND, "missing").into_response(),
                }
            }
        }),
    );
    let (address, origin) = crate::test_tls::serve(app).await;
    let url = "https://api.example.com/spec".to_owned();
    let socks = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::None,
    )
    .await;
    let context = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.spec = url.clone();
    cfg.max_spec_bytes = 1024;
    let id = crate::network::IsolationId::discovery(&url).unwrap();

    let initial_selected = context
        .http_participating(&id, &url, None, false, cfg.transport())
        .unwrap();
    let started = std::time::Instant::now();
    let doc = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap();
    assert_eq!(doc["value"], 1);
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(step.load(Ordering::SeqCst), 2);
    let promoted_1 = context
        .http_participating(&id, &url, None, false, cfg.transport())
        .unwrap();
    assert!(!Arc::ptr_eq(&initial_selected, &promoted_1));
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_ne!(records[0].password, records[1].password);
        assert_eq!(
            records[1].password,
            context.credentials(&promoted_1.scope).1
        );
    }

    // Fresh disk cache hit returns without network I/O or transport changes.
    let cached_doc = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap();
    assert_eq!(cached_doc["value"], 1);
    assert_eq!(step.load(Ordering::SeqCst), 2);
    assert_eq!(socks.records.lock().unwrap().len(), 2);
    assert!(Arc::ptr_eq(
        &promoted_1,
        &context
            .http_participating(&id, &url, None, false, cfg.transport())
            .unwrap()
    ));

    // Expire disk cache entry to trigger conditional revalidation: 503 on promoted_1 (reusing pooled HTTP/2 connection) -> 304 on candidate -> promotes candidate.
    let slot = Slot::new(&cfg, &url, "catalog", cfg.max_spec_bytes).unwrap();
    let mut entry = slot.read().await.unwrap();
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    let revalidated = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap();
    assert_eq!(revalidated["value"], 1);
    assert_eq!(step.load(Ordering::SeqCst), 4);
    let promoted_2 = context
        .http_participating(&id, &url, None, false, cfg.transport())
        .unwrap();
    assert!(!Arc::ptr_eq(&promoted_1, &promoted_2));
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[2].password,
            context.credentials(&promoted_2.scope).1
        );
    }

    // Expire disk cache entry; next sequence gets 403 on promoted_2 then 200 invalid JSON on candidate -> fails at Parse without third attempt or promotion.
    let mut entry = slot.read().await.unwrap();
    entry.metadata.expires = 0;
    slot.write(Some(entry)).await;
    let parse_err = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap_err();
    assert_eq!(
        parse_err.downcast_ref::<catalog::LoadStage>(),
        Some(&catalog::LoadStage::Parse)
    );
    assert_eq!(step.load(Ordering::SeqCst), 6);
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert_ne!(
            records[3].password,
            context.credentials(&promoted_2.scope).1
        );
    }
    assert!(Arc::ptr_eq(
        &promoted_2,
        &context
            .http_participating(&id, &url, None, false, cfg.transport())
            .unwrap()
    ));

    // Sequence 4: 400 on promoted_2 then 200 oversized body on candidate -> fails at Body limit without third attempt or promotion.
    let body_err = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap_err();
    assert_eq!(
        body_err.downcast_ref::<catalog::LoadStage>(),
        Some(&catalog::LoadStage::Body)
    );
    assert_eq!(step.load(Ordering::SeqCst), 8);
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 5);
        assert_ne!(
            records[4].password,
            context.credentials(&promoted_2.scope).1
        );
    }
    assert!(Arc::ptr_eq(
        &promoted_2,
        &context
            .http_participating(&id, &url, None, false, cfg.transport())
            .unwrap()
    ));

    // Sequence 5: 404 (ineligible HTTP status) -> fails immediately after 1 attempt without opening any candidate SOCKS connection.
    let status_err = catalog::load_json_discovery_with(&cfg, &context)
        .await
        .unwrap_err();
    assert_eq!(
        status_err
            .downcast_ref::<reqwest::Error>()
            .unwrap()
            .status(),
        Some(reqwest::StatusCode::NOT_FOUND)
    );
    assert_eq!(step.load(Ordering::SeqCst), 9);
    assert_eq!(socks.records.lock().unwrap().len(), 5);

    // Untrusted TLS remains rejected on all three fresh-scope attempts.
    let untrusted_ctx = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(socks.address),
        ..Default::default()
    })
    .unwrap();
    let untrusted_selected = untrusted_ctx
        .http_participating(&id, &url, None, false, cfg.transport())
        .unwrap();
    let error = catalog::load_json_discovery_with(&cfg, &untrusted_ctx)
        .await
        .unwrap_err();
    assert!(crate::network::tls_failure(
        error.downcast_ref::<reqwest::Error>().unwrap()
    ));
    {
        let records = socks.records.lock().unwrap();
        assert_eq!(records.len(), 8);
        let rejected = &records[5..];
        assert_eq!(
            rejected
                .iter()
                .map(|record| &record.password)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            (rejected[0].user.clone(), rejected[0].password.clone()),
            untrusted_ctx.credentials(&untrusted_selected.scope)
        );
    }
    assert_eq!(
        step.load(Ordering::SeqCst),
        9,
        "untrusted TLS never reaches HTTP"
    );
    assert!(
        Arc::ptr_eq(
            &untrusted_selected,
            &untrusted_ctx
                .http_participating(&id, &url, None, false, cfg.transport())
                .unwrap()
        ),
        "TLS exhaustion cannot promote"
    );

    // Local SOCKS auth failure (BadAuth) and closed local SOCKS listener fail after 1 attempt.
    let bad_socks = crate::test_socks::Socks::start(
        std::collections::BTreeMap::from([("api.example.com".into(), address)]),
        crate::test_socks::Fault::BadAuth,
    )
    .await;
    let bad_ctx = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(bad_socks.address),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let err = catalog::load_json_discovery_with(&cfg, &bad_ctx)
        .await
        .unwrap_err();
    assert!(!crate::network::eligible_retry_transport(
        err.downcast_ref::<reqwest::Error>().unwrap()
    ));
    assert_eq!(bad_socks.accepted.load(Ordering::SeqCst), 1);
    assert_eq!(bad_socks.records.lock().unwrap().len(), 0);

    let closed_addr = bad_socks.address;
    drop(bad_socks);
    tokio::task::yield_now().await;
    let closed_ctx = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(closed_addr),
        ..Default::default()
    })
    .unwrap()
    .with_test_root(crate::test_tls::CA);
    let err = catalog::load_json_discovery_with(&cfg, &closed_ctx)
        .await
        .unwrap_err();
    assert!(!crate::network::eligible_retry_transport(
        err.downcast_ref::<reqwest::Error>().unwrap()
    ));

    origin.abort();
}

#[tokio::test]
async fn static_catalog_partial_documents_and_bounds_never_schedule_paid_fallback() {
    use std::sync::atomic::Ordering;
    let (url, origin, task) = fixture().await;
    let (relay, observed, relay_task) =
        crate::discovery_relay::tests::fixture(serde_json::json!({}), false, false).await;
    let network = crate::network::NetworkContext::new(Default::default()).unwrap();
    let cfg = Config {
        spec: url.clone(),
        max_spec_bytes: 1024,
        relay_fallback: Some(relay),
        ..Default::default()
    };
    for mode in [16, 17, 8, 20] {
        origin.lock().unwrap().mode = mode;
        let result = catalog::prepare_json_discovery_with(&cfg, &network).await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("partial/bounded catalog must fail before a relay continuation"),
        };
        assert_eq!(
            error.downcast_ref::<catalog::LoadStage>(),
            Some(&catalog::LoadStage::Body)
        );
        assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
        assert_eq!(observed.signed.load(Ordering::SeqCst), 0);
    }
    assert_eq!(origin.lock().unwrap().calls.len(), 4);
    // Complete malformed remote content retains the authored static fallback rule.
    for mode in [4, 18] {
        origin.lock().unwrap().mode = mode;
        assert!(matches!(
            catalog::prepare_json_discovery_with(&cfg, &network)
                .await
                .unwrap(),
            catalog::CatalogLoad::RelayPending(_)
        ));
    }
    origin.lock().unwrap().mode = 19;
    let document = catalog::prepare_json_discovery_with(&cfg, &network)
        .await
        .unwrap();
    assert!(matches!(document, catalog::CatalogLoad::Ready(value) if value["value"] == 19));
    origin.lock().unwrap().mode = 0;
    assert_eq!(
        catalog::load_json_with_limit(&url, &client(&url), 1024)
            .await
            .unwrap()["value"],
        1
    );
    origin.lock().unwrap().mode = 16;
    assert!(
        catalog::load_json_with_limit(&url, &client(&url), 1024)
            .await
            .is_err()
    );
    assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    assert_eq!(observed.signed.load(Ordering::SeqCst), 0);
    task.abort();
    relay_task.abort();
}

#[tokio::test]
async fn static_catalog_proven_local_proxy_refusal_never_schedules_paid_fallback() {
    use std::sync::atomic::Ordering;
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = socket.local_addr().unwrap();
    drop(socket);
    let network = crate::network::NetworkContext::new(crate::network::NetworkPolicy {
        mode: crate::network::Mode::Tor,
        socks_endpoint: Some(endpoint),
        ..Default::default()
    })
    .unwrap();
    let (relay, observed, relay_task) =
        crate::discovery_relay::tests::fixture(serde_json::json!({}), false, false).await;
    let cfg = Config {
        spec: "https://refusal.example.com/openapi.json".into(),
        relay_fallback: Some(relay),
        ..Default::default()
    };
    let result = catalog::prepare_json_discovery_with(&cfg, &network).await;
    assert!(
        result.is_err(),
        "a permanent local proxy failure cannot schedule relay work"
    );
    assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    assert_eq!(observed.signed.load(Ordering::SeqCst), 0);
    relay_task.abort();
}
