//! Opt-in localhost replay, not a live provider/Tor performance claim.
use axum::{
    Router,
    extract::{OriginalUri, State},
    http::HeaderMap,
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use x402_treazury::{
    catalog::{self, Config},
    deployment::Deployment,
    payment::USDC,
    pricing::PricingCache,
};
const LARGE_ROUTES: usize = 32;

fn synthetic_documents() -> Vec<serde_json::Value> {
    let mut large = serde_json::Map::new();
    for index in 0..LARGE_ROUTES {
        large.insert(
            format!("/lookup-{index}"),
            json!({"get":{"summary":"Lookup"}}),
        );
    }
    vec![
        json!({"openapi":"3.1.0","info":{"title":"Requests","version":"1"},"paths":{
            "/lookup":{"get":{"summary":"Lookup"}},
            "/items/{id}":{"get":{"parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}]}},
            "/echo":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["items"],"properties":{"items":{"type":"array","items":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}}}}}}}}}
        }}),
        json!({"openapi":"3.1.0","info":{"title":"Large","version":"1"},"paths":large}),
        json!({"openapi":"3.1.0","info":{"title":"Pricing and methods","version":"1"},"paths":{
            "/quoted":{"get":{"x-payment-info":{"price":{"amount":"0.002","currency":"USD"}}}},
            "/twins":{"get":{"summary":"Query variant"},"post":{"summary":"JSON variant","requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"query":{"type":"string"}}}}}}}},
            "/probe":{"get":{"summary":"Unsigned probe"}}
        }}),
    ]
}

#[derive(Clone)]
struct Replay {
    specs: Arc<Vec<Vec<u8>>>,
    catalogs: Arc<AtomicUsize>,
    probes: Arc<AtomicUsize>,
}
async fn response(
    State(state): State<Replay>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> axum::response::Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("x-payment"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(id) = uri.path().strip_prefix("/spec/") {
        state.catalogs.fetch_add(1, Ordering::SeqCst);
        return state.specs[id.parse::<usize>().unwrap()]
            .clone()
            .into_response();
    }
    state.probes.fetch_add(1, Ordering::SeqCst);
    let challenge = json!({"x402Version":2,"accepts":[{"scheme":"exact","network":"eip155:8453","asset":USDC,"amount":"1000"}]});
    (
        axum::http::StatusCode::PAYMENT_REQUIRED,
        [(
            "payment-required",
            STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
        )],
    )
        .into_response()
}
#[tokio::test]
#[ignore = "explicit localhost performance measurement; no mainnet requests or signing"]
async fn representative_startup_replay() {
    let inputs: Vec<_> = synthetic_documents()
        .into_iter()
        .map(|document| {
            let settings = Config {
                pricing_key: Some("x-payment-info".into()),
                ..Config::default()
            };
            (settings, document)
        })
        .collect();
    let specs: Vec<_> = inputs
        .iter()
        .map(|(_, document)| serde_json::to_vec(document).unwrap())
        .collect();
    let unique_specs = specs.len();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let replay = Replay {
        specs: Arc::new(specs),
        catalogs: Arc::default(),
        probes: Arc::default(),
    };
    let state = replay.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(response).with_state(state))
            .await
            .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    // Two aliases share synthetic spec URLs but retain distinct tool prefixes.
    let selected: Vec<usize> = (0..unique_specs).chain([0, 1]).collect();
    let mut baseline = None;
    for limit in [1, 4, 8, 16] {
        let mut config = format!(
            "version=1\n[startup]\ncatalog_concurrency={limit}\n[wallets.w]\nmode='static'\nprivate_key_env='UNUSED'\nmax_api_payment_usdc='0.01'\n"
        );
        let mut names = Vec::new();
        for (alias, index) in selected.iter().enumerate() {
            let name = format!("p{alias:02}");
            names.push(name.clone());
            config += &format!(
                "[sources.{name}]\nspec='{base}/spec/{index}'\nbase_url='{base}/api/{index}'\nprefix='{name}'\npricing_key='x-payment-info'\nprobe_pricing=false\n"
            );
        }
        config += &format!(
            "[servers.test]\nlisten='127.0.0.1:0'\nbearer_token_env='UNUSED'\nwallet='w'\nsources={}\n",
            serde_json::to_string(&names).unwrap()
        );
        let path = dir.path().join("bench.toml");
        std::fs::write(&path, config).unwrap();
        for sample in 0..3 {
            replay.catalogs.store(0, Ordering::SeqCst);
            let start = Instant::now();
            let deployment = Deployment::load(&path).await.unwrap();
            let elapsed = start.elapsed();
            assert_eq!(replay.catalogs.load(Ordering::SeqCst), unique_specs);
            let inventory = serde_json::to_value(deployment.inventory()).unwrap();
            if let Some(expected) = &baseline {
                assert_eq!(expected, &inventory);
            } else {
                baseline = Some(inventory);
            }
            println!(
                "{}",
                json!({"stage":"catalogs","concurrency":limit,"sample":sample,"elapsed_ms":elapsed.as_millis(),"fetches":replay.catalogs.load(Ordering::SeqCst),"source_count":selected.len(),"unique_specs":unique_specs})
            );
        }
    }
    use futures_util::{StreamExt, stream};
    for concurrency in [1, 16] {
        let cache = PricingCache::default();
        replay.probes.store(0, Ordering::SeqCst);
        let start = Instant::now();
        let mut jobs = Vec::new();
        for index in &selected {
            let cache = &cache;
            let inputs = &inputs;
            let base = &base;
            jobs.push(async move {
                let (settings, doc) = &inputs[*index];
                let endpoint = format!("{base}/api/{index}");
                let mut settings = settings.clone();
                settings.base_url = Some(endpoint.clone());
                let tools = catalog::build_tools(
                    &settings,
                    doc,
                    settings.prefix.as_deref().unwrap_or("api"),
                )
                .unwrap();
                // Check every possible probe destination before running the production discovery function.
                for tool in tools
                    .iter()
                    .filter(|t| t.method == "GET" && t.help_url.is_none() && !t.path.contains('{'))
                {
                    assert!(
                        tool.route(&endpoint, &serde_json::Map::new())
                            .unwrap()
                            .url
                            .starts_with(&format!("{base}/")),
                        "non-loopback probe refused"
                    );
                }
                cache
                    .discover(&settings, doc, &tools, &endpoint)
                    .await
                    .unwrap();
            });
        }
        let mut pending = stream::iter(jobs).buffer_unordered(concurrency);
        while pending.next().await.is_some() {}
        // One small lookup, the large inventory, and one unpriced GET.
        // Embedded prices, templated GETs and the POST-preferred twin need no probe.
        assert_eq!(replay.probes.load(Ordering::SeqCst), LARGE_ROUTES + 2);
        println!(
            "{}",
            json!({"stage":"pricing_total", "source_concurrency":concurrency,
            "elapsed_ms":start.elapsed().as_millis(), "requests":replay.probes.load(Ordering::SeqCst),
            "delay_per_request_ms":100})
        );
    }
    server.abort();
}
