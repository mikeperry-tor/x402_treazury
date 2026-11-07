use super::*;
use serde_json::{Value, json};

#[tokio::test]
async fn direct_discovery_dispatch_and_status_share_listener_authority() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let (store, worker) = crate::rotation::store::StoreHandle::spawn(state);
    let operation = store
        .call(|s| {
            s.reserve_direct(
                "alice",
                "receive",
                "original",
                json!({"private_terms":"alice"}),
            )
        })
        .await
        .unwrap()
        .0;
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = crate::direct_swap::Service::new(
        store.clone(),
        treasury,
        crate::rotation::near::NearClient::with_session(None, None).unwrap(),
        serde_json::from_value(json!({})).unwrap(),
        0,
    );
    let mut server = Server::from_bindings(vec![], None);
    server.catalog_server = "alice".into();
    server.discover_on_demand = true;
    server.direct_swap = Some((
        service,
        crate::direct_swap::Config {
            receive: true,
            send: false,
            ..Default::default()
        },
    ));
    assert!(
        server
            .get_info()
            .instructions
            .unwrap()
            .contains("Native receiving needs no swap provider")
    );
    assert!(
        server
            .management_tools()
            .iter()
            .all(|t| !t.name.contains("swap_status"))
    );
    let search: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_tools_search",
                json!({"limit":1}).as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(search["total"], 3);
    assert_eq!(search["items"].as_array().unwrap().len(), 1);
    assert!(search["items"][0]["input_schema"]["properties"].is_object());
    let next: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_tools_search",
                json!({"limit":1,"cursor":search["next_cursor"]})
                    .as_object()
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_ne!(search["items"][0]["tool_id"], next["items"][0]["tool_id"]);
    let native: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_tools_search",
                json!({"query":"Zcash receive"}).as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        native["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["tool_id"] == "x402_treazury_payment_request_create")
    );
    server.direct_swap.as_mut().unwrap().1.receive = false;
    let disabled: Value = serde_json::from_str(
        &server
            .invoke("x402_treazury_tools_search", &Map::new())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(disabled["total"], 1);
    assert_eq!(disabled["items"][0]["tool_id"], "x402_treazury_swap_status");
    assert!(
        server
            .invoke(
                "x402_treazury_tools_search",
                json!({"cursor":search["next_cursor"]}).as_object().unwrap()
            )
            .await
            .is_err()
    );
    for args in [
        json!({}),
        json!({"tool_id":"x402_treazury_payment_request_create","arguments":{}}),
    ] {
        let name = if args.get("tool_id").is_some() {
            "x402_treazury_tool_call"
        } else {
            "x402_treazury_payment_request_create"
        };
        assert!(
            server
                .invoke(name, args.as_object().unwrap())
                .await
                .is_err()
        );
    }
    let call =
        json!({"tool_id":"x402_treazury_swap_status","arguments":{"operation_id":operation.id}});
    let own: Value = serde_json::from_str(
        &server
            .invoke("x402_treazury_tool_call", call.as_object().unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(own["operation"]["terms"]["private_terms"], "alice");
    server.catalog_server = "bob".into();
    assert!(
        server
            .invoke("x402_treazury_tool_call", call.as_object().unwrap())
            .await
            .is_err()
    );
    assert!(
        server
            .invoke(
                "x402_treazury_swap_status",
                call["arguments"].as_object().unwrap()
            )
            .await
            .is_err()
    );
    drop(server);
    drop(commands);
    drop(store);
    worker.await.unwrap();
}
