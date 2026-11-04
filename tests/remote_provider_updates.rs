//! Synthetic contracts exercise authored policies without freezing vendor inventories.
use serde_json::{Value, json};
use std::path::Path;
use x402_treazury::{catalog, config};

async fn provider(path: &str) -> catalog::Config {
    let mut cfg = config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .await
        .unwrap()
        .settings;
    // These small contracts test compatibility and authored guidance, not advice coverage.
    cfg.recommended_tools.clear();
    cfg.help_url = None;
    cfg
}

fn tools(cfg: &catalog::Config, spec: &Value) -> Vec<catalog::ToolSpec> {
    catalog::build_tools(cfg, spec, cfg.prefix.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn strale_exact_owned_operations_preserve_public_transaction_and_job_reads() {
    let cfg = provider("providers/strale.toml").await;
    let spec = json!({"openapi":"3.1.0","paths":{
        "/v1/jobs/webhook-secret":{"get":{}},
        "/v1/jobs/public-capabilities":{"get":{}},
        "/v1/transactions":{"get":{}},
        "/v1/transactions/{id}":{"delete":{},"get":{}},
        "/v1/transactions/{id}/audit-token":{"post":{}},
        "/v1/transactions/{id}/dispute":{"get":{},"post":{}},
        "/v1/transactions/{id}/verify":{"get":{}},
        "/v1/transactions/{id}/public-evidence":{"get":{}},
        "/v1/solutions/{slug}/execute":{"post":{}},
        "/v1/solutions/{slug}/public-description":{"get":{}},
        "/v1/web3-assurance":{"post":{}}
    }});
    let selected = tools(&cfg, &spec);
    for (method, path) in [
        ("GET", "/v1/jobs/webhook-secret"),
        ("GET", "/v1/transactions"),
        ("DELETE", "/v1/transactions/{id}"),
        ("POST", "/v1/transactions/{id}/audit-token"),
        ("GET", "/v1/transactions/{id}/dispute"),
        ("POST", "/v1/transactions/{id}/dispute"),
        ("POST", "/v1/solutions/{slug}/execute"),
        ("POST", "/v1/web3-assurance"),
    ] {
        assert!(
            !selected
                .iter()
                .any(|t| t.method == method && t.path == path)
        );
    }
    for path in [
        "/v1/jobs/public-capabilities",
        "/v1/transactions/{id}/verify",
        "/v1/transactions/{id}/public-evidence",
        "/v1/solutions/{slug}/public-description",
    ] {
        assert!(selected.iter().any(|t| t.method == "GET" && t.path == path));
    }
}

#[tokio::test]
async fn relayshield_tap_binding_exclusions_preserve_unrelated_public_reads() {
    let cfg = provider("providers/relayshield.toml").await;
    let spec = json!({"openapi":"3.1.0","paths":{
        "/v1/tap/intent-bind":{"post":{},"get":{}},
        "/v1/tap/settle-check":{"post":{}},
        "/v1/tap/public-proof":{"get":{}},
        "/v1/email-check":{"post":{}},
        "/v1/payg/breach":{"post":{}}
    }});
    let selected = tools(&cfg, &spec);
    for path in ["/v1/tap/intent-bind", "/v1/tap/settle-check"] {
        assert!(
            !selected
                .iter()
                .any(|t| t.method == "POST" && t.path == path)
        );
    }
    // Excluding the owned POST does not hide a separately documented future GET twin.
    assert!(
        selected
            .iter()
            .any(|t| t.method == "GET" && t.path == "/v1/tap/intent-bind")
    );
    for path in ["/v1/tap/public-proof", "/v1/email-check", "/v1/payg/breach"] {
        assert!(selected.iter().any(|t| t.path == path));
    }
}

#[tokio::test]
async fn vaaya_fundable_lookup_and_search_retain_their_distinct_tariffs() {
    let cfg = provider("providers/vaaya.toml").await;
    let spec = json!({"openapi":"3.1.0","paths":{
        "/api/run/fundable/company":{"post":{"x-payment-info":{"price":{"amount":"0.10","currency":"USD"}}}},
        "/api/run/fundable/company-search":{"post":{"x-payment-info":{"price":{"amount":"0.01","currency":"USD"}}}}
    }});
    let selected = tools(&cfg, &spec);
    let lookup = selected
        .iter()
        .find(|t| t.path == "/api/run/fundable/company")
        .unwrap();
    let search = selected
        .iter()
        .find(|t| t.path == "/api/run/fundable/company-search")
        .unwrap();
    assert!(lookup.description.contains("$0.10/call"));
    assert!(!lookup.description.contains("$0.01/call"));
    assert!(search.description.contains("$0.01/call"));
    assert!(lookup.description.contains("Verify identity"));
}

#[tokio::test]
async fn shopsocial_reel_guidance_preserves_null_views_and_specific_uncharged_failure() {
    let cfg = provider("providers/shopsocial.toml").await;
    // This route's published quirk differs from full reel-tab or single-reel retrieval.
    let spec = json!({"openapi":"3.1.0","paths":{
        "/api/social/instagram-reel":{"get":{
            "parameters":[{"name":"username","in":"query","required":true,"schema":{"type":"string"}}],
            "x-payment-info":{"price":{"amount":"0.005","currency":"USD"}}
        }}
    }});
    let selected = tools(&cfg, &spec);
    let reel = &selected[0];
    assert!(reel.description.contains("View counts are null"));
    assert!(reel.description.contains("12 newest grid posts"));
    assert!(reel.description.contains("not charged") && reel.description.contains("503"));
    assert!(reel.description.contains("$0.005/call"));
    let request = reel
        .route(
            cfg.base_url.as_deref().unwrap(),
            json!({"username":"nasa"}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.query["username"], "nasa");
    assert!(request.body.is_none());
}

#[tokio::test]
async fn exa_mode_guidance_keeps_lite_advisory_and_preserves_explicit_request_type() {
    let cfg = provider("providers/exa.toml").await;
    let spec = json!({"openapi":"3.1.0","paths":{
        "/search":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","properties":{"query":{"type":"string"},"type":{"type":"string","default":"auto","enum":["auto","lite","deep-lite","deep"]}},"required":["query"]
        }}}}}},
        "/contents":{"post":{}}
    }});
    let selected = tools(&cfg, &spec);
    let search = selected.iter().find(|t| t.path == "/search").unwrap();
    for tariff in [
        "lite/instant $0.004",
        "auto/fast $0.007",
        "deep-lite/deep $0.012",
        "deep-reasoning $0.015",
    ] {
        assert!(search.description.contains(tariff));
    }
    assert!(
        search.input_schema["properties"]["type"]["description"]
            .as_str()
            .unwrap()
            .contains("without live fetching")
    );
    assert_eq!(search.input_schema["properties"]["type"]["default"], "auto");
    for args in [
        json!({"query":"public source records","type":"lite"}),
        json!({"query":"public source records","type":"deep"}),
        json!({"query":"public source records"}),
    ] {
        let request = search
            .route(cfg.base_url.as_deref().unwrap(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.body, Some(args));
        assert!(request.query.is_empty());
    }
}

#[tokio::test]
async fn shopsocial_logged_out_contracts_preserve_null_fields_and_uncharged_failures() {
    let cfg = provider("providers/shopsocial.toml").await;
    let spec = json!({"openapi":"3.1.0","paths":{
        "/api/social/instagram-contact":{"get":{}},
        "/api/social/instagram-profile":{"get":{}},
        "/api/social/youtube-transcript":{"get":{}}
    }});
    let selected = tools(&cfg, &spec);
    assert_eq!(selected.len(), 3);
    for tool in &selected {
        assert!(tool.description.contains("503") && tool.description.contains("not charged"));
        assert!(tool.description.contains("$0.002/call"));
    }
    let contact = selected
        .iter()
        .find(|t| t.path.ends_with("instagram-contact"))
        .unwrap();
    assert!(
        contact
            .description
            .contains("bio links and email/phone-like bio text")
    );
    assert!(
        contact
            .description
            .contains("Business email, phone and address buttons are null")
    );
    let profile = selected
        .iter()
        .find(|t| t.path.ends_with("instagram-profile"))
        .unwrap();
    assert!(
        profile
            .description
            .contains("Business category and reel/highlight counts are null")
    );
}
