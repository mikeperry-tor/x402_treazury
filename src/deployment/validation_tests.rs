//! Characterize orchestration independently of runtime listeners and credentials.
use super::MetaConfig;
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({"version":1,
        "sources":{"a":{"wallet":"payer"},"b":{}},
        "wallets":{"payer":{"mode":"static","private_key_env":"ABSENT_TEST_KEY","max_api_payment_usdc":"1"}},
        "servers":{"one":{"listen":"127.0.0.1:0","bearer_token_env":"ABSENT_TEST_TOKEN","wallet":"payer","sources":["a","b"]}}})
}
fn config(value: &Value) -> MetaConfig {
    serde_json::from_value(value.clone()).unwrap()
}
fn error(value: &Value, expected: &str) {
    let actual = config(value).validate().err().expect("must reject");
    assert_eq!(format!("{actual:#}"), expected);
}
fn resolved(value: &Value) -> Value {
    let r = config(value).validate().unwrap();
    json!({"wallets":r.wallets,"bindings":r.bindings,"generated":r.generated,"summary":r.summary})
}
fn managed() -> Value {
    json!({"mode":"zcash_rotation","reserves":{"small":{"funding_amount_usdc":"5"}},
        "max_api_payment_usdc":"1","max_retirement_remainder_usdc":"0",
        "max_funding_spend_zec":null,"max_conversion_overhead_percent":5,"max_attempts":3})
}
fn treasury() -> Value {
    json!({"state_dir":"never-opened-state","confirmations":1})
}

#[test]
fn preliminary_stages_precede_structural_validation() {
    let mut v = fixture();
    v["startup"] = json!({"catalog_concurrency":0});
    v["network"] = json!({"connect_timeout_seconds":0});
    v["relay_fallback"] = json!({"relays":{"GET":["never-read.toml"]},"wallet":"missing"});
    v["version"] = json!(2);
    v["servers"] = json!({});
    error(&v, "startup.catalog_concurrency must be in 1..64");
    v.as_object_mut().unwrap().remove("startup");
    error(&v, "network connect timeout must be 1..300 seconds");
    v.as_object_mut().unwrap().remove("network");
    error(&v, "relay_fallback.wallet must name a declared wallet");
    v.as_object_mut().unwrap().remove("relay_fallback");
    error(&v, "unsupported meta-config version 2");
    v["version"] = json!(1);
    error(&v, "at least one server is required");
}

#[test]
fn identifier_categories_and_names_precede_wallet_and_reference_checks() {
    let mut v = fixture();
    v["sources"]["Z"] = json!({});
    v["sources"]["A"] = json!({});
    v["wallets"]["B"] = v["wallets"]["payer"].clone();
    v["wallet_templates"] = json!({"C":managed()});
    v["servers"]["D"] = v["servers"]["one"].clone();
    v["wallets"]["payer"]["private_key_env"] = json!("bad key");
    v["sources"]["a"]["wallet"] = json!("missing");
    for (category, name) in [
        ("sources", "A"),
        ("sources", "Z"),
        ("wallets", "B"),
        ("wallet_templates", "C"),
        ("servers", "D"),
    ] {
        error(
            &v,
            &format!("invalid identifier {name}: use lowercase letters, digits and underscores"),
        );
        v[category].as_object_mut().unwrap().remove(name);
    }
    error(&v, "wallet payer: invalid environment reference");
    v["wallets"]["payer"]["private_key_env"] = json!("ABSENT_TEST_KEY");
    v["servers"]["one"]["wallet"] = json!("also_missing");
    error(&v, "source a: unknown wallet missing");
    v["sources"]["a"]["wallet"] = json!("payer");
    error(&v, "server one: unknown wallet also_missing");
}

#[test]
fn listener_checks_keep_their_first_error_and_context() {
    let mut v = fixture();
    let s = &mut v["servers"]["one"];
    s["allowed_hosts"] = json!([]);
    s["disable_host_check"] = json!(true);
    s["listen"] = json!("0.0.0.0:0");
    s["bearer_token_env"] = json!(" ");
    s["wallet"] = json!("missing");
    s["sources"] = json!([]);
    s["include_tools"] = json!([""]);
    error(
        &v,
        "server one: invalid Host policy: allowed_hosts and disable_host_check cannot be combined",
    );
    v["servers"]["one"]
        .as_object_mut()
        .unwrap()
        .remove("allowed_hosts");
    error(
        &v,
        "server one: this release requires a loopback listen address",
    );
    v["servers"]["one"]["listen"] = json!("127.0.0.1:0");
    error(
        &v,
        "server one: bearer_token_env is required unless auth = false",
    );
    v["servers"]["one"]["bearer_token_env"] = json!("TOKEN");
    error(&v, "server one: unknown wallet missing");
    v["servers"]["one"]["wallet"] = json!("payer");
    error(&v, "server one: sources cannot be empty");
    v["servers"]["one"]["sources"] = json!(["missing"]);
    error(&v, "server one: unknown source missing");
    v["servers"]["one"]["sources"] = json!(["a", "a", "missing"]);
    error(&v, "server one: duplicate source a");
    v["servers"]["one"]["sources"] = json!(["missing", "a", "a"]);
    error(&v, "server one: unknown source missing");
    v["servers"]["one"]["sources"] = json!(["a"]);
    error(&v, "empty tool selector");
    v["servers"]["one"]["include_tools"] = json!(["literal[brackets]*?"]);
    v["servers"]["one"]["exclude_tools"] = json!([""]);
    error(&v, "empty tool selector");
    v["servers"]["one"]["exclude_tools"] = json!([]);
    config(&v).validate().unwrap();
}

