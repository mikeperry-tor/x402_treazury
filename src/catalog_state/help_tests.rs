//! Complete help and financial-cache ownership through the real bound-tool path.
use super::*;
use crate::{
    catalog::Config,
    payment::{Payer, SpendPolicy},
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};

fn tool(url: &str) -> ToolSpec {
    crate::catalog::build_tools(
        &Config {
            help_url: Some(url.into()),
            ..Default::default()
        },
        &json!({"paths":{"/unused":{"get":{}}}}),
        "fixture",
    )
    .unwrap()
    .pop()
    .unwrap()
}
fn bound(url: &str, client: PaidClient) -> BoundTool {
    bind(vec![(tool(url), client, String::new())])
        .pop()
        .unwrap()
}
fn payer(key: u64) -> Payer {
    Payer::new(
        &format!("{key:064x}"),
        SpendPolicy::dollars("0.01").unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn complete_empty_and_utf8_help_are_cached_but_partial_or_invalid_text_is_not() {
    let calls = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let app = axum::Router::new().route(
        "/{kind}",
        get({
            let calls = calls.clone();
            move |Path(kind): Path<String>| {
                let calls = calls.clone();
                async move {
                    *calls.lock().unwrap().entry(kind.clone()).or_default() += 1;
                    match kind.as_str() {
                        "partial-status" => {
                            (StatusCode::PARTIAL_CONTENT, "valid text fragment").into_response()
                        }
                        "partial-header" => {
                            ([("content-range", "bytes 0-10/100")], "valid text fragment")
                                .into_response()
                        }
                        "invalid-utf8" => (StatusCode::OK, vec![0xff]).into_response(),
                        "empty" => StatusCode::NO_CONTENT.into_response(),
                        _ => "complete café\n".into_response(),
                    }
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for kind in [
        "partial-status",
        "partial-header",
        "invalid-utf8",
        "empty",
        "complete",
    ] {
        let help = bound(&format!("{base}/{kind}"), PaidClient::unsigned());
        for _ in 0..2 {
            let result = help.invoke(&Default::default()).await;
            match kind {
                "empty" => assert_eq!(result.unwrap(), ""),
                "complete" => assert_eq!(result.unwrap(), "complete café\n"),
                _ => assert!(result.is_err()),
            }
        }
        let complete = matches!(kind, "empty" | "complete");
        assert_eq!(calls.lock().unwrap()[kind], if complete { 1 } else { 2 });
        assert_eq!(
            help.help.get().is_some(),
            complete,
            "partial and unsupported text never initialize help"
        );
    }
    server.abort();
}

#[tokio::test]
async fn authority_snapshot_does_not_freeze_ordinary_payer_replacement() {
    let client = PaidClient::new(payer(111));
    let original = HelpBinding::new(client.clone());
    let same = HelpBinding::new(client.clone());
    assert!(original.compatible(&same));
    let original_identity = original.client.documentation_identity().await.unwrap();
    client.replace_payer(payer(112));
    assert_ne!(
        client.documentation_identity().await.unwrap(),
        original_identity
    );
    assert_eq!(
        original.client.documentation_identity().await.unwrap(),
        original_identity
    );
    assert!(!original.compatible(&HelpBinding::new(client.clone())));
    assert!(
        !original.compatible(&HelpBinding::new(PaidClient::new(payer(111)))),
        "an independently bound authority must not share paid initialization"
    );
    let public = client.clone().public_destinations();
    assert!(!HelpBinding::new(client).compatible(&HelpBinding::new(public.clone())));
    assert!(
        HelpBinding::with_origins(
            public.clone(),
            vec![
                "https://a.example.com".into(),
                "https://b.example.com".into()
            ]
        )
        .compatible(&HelpBinding::with_origins(
            public.clone(),
            vec![
                "https://b.example.com".into(),
                "https://a.example.com".into()
            ]
        ))
    );
    assert!(
        !HelpBinding::with_origins(public.clone(), vec!["https://a.example.com".into()])
            .compatible(&HelpBinding::with_origins(
                public,
                vec!["https://b.example.com".into()]
            ))
    );
}

#[test]
fn help_relay_cache_and_cancelled_waiter_retain_exact_authority_in_isolated_process() {
    const CHILD: &str = "TREAZURY_HELP_RELAY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(paid_fixture());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "catalog_state::help_tests::help_relay_cache_and_cancelled_waiter_retain_exact_authority_in_isolated_process", "--nocapture"])
        .env(CHILD, "1").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Clone, Default)]
struct PaidFixture {
    calls: Arc<Mutex<Vec<(String, String)>>>,
    signers: Arc<Mutex<Vec<String>>>,
    held: tokio_util::sync::CancellationToken,
    release: tokio_util::sync::CancellationToken,
}
async fn origin(
    State(state): State<PaidFixture>,
    Path(kind): Path<String>,
    headers: HeaderMap,
) -> Response {
    assert!(!headers.contains_key("payment-signature"));
    assert!(!headers.contains_key("authorization"));
    state
        .calls
        .lock()
        .unwrap()
        .push((kind.clone(), "origin".into()));
    if matches!(kind.as_str(), "healthy" | "healthy-metadata-off") {
        "independent origin".into_response()
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}
async fn relay(
    State(state): State<PaidFixture>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["method"], "GET");
    let target = request["url"].as_str().unwrap();
    let kind = target.rsplit('/').next().unwrap();
    let Some(signature) = headers.get("payment-signature") else {
        state
            .calls
            .lock()
            .unwrap()
            .push((kind.into(), "challenge".into()));
        let challenge = json!({"x402Version":2,"resource":{"url":"https://spec.example.com/relay"},"accepts":[{
            "scheme":"exact","network":"eip155:8453","asset":crate::payment::USDC,
            "amount":"1","payTo":"0x0000000000000000000000000000000000000003",
            "maxTimeoutSeconds":60,"extra":{"name":"USD Coin","version":"2"}
        }]});
        return (
            StatusCode::PAYMENT_REQUIRED,
            [(
                "payment-required",
                STANDARD.encode(serde_json::to_vec(&challenge).unwrap()),
            )],
            "",
        )
            .into_response();
    };
    let payment: Value =
        serde_json::from_slice(&STANDARD.decode(signature.as_bytes()).unwrap()).unwrap();
    crate::test_signatures::recover_exact(&payment);
    state.signers.lock().unwrap().push(
        payment["payload"]["authorization"]["from"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase(),
    );
    state
        .calls
        .lock()
        .unwrap()
        .push((kind.into(), "signed".into()));
    if kind == "held" {
        state.held.cancel();
        state.release.cancelled().await;
    }
    if kind.starts_with("uncertain") {
        return (StatusCode::BAD_GATEWAY, "PRIVATE_UPSTREAM_PROSE").into_response();
    }
    axum::Json(json!({"url":target,"method":"GET","statusCode":if kind == "partial" {206} else {200},
        "headers":{"content-type":"text/plain"},"body":"complete relay help\n","redirectChain":[],"error":null})).into_response()
}
fn sequence(state: &PaidFixture, kind: &str) -> Vec<String> {
    state
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(key, _)| key == kind)
        .map(|(_, step)| step.clone())
        .collect()
}
async fn paid_fixture() {
    use crate::{
        discovery_relay::SearchRelay,
        network::{Mode, NetworkContext, NetworkPolicy},
    };
    let fixture = PaidFixture::default();
    let app = axum::Router::new()
        .route("/docs/{kind}", get(origin))
        .route("/relay", post(relay))
        .with_state(fixture.clone());
    let (address, server) = crate::test_tls::serve(app).await;
    let proxy = crate::test_socks::Socks::start(
        BTreeMap::from([
            ("api.example.com".into(), address),
            ("spec.example.com".into(), address),
        ]),
        crate::test_socks::Fault::None,
    )
    .await;
    crate::network::install_test_context(
        NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(proxy.address),
            tor_isolation_retries: Some(0),
            cover_traffic_enabled: Some(false),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA),
    );
    let client = PaidClient::new(payer(113)).with_document_relay(Some(Arc::new(
        SearchRelay::fixture("https://spec.example.com/relay".into(), Config::default()),
    )));
    let held = bound("https://api.example.com/docs/held", client.clone());
    let expected = held.help.client.documentation_identity().await.unwrap();
    client.replace_payer(payer(114));
    assert_ne!(
        held.client.documentation_identity().await.unwrap(),
        expected
    );
    let first = tokio::spawn({
        let held = held.clone();
        async move { held.invoke(&Default::default()).await }
    });
    tokio::time::timeout(Duration::from_secs(5), fixture.held.cancelled())
        .await
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let second = tokio::spawn({
        let held = held.clone();
        async move { held.invoke(&Default::default()).await }
    });
    let healthy = bound("https://api.example.com/docs/healthy", client.clone());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), healthy.invoke(&Default::default()))
            .await
            .unwrap()
            .unwrap(),
        "independent origin"
    );
    fixture.release.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "complete relay help\n"
    );
    assert_eq!(
        held.invoke(&Default::default()).await.unwrap(),
        "complete relay help\n"
    );
    assert_eq!(
        sequence(&fixture, "held"),
        ["origin", "challenge", "signed"],
        "cancelled and compatible waiters share one retained paid acquisition"
    );
    let signer: alloy_signer_local::PrivateKeySigner = format!("{:064x}", 113).parse().unwrap();
    assert_eq!(
        fixture.signers.lock().unwrap()[0],
        signer.address().to_string().to_ascii_lowercase(),
        "the exact help-binding payer snapshot signed the relay request"
    );
    // Help uses metadata permission even through the bound tool's API wrapper.
    // Reusing both client and URL for an ordinary call cannot reuse that grant.
    let url = "https://api.example.com/docs/api-default";
    let help = bound(url, client.clone());
    assert_eq!(
        help.invoke(&Default::default()).await.unwrap(),
        "complete relay help\n"
    );
    let mut ordinary = help.clone();
    ordinary.tool.help_url = None;
    ordinary.tool.path = url.into();
    assert!(ordinary.invoke(&Default::default()).await.is_err());
    assert_eq!(
        sequence(&fixture, "api-default"),
        ["origin", "challenge", "signed", "origin"]
    );
    assert_eq!(
        help.invoke(&Default::default()).await.unwrap(),
        "complete relay help\n"
    );
    assert_eq!(
        sequence(&fixture, "api-default").len(),
        4,
        "help's metadata cache remains usable independently"
    );

    let metadata_off = client.clone().with_document_relay(Some(Arc::new(
        SearchRelay::fixture("https://spec.example.com/relay".into(), Config::default())
            .with_policy(crate::discovery_relay::Policy {
                provider_metadata: false,
                api_requests: true,
                ..Default::default()
            }),
    )));
    let url = "https://api.example.com/docs/metadata-off";
    let help = bound(url, metadata_off.clone());
    assert!(help.invoke(&Default::default()).await.is_err());
    assert_eq!(
        sequence(&fixture, "metadata-off"),
        ["origin"],
        "API permission cannot pay for help"
    );
    let mut ordinary = help.clone();
    ordinary.tool.help_url = None;
    ordinary.tool.path = url.into();
    let output = ordinary.invoke_output(&Default::default()).await.unwrap();
    assert_eq!(output.text, "complete relay help\n");
    assert!(output.payment_may_have_occurred);
    assert_eq!(
        output.advisories,
        ["This call may have charged; do not automatically repeat this logical call."]
    );
    assert_eq!(
        sequence(&fixture, "metadata-off"),
        ["origin", "origin", "challenge", "signed"]
    );
    assert_eq!(
        bound(
            "https://api.example.com/docs/healthy-metadata-off",
            metadata_off
        )
        .invoke(&Default::default())
        .await
        .unwrap(),
        "independent origin"
    );
    assert_eq!(sequence(&fixture, "healthy-metadata-off"), ["origin"]);
    for kind in ["uncertain", "partial"] {
        let help = bound(
            &format!("https://api.example.com/docs/{kind}"),
            client.clone(),
        );
        for _ in 0..2 {
            let error = help.invoke(&Default::default()).await.unwrap_err();
            let failure = SharedFailure::capture(&error, "help unavailable");
            assert!(failure.possible_payment(), "{error:#}");
            assert!(format!("{error:#}").contains("do not automatically repeat"));
            assert!(!format!("{error:#}").contains("PRIVATE_UPSTREAM_PROSE"));
        }
        assert!(
            help.help
                .get()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .possible_payment()
        );
        assert_eq!(
            sequence(&fixture, kind),
            ["origin", "challenge", "signed"],
            "a possible-paid failure remains sticky and cannot buy a retry"
        );
    }
    assert_eq!(sequence(&fixture, "healthy"), ["origin"]);
    // Reject only the completion observation, after normal challenge/signing
    // and document validation have already run. Use the existing qualification
    // registry worker rather than a synthetic production failure hook.
    for (kind, failed_delivery) in [
        ("qualification-success", false),
        ("uncertain-qualification", true),
    ] {
        let (_dir, claim, registry) = crate::qualification::managed_claim_fixture().await;
        let db = rusqlite::Connection::open(registry).unwrap();
        db.execute_batch("CREATE TRIGGER reject_help_completion BEFORE INSERT ON events
            WHEN NEW.kind='application_help' AND json_extract(NEW.detail,'$.result.status') != 'started'
            BEGIN SELECT RAISE(ABORT,'forced help completion observation failure'); END;").unwrap();
        let help = bound(
            &format!("https://api.example.com/docs/{kind}"),
            client.clone(),
        );
        let error = claim
            .invoke(help.invoke(&Default::default()))
            .await
            .unwrap_err();
        let failure = SharedFailure::capture(&error, "help unavailable");
        assert!(
            failure.possible_payment(),
            "qualification failure after paid help retains fee context: {error:#}"
        );
        if failed_delivery {
            assert_eq!(
                failure.status(),
                Some(502),
                "the original paid delivery failure takes precedence over a secondary qualification error"
            );
            assert!(!format!("{error:#}").contains("forced help completion"));
        } else {
            assert!(format!("{error:#}").contains("forced help completion"));
        }
        assert!(help.help.result.get().unwrap().possible);
        assert_eq!(help.help.get().unwrap().is_err(), failed_delivery);
        assert_eq!(sequence(&fixture, kind), ["origin", "challenge", "signed"]);
        db.execute_batch("DROP TRIGGER reject_help_completion;")
            .unwrap();
        let cached = help.invoke(&Default::default()).await;
        assert_eq!(cached.is_err(), failed_delivery);
        assert_eq!(
            sequence(&fixture, kind),
            ["origin", "challenge", "signed"],
            "qualification completion failure cannot purchase another help read"
        );
    }
    for (kind, failed_delivery) in [("start-success", false), ("uncertain-start", true)] {
        let help = bound(
            &format!("https://api.example.com/docs/{kind}"),
            client.clone(),
        );
        assert_eq!(
            help.invoke(&Default::default()).await.is_err(),
            failed_delivery
        );
        let (_dir, claim, registry) = crate::qualification::managed_claim_fixture().await;
        let db = rusqlite::Connection::open(registry).unwrap();
        db.execute_batch("CREATE TRIGGER reject_help_start BEFORE INSERT ON events
            WHEN NEW.kind='application_help' AND json_extract(NEW.detail,'$.result.status') = 'started'
            BEGIN SELECT RAISE(ABORT,'forced help start observation failure'); END;").unwrap();
        let error = claim
            .invoke(help.invoke(&Default::default()))
            .await
            .unwrap_err();
        let failure = SharedFailure::capture(&error, "help unavailable");
        assert!(
            failure.possible_payment(),
            "start observation preserves cached fee evidence: {error:#}"
        );
        if failed_delivery {
            assert_eq!(failure.status(), Some(502));
            assert!(
                !format!("{error:#}").contains("forced help start"),
                "a secondary observation cannot replace the original paid failure"
            );
        } else {
            assert!(format!("{error:#}").contains("forced help start"));
        }
        assert_eq!(sequence(&fixture, kind), ["origin", "challenge", "signed"]);
        db.execute_batch("DROP TRIGGER reject_help_start;").unwrap();
        assert_eq!(
            help.invoke(&Default::default()).await.is_err(),
            failed_delivery
        );
        assert_eq!(
            sequence(&fixture, kind),
            ["origin", "challenge", "signed"],
            "a failed start observation cannot purchase another help read"
        );
    }
    client.drain().await;
    server.abort();
}
