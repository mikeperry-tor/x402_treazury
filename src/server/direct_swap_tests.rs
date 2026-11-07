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

#[tokio::test]
async fn send_only_discovery_native_preview_and_disabled_status_preserve_authority() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::rotation::store::Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let (store, worker) = crate::rotation::store::StoreHandle::spawn(state);
    store
        .call(|s| s.reserve_direct("alice", "send", "accepted", json!({"original":"alice"})))
        .await
        .unwrap();
    let (treasury, commands) = crate::treasury::actor::channel();
    let service = crate::direct_swap::Service::native(store.clone(), treasury, 15_000, 5);
    let mut server = Server::from_bindings(vec![], None);
    server.catalog_server = "alice".into();
    server.direct_swap = Some((
        service,
        crate::direct_swap::Config {
            send: true,
            ..Default::default()
        },
    ));
    for name in ["x402_treazury_swap_send", "x402_treazury_swap_preview"] {
        let definition = server.get_tool(name).expect("enabled send schema");
        assert!(definition.input_schema["properties"]["recipient"].is_object());
        assert!(
            definition.input_schema["properties"]
                .as_object()
                .unwrap()
                .get("owner")
                .is_none()
        );
        assert!(
            server
                .management_tools()
                .iter()
                .any(|tool| tool.name == name)
        );
    }
    assert!(
        server
            .get_tool("x402_treazury_payment_request_create")
            .is_none()
    );
    assert!(
        server
            .get_info()
            .instructions
            .unwrap()
            .contains("Native sending needs no swap provider")
    );
    server.discover_on_demand = true;
    assert!(
        server
            .management_tools()
            .iter()
            .all(|tool| tool.name != "x402_treazury_swap_send")
    );
    let search: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_tools_search",
                json!({"query":"Zcash send"}).as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    for name in ["x402_treazury_swap_send", "x402_treazury_swap_preview"] {
        assert!(
            search["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["tool_id"] == name)
        );
    }
    let preview_args = json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec","amount":"1000","recipient":SHIELDED_RECIPIENT});
    let preview: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_swap_preview",
                preview_args.as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(preview["executable"], false);
    let wrapped = json!({"tool_id":"x402_treazury_swap_preview","arguments":preview_args});
    let same: Value = serde_json::from_str(
        &server
            .invoke("x402_treazury_tool_call", wrapped.as_object().unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(preview, same);
    // An owner argument is not a routing authority, even on an enabled send tool.
    let foreign_owner = json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec","amount":"1000","recipient":SHIELDED_RECIPIENT,"request_id":"denied","owner":"bob"});
    assert!(
        server
            .invoke(
                "x402_treazury_swap_send",
                foreign_owner.as_object().unwrap()
            )
            .await
            .is_err()
    );
    server.direct_swap.as_mut().unwrap().1.send = false;
    let send = json!({"asset_id":crate::direct_swap::route::ZEC,"chain":"zec","amount":"1000","recipient":SHIELDED_RECIPIENT,"request_id":"denied"});
    for (name, args) in [
        ("x402_treazury_swap_send", send.clone()),
        (
            "x402_treazury_tool_call",
            json!({"tool_id":"x402_treazury_swap_send","arguments":send}),
        ),
    ] {
        assert!(
            server
                .invoke(name, args.as_object().unwrap())
                .await
                .is_err()
        );
    }
    assert!(server.get_tool("x402_treazury_swap_send").is_none());
    let disabled: Value = serde_json::from_str(
        &server
            .invoke("x402_treazury_tools_search", &Map::new())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(disabled["total"], 1);
    let status_args = json!({"kind":"send","request_id":"accepted"});
    let status: Value = serde_json::from_str(
        &server
            .invoke(
                "x402_treazury_swap_status",
                status_args.as_object().unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status["operation"]["terms"]["original"], "alice");
    server.catalog_server = "bob".into();
    assert!(
        server
            .invoke(
                "x402_treazury_swap_status",
                status_args.as_object().unwrap()
            )
            .await
            .is_err()
    );
    store
        .call(|s| {
            assert!(s.direct_request("alice", "send", "denied").is_err());
            assert!(s.funding_jobs()?.is_empty());
            assert!(!s.status()?.outgoing_pending);
            Ok(())
        })
        .await
        .unwrap();
    drop((server, commands, store));
    worker.await.unwrap();
}

// Public unfunded Orchard/Sapling receiver from the existing route contract fixture.
const SHIELDED_RECIPIENT: &str = "u1twhzfnwvps8vlv20jak2lud8x3j5y9p42unjrq69kvhlsu3yp88e8m4jn9r53369su3mx2e9hjan5kl4pwqkfgetam7v7rzwfduu2q6dp90um6al5zjyk3akns2923va5xyn92f6f5neq9amdshzpj67n2e9ez94qj36hgagpv5ltwk7";
