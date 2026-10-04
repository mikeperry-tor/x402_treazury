//! Opt-in localhost replay, not a live provider/Tor performance claim.
use axum::{
    Router,
    extract::{OriginalUri, State},
    response::IntoResponse,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use x402_treazury::{
    catalog, config, deployment::Deployment, payment::USDC, pricing::PricingCache,
};
#[derive(Deserialize)]
struct Case {
    name: String,
    provider: String,
    spec: Option<String>,
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
) -> axum::response::Response {
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/catalogs/cases.json")).unwrap();
    let mut inputs = Vec::new();
    let mut specs = Vec::new();
    for case in &cases {
        let settings = config::load(&root.join(&case.provider))
            .await
            .unwrap()
            .settings;
        let path = case
            .spec
            .as_ref()
            .map(|p| root.join(p))
            .unwrap_or_else(|| settings.spec.clone().into());
        let bytes = std::fs::read(path).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        specs.push(bytes);
        inputs.push((settings, doc));
    }
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
    // Two aliases share original spec URLs and provider policy, but use distinct tool prefixes.
    let selected: Vec<usize> = (0..cases.len()).chain([0, 1]).collect();
    let mut baseline = None;
    for limit in [1, 4, 8, 16] {
        let mut config = format!(
            "version=1\n[startup]\ncatalog_concurrency={limit}\n[wallets.w]\nmode='static'\nprivate_key_env='UNUSED'\nmax_price_usd='0.01'\n"
        );
        let mut names = Vec::new();
        for (alias, index) in selected.iter().enumerate() {
            let name = format!("p{alias:02}");
            names.push(name.clone());
            config += &format!(
                "[sources.{name}]\nextends={}\nspec='{base}/spec/{index}'\nbase_url='{base}/api/{index}'\nprefix='{name}'\n",
                toml::Value::String(
                    root.join(&cases[*index].provider)
                        .to_string_lossy()
                        .into_owned()
                )
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
            let inventory = serde_json::to_value(deployment.inventory()).unwrap();
            if let Some(expected) = &baseline {
                assert_eq!(expected, &inventory);
            } else {
                baseline = Some(inventory);
            }
            println!(
                "{}",
                json!({"stage":"catalogs","concurrency":limit,"sample":sample,"elapsed_ms":elapsed.as_millis(),"fetches":replay.catalogs.load(Ordering::SeqCst),"source_count":selected.len(),"unique_specs":cases.len()})
            );
        }
    }
    let cache = PricingCache::default();
    let start = Instant::now();
    for index in &selected {
        let (settings, doc) = &inputs[*index];
        let endpoint = format!("{base}/api/{index}");
        let mut settings = settings.clone();
        settings.base_url = Some(endpoint.clone());
        let tools =
            catalog::build_tools(&settings, doc, settings.prefix.as_deref().unwrap_or("api"))
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
        let before = replay.probes.load(Ordering::SeqCst);
        let source_started = Instant::now();
        cache
            .discover(&settings, doc, &tools, &endpoint)
            .await
            .unwrap();
        println!(
            "{}",
            json!({"stage":"pricing_source","provider":cases[*index].name,"enabled":settings.probe_pricing,"requests":replay.probes.load(Ordering::SeqCst)-before,"elapsed_ms":source_started.elapsed().as_millis()})
        );
    }
    println!(
        "{}",
        json!({"stage":"pricing_total","elapsed_ms":start.elapsed().as_millis(),"requests":replay.probes.load(Ordering::SeqCst),"delay_per_request_ms":100})
    );
    server.abort();
}
