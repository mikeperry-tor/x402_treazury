//! Curated discovery stays offline; registration retains endpoint ownership and idempotency.
use super::*;

async fn without_wallets() -> Arc<Manager> {
    Manager::new(
        listeners(false),
        BTreeMap::new(),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn public_local_tools_dispatch_offline_with_complete_json() {
    let m = without_wallets().await;
    let s = server(&m, "writer");
    let search = s
        .invoke(
            local::NAME,
            json!({"q":"xqzunmatchedcapabilitysentinel"})
                .as_object()
                .unwrap(),
        )
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&search).unwrap();
    assert_eq!(value["total"], 0);
    assert_eq!(value["items"], json!([]));
    let listed = s
        .invoke(local::NAME, json!({}).as_object().unwrap())
        .await
        .unwrap();
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed["per_page"], 25);
    assert!(listed["items"].is_array());
    let query = "retrieval contents";
    let expected = crate::providers::browse(Some(query), 1, 25).unwrap();
    let ranked = s
        .invoke(local::NAME, json!({"q":query}).as_object().unwrap())
        .await
        .unwrap();
    let ranked: Value = serde_json::from_str(&ranked).unwrap();
    let ids = |value: &Value| -> Vec<String> {
        value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["provider_id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(!expected["items"].as_array().unwrap().is_empty());
    assert_eq!(ids(&ranked), ids(&expected));
    assert_eq!(ranked["items"], expected["items"]);
    assert_eq!(ranked["total"], expected["total"]);
    assert!(
        m.payers.is_empty(),
        "public local search must not need a wallet or documentation identity"
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[tokio::test]
async fn public_provider_repeat_retains_accepted_configuration_after_bundle_removal() {
    let m = manager(None).await;
    let id = super::curated_tests::commit_curated(&m, "retired").await;
    let before = serde_json::to_value(&m.inner.lock().unwrap().state.records[&id]).unwrap();
    let result = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_local",
            json!({"provider_id":"retired-provider-not-in-bundle","name":"ignored"}),
        )
        .await
        .unwrap();
    assert_eq!(result["name"], "retired");
    assert_eq!(result["provider_id"], "retired-provider-not-in-bundle");
    assert_eq!(
        serde_json::to_value(&m.inner.lock().unwrap().state.records[&id]).unwrap(),
        before
    );
    assert!(m.fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn local_search_and_list_work_without_any_wallet_or_network_client() {
    let m = without_wallets().await;
    let list = m.invoke("writer", local::NAME, json!({})).await.unwrap();
    let search: Value = serde_json::from_str(
        &m.invoke("writer", local::NAME, json!({}))
            .await
            .unwrap()
            .to_string(),
    )
    .unwrap();
    assert_eq!(list, search);
    assert_eq!(list["total"], crate::providers::all().len());
    for item in list["items"].as_array().unwrap() {
        assert!(item["provider_id"].is_string());
        assert!(!item["description"].as_str().unwrap().trim().is_empty());
        assert!(item.get("tools").is_none());
    }
    let found: Value = serde_json::from_str(
        &m.invoke("writer", local::NAME, json!({"q":"EXA"}))
            .await
            .unwrap()
            .to_string(),
    )
    .unwrap();
    assert!(
        found["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["provider_id"] == "exa")
    );
    let first = m
        .invoke("writer", local::NAME, json!({"per_page":1}))
        .await
        .unwrap();
    let second = m
        .invoke("writer", local::NAME, json!({"per_page":1,"page":2}))
        .await
        .unwrap();
    assert_eq!(first["next_page"], 2);
    assert_ne!(
        first["items"][0]["provider_id"],
        second["items"][0]["provider_id"]
    );
    assert_eq!(list["per_page"], 25);
    for size in [101, u32::MAX as u64 + 1, u64::MAX] {
        let all = m
            .invoke("writer", local::NAME, json!({"per_page":size}))
            .await
            .unwrap();
        let total = all["total"].as_u64().unwrap();
        assert_eq!(
            all["items"].as_array().unwrap().len() as u64,
            size.min(total)
        );
        assert_eq!(
            all["next_page"],
            json!(if size < total { Some(2) } else { None })
        );
    }
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[tokio::test]
async fn local_arguments_and_authority_reject_before_wallet_lookup() {
    let m = without_wallets().await;
    for args in [
        json!({"local_only":true}),
        json!({"mode":"browse"}),
        json!({"q":null}),
        json!({"page":0}),
        json!({"per_page":0}),
        json!({"category":"finance"}),
    ] {
        let error = m.invoke("writer", local::NAME, args).await.unwrap_err();
        assert!(!error.to_string().contains("wallet unavailable"));
    }
    assert!(m.invoke("hidden", local::NAME, json!({})).await.is_err());
    assert_eq!(
        tools::listener_definitions(policy::EndpointPolicy::default(), true, None, None).len(),
        2
    );
    assert_eq!(
        tools::listener_definitions(m.policy("writer"), true, None, None).len(),
        8
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[tokio::test]
async fn curated_details_match_without_documentation_identity_or_probes() {
    let m = Manager::new(
        listeners(false),
        BTreeMap::from([("shared".into(), PaidClient::unsigned())]),
        Arc::new(CatalogState::default()),
        vec![],
    )
    .await
    .unwrap();
    for (base, count) in [
        ("https://netintel.dev/", 1),
        ("https://api.relayshield.net", 2),
    ] {
        let output = m
            .invoke_source_details("writer", json!({"base_url":base}).as_object().unwrap())
            .await
            .unwrap();
        assert_eq!(output["url_discovery"]["status"], "bundled_provider_found");
        assert_eq!(output["provider_matches"].as_array().unwrap().len(), count);
        assert_eq!(output["url_discovery"]["results"], json!([]));
    }
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[tokio::test]
async fn curated_details_preserve_the_single_directory_listing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let listing =
        json!({"data":{"service":{"base_url":"https://netintel.dev","name":"Listing metadata"}}});
    let response = listing.clone();
    let app = axum::Router::new().fallback(axum::routing::get(move || {
        seen.fetch_add(1, Ordering::SeqCst);
        let response = response.clone();
        async move { axum::Json(response) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let m = manager(None).await;
    *m.fixture_directory.lock().unwrap() = Some(format!("http://{address}"));
    let output = m
        .invoke_source_details(
            "writer",
            json!({"x402_list_id":"vendor"}).as_object().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(output["listing"], listing);
    assert_eq!(output["url_discovery"]["status"], "bundled_provider_found");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn local_provider_add_uses_bundled_schema_and_stable_short_registration_name() {
    let m = manager(None).await;
    let args = json!({"provider_id":"relayshield-phone-reputation"});
    let first = m
        .invoke("writer", "x402_treazury_providers_add_local", args.clone())
        .await
        .unwrap();
    assert_eq!(first["name"], "relayshield-phone-reputation");
    assert_eq!(first["provider_id"], "relayshield-phone-reputation");
    assert_eq!(first["schema"]["kind"], "bundled");
    assert_eq!(first["tool_count"], 1);
    let repeated = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_local",
            json!({"provider_id":"relayshield-phone-reputation","name":"ignored"}),
        )
        .await
        .unwrap();
    assert_eq!(first, repeated);
    let found = m
        .search_tools(
            "writer",
            "x402_treazury_tools_search",
            json!({"source_name":"relayshield-phone-reputation"}),
        )
        .unwrap();
    assert_eq!(
        found["items"][0]["tool_id"],
        "dyn_relayshield-phone-reputation__phone_reputation"
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
}

fn netintel_document() -> Value {
    json!({"openapi":"3.0.3","servers":[{"url":"https://broken.invalid"}],"paths":{"/dns/lookup":{"post":{"summary":"DNS lookup"}}}})
}

fn seed_netintel(m: &Manager) {
    let provider = AcceptedProvider::current("netintel").unwrap();
    let (client, authority) = m.source_client("writer").unwrap().pin_document_authority();
    let cell = OnceCell::new();
    cell.set(Fetched {
        completed: Instant::now(),
        result: Ok(Arc::new(Document::local(
            netintel_document().to_string().into_bytes(),
        ))),
    })
    .unwrap();
    m.fetches.lock().unwrap().insert(
        FetchKey {
            authority,
            policy: client.document_policy_key(),
            url: provider.settings.spec.clone(),
            transport: provider.settings.transport(),
            timeout: import::read_timeout(Some(&provider)),
            max_spec_bytes: import::max_spec_bytes(Some(&provider)),
        },
        Arc::new(cell),
    );
}

#[tokio::test]
async fn known_spec_and_provider_selectors_share_registration_under_racing_adds() {
    let m = manager(None).await;
    seed_netintel(&m);
    let spec = crate::providers::get("netintel")
        .unwrap()
        .config
        .spec
        .clone();
    let (by_id, by_url) = tokio::join!(
        m.invoke(
            "writer",
            "x402_treazury_providers_add_local",
            json!({"provider_id":"netintel","name":"net"})
        ),
        m.invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":spec,"name":"net"})
        ),
    );
    assert_eq!(by_id.unwrap(), by_url.unwrap());
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 1);
    let snapshot = m.catalog.read();
    assert!(
        snapshot.views["writer"]
            .iter()
            .all(|b| b.base == "https://netintel.dev")
    );
    assert!(
        snapshot.views["writer"]
            .iter()
            .any(|b| b.tool.name == "dyn_net__dns_lookup")
    );
    assert!(snapshot.views["reader"].is_empty());
}

#[tokio::test]
async fn legacy_generic_url_repeat_wins_even_when_curated_source_coexists() {
    let m = manager(None).await;
    seed_netintel(&m);
    let url = crate::providers::get("netintel")
        .unwrap()
        .config
        .spec
        .clone();
    let mut c: Candidate =
        serde_json::from_value(candidate("legacy", "process", "server")).unwrap();
    c.spec_url = url.clone();
    let mut legacy_document = netintel_document();
    legacy_document["servers"] = json!([{ "url":"https://api.example.com" }]);
    let doc = legacy_document.to_string();
    let id = Uuid::new_v4().to_string();
    m.commit(
        "writer".into(),
        false,
        Record {
            id,
            owner: "writer".into(),
            candidate: c,
            targets: vec![],
            revision: 1,
            hash: hash(doc.as_bytes()),
            document: doc,
            format: FORMAT,
            accepted_at: epoch(),
            updated_at: epoch(),
            removed: false,
            disabled: None,
        },
    )
    .await
    .unwrap();
    m.invoke(
        "writer",
        "x402_treazury_providers_add_local",
        json!({"provider_id":"netintel","name":"curated"}),
    )
    .await
    .unwrap();
    let repeat = m
        .invoke(
            "writer",
            "x402_treazury_providers_add_remote",
            json!({"spec_url":url,"name":"ignored"}),
        )
        .await
        .unwrap();
    assert_eq!(repeat["name"], "legacy");
    assert!(repeat.get("provider_id").is_none());
    assert_eq!(m.inner.lock().unwrap().state.records.len(), 2);
}

#[tokio::test]
async fn provider_add_rejects_unknown_null_mixed_and_unauthorized_inputs_before_io() {
    let m = manager(None).await;
    for args in [
        json!({}),
        json!({"provider_id":null}),
        json!({"provider_id":"unknown-provider"}),
        json!({"provider_id":"relayshield-phone-reputation","spec_url":null}),
        json!({"provider_id":null,"spec_url":"https://api.example.com/openapi.json"}),
        json!({"provider_id":"relayshield-phone-reputation","wallet":"spoofed"}),
        json!({"provider_id":"relayshield-phone-reputation","name":"bad__name"}),
    ] {
        assert!(
            m.invoke("writer", "x402_treazury_providers_add_local", args)
                .await
                .is_err()
        );
    }
    assert!(
        m.invoke(
            "hidden",
            "x402_treazury_providers_add_local",
            json!({"provider_id":"relayshield-phone-reputation"})
        )
        .await
        .is_err()
    );
    assert!(m.fetches.lock().unwrap().is_empty());
    assert!(m.inner.lock().unwrap().state.records.is_empty());
}

#[test]
fn ambiguous_spec_choices_only_recommend_available_local_registration() {
    let first = crate::providers::get("exa").unwrap();
    let second = crate::providers::get("netintel").unwrap();
    assert!(curated::select_spec_match(&[], false).unwrap().is_none());
    let selected = curated::select_spec_match(&[first], false)
        .unwrap()
        .unwrap();
    assert_eq!(selected.id, first.id);
    assert_eq!(selected.settings.spec, first.config.spec);
    let local = curated::select_spec_match(&[first, second], true)
        .unwrap_err()
        .to_string();
    assert!(
        local.contains("providers_add_local")
            && local.contains("exa")
            && local.contains("netintel")
    );
    let remote = curated::select_spec_match(&[first, second], false)
        .unwrap_err()
        .to_string();
    assert!(remote.contains("operator"));
    assert!(
        !remote.contains("providers_add_local")
            && !remote.contains("exa")
            && !remote.contains("netintel")
    );
}