#[test]
fn address_uniqueness_and_listener_iteration_are_ordered() {
    let mut v = fixture();
    v["servers"]["one"]["listen"] = json!("127.0.0.1:4567");
    v["servers"]["two"] = v["servers"]["one"].clone();
    v["servers"]["two"]["bearer_token_env"] = json!("");
    v["servers"]["one"]["exclude_tools"] = json!([""]);
    error(&v, "empty tool selector");
    v["servers"]["one"]["exclude_tools"] = json!([]);
    error(&v, "duplicate listener 127.0.0.1:4567");
    v["servers"]["two"]["allowed_hosts"] = json!(["https://invalid"]);
    error(
        &v,
        "server two: invalid Host policy: allowed_hosts entries must be hostnames or IP addresses, optionally with a port; URLs and wildcards are not supported",
    );
    v["servers"]["two"]
        .as_object_mut()
        .unwrap()
        .remove("allowed_hosts");
    v["servers"]["two"]["auth"] = json!(false);
    for address in ["127.0.0.1:0", "[::1]:0"] {
        v["servers"]["one"]["listen"] = json!(address);
        v["servers"]["two"]["listen"] = json!(address);
        config(&v).validate().unwrap();
    }
}

#[test]
fn auth_whitespace_and_host_settings_remain_independent() {
    for (auth, token, expected) in [
        (
            true,
            "",
            Some("server one: bearer_token_env is required unless auth = false"),
        ),
        (
            true,
            " \t",
            Some("server one: bearer_token_env is required unless auth = false"),
        ),
        (true, " TOKEN ", None),
        (false, "", None),
        (
            false,
            "TOKEN",
            Some("server one: omit bearer_token_env when auth = false"),
        ),
        (
            false,
            " ",
            Some("server one: omit bearer_token_env when auth = false"),
        ),
    ] {
        for hosts in [Value::Null, json!([]), json!(["localhost"])] {
            let mut v = fixture();
            v["servers"]["one"]["auth"] = json!(auth);
            v["servers"]["one"]["bearer_token_env"] = json!(token);
            v["servers"]["one"]["allowed_hosts"] = hosts;
            if let Some(expected) = expected {
                error(&v, expected);
            } else {
                config(&v).validate().unwrap();
            }
        }
    }
}

