use serde_json::json;
use x402_treazury::catalog::{Config, build_tools};

#[test]
fn advertised_prices_preserve_provenance_even_when_amounts_appear_in_prose() {
    let root = json!({"operations":[
        {"method":"GET","path":"/fixed","description":"A $0.15 example, with $0.10 mentioned elsewhere.","pricing":{"amount":"0.10","currency":"USD"}},
        {"method":"GET","path":"/range","pricing":{"min":"0.10","max":"2.00","currency":"USD"}},
        {"method":"GET","path":"/dynamic","pricing":{"amount":"0.10","currency":"USD","mode":"dynamic"}},
        {"method":"GET","path":"/free","pricing":{"authMode":"free"}},
        {"method":"GET","path":"/unit","pricing":{"amount":"0.01","currency":"USD","unit":"token"}},
        {"method":"GET","path":"/unknown"}
    ]});
    let tools = build_tools(&Config::default(), &root, "api").unwrap();
    let desc = |path: &str| {
        tools
            .iter()
            .find(|t| t.path == path)
            .unwrap()
            .description
            .as_str()
    };
    assert!(desc("/fixed").contains("Cost: ~$0.10/call [spec]."));
    assert!(desc("/fixed").contains("[spec]"));
    assert!(desc("/range").contains("Cost: ~$0.10–$2.00/call [spec]."));
    assert!(desc("/dynamic").contains("[spec, dynamic]"));
    assert!(desc("/free").contains("Cost: free [spec]."));
    assert!(desc("/unit").contains("Cost: ~$0.01/token [spec]."));
    assert!(desc("/unknown").contains("Cost: unknown."));
    assert!(!desc("/unknown").contains("X402_MAX_PRICE_USD"));
}

#[test]
fn retired_probe_settings_reject_instead_of_silently_disabling_requests() {
    for (key, value) in [
        ("probe_pricing", "false"),
        ("probe_ttl_seconds", "60"),
        ("probe_concurrency", "2"),
        ("probe_max_endpoints", "0"),
        ("probe_methods", "['GET']"),
        ("probe_timeout", "3"),
    ] {
        let error = toml::from_str::<Config>(&format!("spec='spec.json'\n{key}={value}\n"))
            .err()
            .unwrap();
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}
