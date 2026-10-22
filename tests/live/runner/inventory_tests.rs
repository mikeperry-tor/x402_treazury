use std::sync::{Arc, Mutex};

enum InventoryReply {
    Result(Value),
    HttpFailure,
    WrongIdentity,
    Stall(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>),
}
struct InventoryFixture {
    client: Client,
    requests: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for InventoryFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl InventoryFixture {
    async fn new(replies: Vec<InventoryReply>) -> Self {
        use axum::response::IntoResponse;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
        let app = axum::Router::new().route("/mcp", axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            log.lock().unwrap().push(request.clone());
            let reply = replies.lock().unwrap().pop_front();
            async move {
                match reply {
                    Some(InventoryReply::Result(result)) => axum::Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response(),
                    Some(InventoryReply::HttpFailure) | None => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    Some(InventoryReply::WrongIdentity) => axum::Json(json!({"jsonrpc":"2.0","id":"wrong","result":{}})).into_response(),
                    Some(InventoryReply::Stall(entered, release)) => {
                        entered.notify_one();
                        release.notified().await;
                        axum::Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{"tools":[],"nextCursor":"later"}})).into_response()
                    }
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let network = x402_treazury::network::NetworkContext::new(Default::default()).unwrap();
        let client = Client {
            http: network
                .discovery(&endpoint, std::time::Duration::from_secs(10))
                .unwrap(),
            endpoint,
            token: "fixture".into(),
        };
        Self {
            client,
            requests,
            server,
        }
    }
    fn trace(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}
fn inventory_request(page: usize, cursor: Option<&str>, discovery: bool) -> Value {
    let (id, method, params) = if discovery {
        let mut arguments = json!({"limit":100});
        if let Some(cursor) = cursor {
            arguments["cursor"] = json!(cursor);
        }
        (
            format!("qualification-discovery-{page}"),
            "tools/call",
            json!({"name":"x402_treazury_tools_search","arguments":arguments}),
        )
    } else {
        (
            format!("qualification-inventory-{page}"),
            "tools/list",
            cursor.map_or_else(|| json!({}), |cursor| json!({"cursor":cursor})),
        )
    };
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
async fn inventory_case(
    replies: Vec<Value>,
    expected: Value,
    error: Option<&str>,
    trace: Vec<Value>,
) {
    let fixture =
        InventoryFixture::new(replies.into_iter().map(InventoryReply::Result).collect()).await;
    let result = fixture
        .client
        .check_inventory(&expected, crate::files::DOCUMENT_BYTES)
        .await;
    match error {
        Some(error) => assert_eq!(result.unwrap_err().to_string(), error, "{expected}"),
        None => result.unwrap(),
    }
    assert_eq!(fixture.trace(), trace);
}
fn eager_page(tools: Value, cursor: Value) -> Value {
    json!({"tools":tools,"nextCursor":cursor})
}
fn discovery_page(items: Value, cursor: Value) -> Value {
    json!({"structuredContent":{"items":items,"next_cursor":cursor}})
}

#[tokio::test]
async fn inventory_protocols_preserve_exact_multpage_requests_and_nullable_fields() {
    let eager = vec![
        inventory_request(0, None, false),
        inventory_request(1, Some(""), false),
    ];
    inventory_case(vec![eager_page(json!([{"name":"","description":"whole description\nno trimming","inputSchema":{"type":"object"}}]),json!("")), json!({"tools":[]})], json!({"discover_on_demand":"true","tools":[{"name":"","description":"whole description\nno trimming","input_schema":{"type":"object"}}]}), None, eager.clone()).await;
    let mut trace = eager;
    trace.extend([
        inventory_request(0, None, true),
        inventory_request(1, Some(""), true),
    ]);
    inventory_case(vec![eager_page(json!([{"name":"local"}]),json!("")), eager_page(json!([]),Value::Null), discovery_page(json!([{"tool_id":"api"}]),json!("")), json!({"isError":"true","structuredContent":{"items":[]}})], json!({"discover_on_demand":true,"tools":[{"name":"api","input_schema":null}],"management_tools":[{"name":"local","description":null}]}), None, trace).await;
}

#[tokio::test]
async fn inventory_eager_errors_stop_before_cursor_or_discovery() {
    for (result, error) in [
        (json!({}), "MCP inventory missing tools"),
        (json!({"tools":{}}), "MCP inventory missing tools"),
        (eager_page(json!([{}]), json!(7)), "MCP tool missing name"),
        (
            eager_page(json!([{"name":7}]), Value::Null),
            "MCP tool missing name",
        ),
        (
            eager_page(json!([{"name":"x"},{"name":"x"}]), json!(7)),
            "duplicate MCP tool name",
        ),
        (
            eager_page(json!([]), json!(7)),
            "invalid/repeated inventory cursor",
        ),
    ] {
        inventory_case(
            vec![result],
            json!({"discover_on_demand":true}),
            Some(error),
            vec![inventory_request(0, None, false)],
        )
        .await;
    }
    for (second, error) in [
        (
            eager_page(json!([{"name":"x"}]), Value::Null),
            "duplicate MCP tool name",
        ),
        (
            eager_page(json!([]), json!("next")),
            "invalid/repeated inventory cursor",
        ),
    ] {
        inventory_case(
            vec![eager_page(json!([{"name":"x"}]), json!("next")), second],
            json!({"discover_on_demand":true}),
            Some(error),
            vec![
                inventory_request(0, None, false),
                inventory_request(1, Some("next"), false),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn inventory_discovery_errors_share_names_but_preserve_own_validation_order() {
    let first = inventory_request(0, None, false);
    let search = inventory_request(0, None, true);
    for (result, error) in [
        (json!({"isError":true}), "MCP discovery failed"),
        (json!({}), "MCP discovery items missing"),
        (
            discovery_page(json!({}), Value::Null),
            "MCP discovery items missing",
        ),
        (
            discovery_page(json!([{}]), json!(7)),
            "discovered tool missing name",
        ),
        (
            discovery_page(json!([{"tool_id":7}]), Value::Null),
            "discovered tool missing name",
        ),
        (
            discovery_page(json!([{"tool_id":"local"}]), json!(7)),
            "duplicate discovered or eagerly advertised tool",
        ),
        (
            discovery_page(json!([{"tool_id":"api"},{"tool_id":"api"}]), Value::Null),
            "duplicate discovered or eagerly advertised tool",
        ),
        (
            discovery_page(json!([]), json!(false)),
            "invalid/repeated discovery cursor",
        ),
    ] {
        inventory_case(
            vec![json!({"tools":[{"name":"local"}]}), result],
            json!({"discover_on_demand":true}),
            Some(error),
            vec![first.clone(), search.clone()],
        )
        .await;
    }
    for (second, error) in [
        (
            discovery_page(json!([{"tool_id":"api"}]), Value::Null),
            "duplicate discovered or eagerly advertised tool",
        ),
        (
            discovery_page(json!([]), json!("next")),
            "invalid/repeated discovery cursor",
        ),
    ] {
        inventory_case(
            vec![
                json!({"tools":[]}),
                discovery_page(json!([{"tool_id":"api"}]), json!("next")),
                second,
            ],
            json!({"discover_on_demand":true}),
            Some(error),
            vec![
                first.clone(),
                search.clone(),
                inventory_request(1, Some("next"), true),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn inventory_prepared_validation_occurs_after_both_traversals() {
    for (mut expected, error) in [
        (json!({}), "prepared inventory missing tools"),
        (json!({"tools":{}}), "prepared inventory missing tools"),
        (json!({"tools":[{}]}), "prepared tool missing name"),
        (
            json!({"tools":[{"name":"a"},{"name":"a"}]}),
            "duplicate prepared tool name",
        ),
        (
            json!({"tools":[],"management_tools":null}),
            "prepared local tools must be an array",
        ),
        (
            json!({"tools":[],"management_tools":[{}]}),
            "prepared local tool missing name",
        ),
        (
            json!({"tools":[{"name":"a"}],"management_tools":[{"name":"a"}]}),
            "duplicate prepared tool name",
        ),
        (
            json!({"tools":[],"management_tools":[{"name":"a"},{"name":"a"}]}),
            "duplicate prepared tool name",
        ),
        (
            json!({"tools":[{"name":"a","description":"changed"}]}),
            "served MCP inventory differs from pinned catalog",
        ),
        (
            json!({"tools":[{"name":"a","input_schema":{}}]}),
            "served MCP inventory differs from pinned catalog",
        ),
    ] {
        expected["discover_on_demand"] = json!(true);
        inventory_case(
            vec![
                json!({"tools":[]}),
                discovery_page(json!([{"tool_id":"a"}]), Value::Null),
            ],
            expected,
            Some(error),
            vec![
                inventory_request(0, None, false),
                inventory_request(0, None, true),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn inventory_page_bounds_accept_terminal_page_in_each_protocol() {
    for discovery in [false, true] {
        for terminal in [true, false] {
            let mut replies = Vec::new();
            let mut trace = Vec::new();
            if discovery {
                replies.push(json!({"tools":[]}));
                trace.push(inventory_request(0, None, false));
            }
            let pages = if discovery { 100 } else { 400 };
            for page in 0..pages {
                let cursor = if page == pages - 1 && terminal {
                    Value::Null
                } else {
                    json!(format!("page-{page}"))
                };
                replies.push(if discovery {
                    discovery_page(json!([]), cursor)
                } else {
                    eager_page(json!([]), cursor)
                });
                let previous = (page > 0).then(|| format!("page-{}", page - 1));
                trace.push(inventory_request(page, previous.as_deref(), discovery));
            }
            inventory_case(replies,json!({"tools":[],"discover_on_demand":discovery}),if terminal {None}else if discovery {Some("MCP discovered inventory exceeds 100-page limit")}else{Some("MCP inventory exceeds 400-page qualification limit; inventory is incomplete")},trace).await;
        }
    }
}

#[tokio::test]
async fn inventory_tool_bounds_preserve_eager_name_before_limit_and_discovery_limit_before_name() {
    let eager: Vec<_> = (0..10000)
        .map(|n| json!({"name":format!("tool-{n}")}))
        .collect();
    let expected = json!({"tools":eager});
    inventory_case(
        vec![json!({"tools":eager})],
        expected.clone(),
        None,
        vec![inventory_request(0, None, false)],
    )
    .await;
    for (extra, error) in [
        (json!({}), "MCP tool missing name"),
        (
            json!({"name":"tool-0"}),
            "MCP inventory exceeds 10000-tool qualification limit",
        ),
    ] {
        inventory_case(
            vec![
                eager_page(json!(eager), json!("next")),
                json!({"tools":[extra]}),
            ],
            expected.clone(),
            Some(error),
            vec![
                inventory_request(0, None, false),
                inventory_request(1, Some("next"), false),
            ],
        )
        .await;
    }
    let discovered: Vec<_> = (1..10000)
        .map(|n| json!({"tool_id":format!("tool-{n}")}))
        .collect();
    let mut lazy = expected.clone();
    lazy["discover_on_demand"] = json!(true);
    inventory_case(
        vec![
            json!({"tools":[{"name":"tool-0"}]}),
            discovery_page(json!(discovered), Value::Null),
        ],
        lazy.clone(),
        None,
        vec![
            inventory_request(0, None, false),
            inventory_request(0, None, true),
        ],
    )
    .await;
    inventory_case(
        vec![
            json!({"tools":eager}),
            discovery_page(json!([{}]), json!(7)),
        ],
        lazy,
        Some("MCP discovered inventory exceeds 10000 tools"),
        vec![
            inventory_request(0, None, false),
            inventory_request(0, None, true),
        ],
    )
    .await;
}

#[tokio::test]
async fn inventory_rpc_failure_and_cancellation_do_not_start_further_pages() {
    for (reply, error) in [
        (InventoryReply::HttpFailure, "MCP HTTP status 503"),
        (
            InventoryReply::WrongIdentity,
            "MCP response identity/protocol error; request may have executed",
        ),
    ] {
        let fixture = InventoryFixture::new(vec![
            InventoryReply::Result(eager_page(json!([]), json!("next"))),
            reply,
        ])
        .await;
        assert_eq!(
            fixture
                .client
                .check_inventory(&json!({"discover_on_demand":true,"tools":[]}), 10000)
                .await
                .unwrap_err()
                .to_string(),
            error
        );
        assert_eq!(
            fixture.trace(),
            vec![
                inventory_request(0, None, false),
                inventory_request(1, Some("next"), false)
            ]
        );
    }
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let fixture = InventoryFixture::new(vec![
        InventoryReply::Result(eager_page(json!([]), json!("next"))),
        InventoryReply::Stall(entered.clone(), release.clone()),
    ])
    .await;
    let client = fixture.client.clone();
    let task = tokio::spawn(async move {
        client
            .check_inventory(&json!({"discover_on_demand":true,"tools":[]}), 10000)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.notify_one();
    assert_eq!(
        fixture.trace(),
        vec![
            inventory_request(0, None, false),
            inventory_request(1, Some("next"), false)
        ]
    );
}

#[tokio::test]
async fn inventory_response_byte_limit_applies_to_eager_and_discovery_pages() {
    let empty = json!({"jsonrpc":"2.0","id":"qualification-inventory-0","result":{"tools":[]}});
    let limit = serde_json::to_vec(&empty).unwrap().len();
    // Exact eager boundary is accepted, but a larger discovery result must not
    // inherit a different limit from the eager traversal.
    let fixture = InventoryFixture::new(vec![
        InventoryReply::Result(json!({"tools":[]})),
        InventoryReply::Result(discovery_page(json!([]), Value::Null)),
    ])
    .await;
    assert_eq!(
        fixture
            .client
            .check_inventory(&json!({"tools":[],"discover_on_demand":true}), limit)
            .await
            .unwrap_err()
            .to_string(),
        format!(
            "MCP response exceeds result_bytes={limit}; evidence rejected and case remains reserved"
        )
    );
    assert_eq!(
        fixture.trace(),
        vec![
            inventory_request(0, None, false),
            inventory_request(0, None, true)
        ]
    );
    let fixture = InventoryFixture::new(vec![InventoryReply::Result(json!({"tools":[]}))]).await;
    assert_eq!(
        fixture
            .client
            .check_inventory(&json!({"tools":[],"discover_on_demand":true}), limit - 1)
            .await
            .unwrap_err()
            .to_string(),
        format!(
            "MCP response exceeds result_bytes={}; evidence rejected and case remains reserved",
            limit - 1
        )
    );
    assert_eq!(fixture.trace(), vec![inventory_request(0, None, false)]);
}