#[test]
fn dynamic_only_listener_is_valid_but_does_not_bypass_policy() {
    let mut v = fixture();
    v["servers"]["one"]["sources"] = json!([]);
    v["servers"]["one"]["source_management"] = json!(true);
    v["servers"]["one"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    error(&v, "server one: source_management requires wallet");
    v["servers"]["one"]["wallet"] = json!("payer");
    v["version"] = json!(2);
    error(&v, "unsupported meta-config version 2");
    v["version"] = json!(1);
    assert_eq!(resolved(&v)["bindings"], json!({"one":{}}));
}

#[test]
fn assignment_precedes_resolved_treasury_and_funding_requirements() {
    let mut v = fixture();
    v["wallet_templates"] = json!({"small":managed()});
    // An unused managed template alone does not require treasury or funding.
    config(&v).validate().unwrap();
    v["wallet_assignment"] = json!({"scope":"deployment","template":"missing"});
    v["wallets"]["payer"] = managed();
    v["servers"]["one"]["exclude_tools"] = json!([""]);
    error(&v, "empty tool selector");
    v["servers"]["one"]["exclude_tools"] = json!([]);
    error(&v, "unknown wallet template missing");
    v["wallet_assignment"]["template"] = json!("small");
    error(&v, "managed profiles require [treasury]");
    v["treasury"] = treasury();
    v["treasury"]["confirmations"] = json!(0);
    error(&v, "invalid treasury limits");
    v["treasury"]["confirmations"] = json!(1);
    error(&v, "managed profiles require [funding]");
    v["funding"] = json!({"base_rpc_url_env":"bad name"});
    error(&v, "invalid environment reference");
    v["funding"]["base_rpc_url_env"] = json!("ABSENT_TEST_RPC");
    config(&v).validate().unwrap();
    // Static-only deployments still validate optional supplied blocks, in order.
    v["wallets"]["payer"] = fixture()["wallets"]["payer"].clone();
    v["treasury"]["confirmations"] = json!(0);
    v["funding"]["base_rpc_url_env"] = json!("bad name");
    error(&v, "invalid treasury limits");
    v.as_object_mut().unwrap().remove("treasury");
    error(&v, "invalid environment reference");
}

#[test]
fn complete_static_resolution_preserves_source_override_and_listener_sharing() {
    let mut v = fixture();
    v["wallets"]["other"] =
        json!({"mode":"static","private_key_env":"OTHER_KEY","max_api_payment_usdc":"2"});
    v["servers"]["one"]["wallet"] = json!("other");
    v["servers"]["two"] = v["servers"]["one"].clone();
    assert_eq!(
        resolved(&v),
        json!({
        "wallets":v["wallets"],"generated":{},
        "bindings":{
            "one":{"a":{"wallet":"payer","origin":"sources.a.wallet"},"b":{"wallet":"other","origin":"servers.one.wallet"}},
            "two":{"a":{"wallet":"payer","origin":"sources.a.wallet"},"b":{"wallet":"other","origin":"servers.two.wallet"}}},
        "summary":{"managed_pool_count":0,"generated_pool_count":0,"initial_inventory_target_atomic":"0","initial_inventory_target_usdc":"0.000000"}})
    );
}

#[test]
fn complete_generated_resolution_and_explicit_managed_resolution() {
    let mut v = fixture();
    v["sources"]["a"].as_object_mut().unwrap().remove("wallet");
    v["servers"]["one"]
        .as_object_mut()
        .unwrap()
        .remove("wallet");
    v["wallets"] = json!({});
    v["wallet_templates"] = json!({"small":managed()});
    v["wallet_assignment"] = json!({"scope":"deployment","template":"small"});
    error(&v, "managed profiles require [treasury]");
    v["treasury"] = treasury();
    v["funding"] = json!({"base_rpc_url_env":"ABSENT_TEST_RPC"});
    v["servers"]["two"] = v["servers"]["one"].clone();
    let binding = json!({"wallet":"auto_v1_deployment","origin":"wallet_assignment","template":"small","scope":"deployment"});
    let summary = json!({"managed_pool_count":1,"generated_pool_count":1,"initial_inventory_target_atomic":"5000000","initial_inventory_target_usdc":"5.000000"});
    assert_eq!(
        resolved(&v),
        json!({"wallets":{"auto_v1_deployment":managed()},
        "generated":{"auto_v1_deployment":{"template":"small","scope":"deployment"}},
        "bindings":{"one":{"a":binding,"b":binding},"two":{"a":binding,"b":binding}},"summary":summary})
    );
    let mut explicit = fixture();
    explicit["wallets"]["payer"] = managed();
    explicit["treasury"] = treasury();
    explicit["funding"] = v["funding"].clone();
    assert_eq!(
        resolved(&explicit),
        json!({"wallets":{"payer":managed()},"generated":{},
        "bindings":{"one":{"a":{"wallet":"payer","origin":"sources.a.wallet"},"b":{"wallet":"payer","origin":"servers.one.wallet"}}},
        "summary":{"managed_pool_count":1,"generated_pool_count":0,"initial_inventory_target_atomic":"5000000","initial_inventory_target_usdc":"5.000000"}})
    );
}

#[test]
fn retired_response_character_setting_is_rejected_by_name() {
    let mut value = fixture();
    value["servers"]["one"]["max_response_chars"] = json!(1);
    let error = serde_json::from_value::<MetaConfig>(value).err().unwrap();
    assert!(error.to_string().contains("max_response_chars"));
}

#[test]
fn canonical_listener_token_is_optional_only_when_absent() {
    let mut value = fixture();
    value["servers"]["one"]["bearer_token_env"] = json!("X402_TREAZURY_MCP_TOKEN");
    let listener = config(&value).servers.remove("one").unwrap();
    let mut env = std::collections::BTreeMap::new();
    assert_eq!(listener.token(&env).unwrap(), None);
    for token in ["", " \t"] {
        env.insert("X402_TREAZURY_MCP_TOKEN".into(), token.into());
        assert!(listener.token(&env).is_err());
    }
    env.insert("X402_TREAZURY_MCP_TOKEN".into(), "fixture-token".into());
    assert_eq!(
        listener.token(&env).unwrap().as_deref(),
        Some("fixture-token")
    );
    let mut custom = listener.clone();
    custom.bearer_token_env = "CUSTOM_TOKEN".into();
    assert!(custom.token(&env).is_err());
    custom.auth = false;
    assert_eq!(custom.token(&env).unwrap(), None);
}
