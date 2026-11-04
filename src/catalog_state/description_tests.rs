//! Live presentation reads trusted hints without changing the canonical catalog.
use super::*;
use crate::{
    catalog::{Config, build_tools},
    payment::SpendPolicy,
    pricing::runtime::received,
    rotation::{
        base::BaseRpc,
        manager::ManagedPool,
        store::{Store, StoreHandle},
    },
};
use alloy_primitives::U256;
use rmcp::ServerHandler;
use serde_json::json;

#[tokio::test(start_paused = true)]
async fn scoped_descriptions_refresh_expire_and_leave_search_cursors_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = Store::create(
        &dir.path().join("state"),
        &dir.path().join("key"),
        1,
        b"fixture",
    )
    .unwrap();
    let group = state
        .ensure_capacity_group(
            "display",
            &crate::rotation::test_capacity::small_reserve("0.01"),
            Some(U256::from(10_000)),
            0,
        )
        .unwrap();
    assert!(state.funding_jobs().unwrap().is_empty());
    let (store, worker) = StoreHandle::spawn(state);
    {
        let client = PaidClient::managed(Arc::new(
            ManagedPool::new(
                store.clone(),
                group,
                BaseRpc::new("https://rpc.example.com", 12).unwrap(),
                SpendPolicy::dollars("0.01").unwrap(),
            )
            .unwrap(),
        ));
        let hints = crate::pricing::runtime::fixture(dir.path()).await;
        let mut definitions = build_tools(
            &Config::default(),
            &json!({"paths":{
                "/alpha":{"get":{"summary":"Read alpha records."}},
                "/beta":{"get":{"summary":"Read beta records."}}
            }}),
            "display",
        )
        .unwrap();
        for tool in &mut definitions {
            tool.catalog_hints = Some(hints.clone());
        }
        let mut tools = bind(
            definitions
                .into_iter()
                .map(|tool| (tool, client.clone(), "https://api.example.com".into()))
                .collect(),
        );
        for tool in &mut tools {
            tool.source = Some(("registered".into(), 7));
        }
        let key = hints.operation("GET", "/alpha");
        let fallback = tools[0].tool.description.clone();
        assert_eq!(tools[0].description(), fallback);
        key.learn(U256::from(5000), received().unwrap());
        let expected = "Read alpha records. Observed cost: ~$0.005/call.";
        assert_eq!(tools[0].description(), expected);
        assert_eq!(tools[0].tool.description, fallback);
        assert_eq!(
            serde_json::to_value(&tools[0].tool).unwrap()["description"],
            fallback
        );
        for mutation in 0..5 {
            let mut other = tools[0].clone();
            match mutation {
                0 => other.source = Some(("other".into(), 8)),
                1 => other.base = "https://other.example.com".into(),
                2 => other.client = PaidClient::unsigned(),
                3 => {
                    other.client = other.client.with_transport(crate::network::HttpPolicy {
                        allow_http1: true,
                        allow_tls12: true,
                    })
                }
                _ => other.tool.name = "alias".into(),
            }
            assert_eq!(other.description(), expected);
        }
        for mutation in 0..4 {
            let mut other = tools[0].clone();
            match mutation {
                0 => other.tool.method = "POST".into(),
                1 => other.tool.path = "/other".into(),
                2 => other.tool.catalog_hints = None,
                _ => other.tool.help_url = Some("https://api.example.com/help".into()),
            }
            assert_eq!(other.description(), fallback);
        }
        let catalog = Arc::new(CatalogState::new(CatalogSnapshot {
            generation: 11,
            views: BTreeMap::from([("default".into(), tools.clone())]),
            instructions: Default::default(),
        }));
        let mut server =
            crate::server::Server::new(vec![], PaidClient::unsigned(), String::new(), None);
        server.catalog = catalog.clone();
        assert_eq!(
            server
                .get_tool(&tools[0].tool.name)
                .unwrap()
                .description
                .as_deref(),
            Some(expected)
        );
        let search = |args| {
            crate::discovery::search_catalog(
                &catalog,
                "default",
                "x402_treazury_tools_search",
                args,
            )
            .unwrap()
        };
        let first = search(json!({"limit":1}));
        assert_eq!(first["items"][0]["description"], expected);
        assert_eq!(first["catalog_generation"], 11);
        assert_eq!(first["total"], 2);
        assert_eq!(search(json!({"query":"observed"}))["total"], 0);
        let cursor = first["next_cursor"].clone();
        key.learn(U256::from(7000), received().unwrap());
        let second = search(json!({"limit":1,"cursor":cursor}));
        assert_eq!(second["items"][0]["tool_id"], tools[1].tool.name);
        assert_eq!(second["catalog_generation"], 11);
        assert_eq!(
            search(json!({"limit":1}))["items"][0]["description"],
            "Read alpha records. Observed cost: ~$0.007/call."
        );
        let mut refreshed = tools.clone();
        refreshed[0].tool.catalog_hints = None;
        catalog.publish(CatalogSnapshot {
            generation: 12,
            views: BTreeMap::from([("default".into(), refreshed)]),
            instructions: Default::default(),
        });
        assert_eq!(
            server
                .get_tool(&tools[0].tool.name)
                .unwrap()
                .description
                .as_deref(),
            Some(fallback.as_str())
        );
        assert!(
            crate::discovery::search_catalog(
                &catalog,
                "default",
                "x402_treazury_tools_search",
                json!({"limit":1,"cursor":cursor})
            )
            .is_err()
        );
        hints.invalidate();
        assert_eq!(tools[0].description(), fallback);
        assert!(
            store
                .call(|state| Ok(state.funding_jobs()?.is_empty()))
                .await
                .unwrap()
        );
    }
    drop(store);
    worker.await.unwrap();
}
