#[path = "support/base_failover.rs"]
mod fixture;
use x402_treazury::rotation::base::{Anchor, BaseRpc};

#[tokio::test]
async fn availability_failure_discards_partial_evidence_and_restarts_the_entire_view() {
    let (address, logs, server) = fixture::fixture().await;
    for mode in ["403", "429", "503", "limited"] {
        logs.lock().unwrap().clear();
        let rpc = BaseRpc::with_fallbacks(
            &[
                format!("http://{address}/{mode}"),
                format!("http://{address}/ok"),
            ],
            12,
            120,
        )
        .unwrap();
        let view = rpc.view(fixture::query()).await.unwrap();
        assert!(
            view.balances
                .values()
                .all(|n| *n == alloy_primitives::U256::from(7))
        );
        let calls = logs.lock().unwrap();
        let fallback: Vec<_> = calls
            .iter()
            .filter(|(mode, _)| mode == "ok")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(fallback[0]["method"], "eth_chainId");
        assert_eq!(
            fallback
                .iter()
                .filter(|v| v["method"] == "eth_call")
                .count(),
            4
        );
        assert_eq!(fallback.last().unwrap()["method"], "eth_getBlockByNumber");
    }
    server.abort();
}

#[tokio::test]
async fn invalid_evidence_never_shops_for_a_more_favorable_provider() {
    let (address, logs, server) = fixture::fixture().await;
    for mode in ["wrong-chain", "malformed", "conflict", "401"] {
        logs.lock().unwrap().clear();
        let rpc = BaseRpc::with_fallbacks(
            &[
                format!("http://{address}/{mode}"),
                format!("http://{address}/ok"),
            ],
            12,
            120,
        )
        .unwrap();
        let mut query = fixture::query();
        query.anchor = Some(Anchor {
            height: 88,
            hash: format!("0x{:064x}", 88),
        });
        assert!(rpc.view(query).await.is_err());
        assert!(logs.lock().unwrap().iter().all(|(seen, _)| seen == mode));
    }
    // A fallback must independently honor the previously persisted anchor too.
    let rpc = BaseRpc::with_fallbacks(
        &[
            format!("http://{address}/403"),
            format!("http://{address}/conflict"),
        ],
        12,
        120,
    )
    .unwrap();
    let mut query = fixture::query();
    query.anchor = Some(Anchor {
        height: 88,
        hash: format!("0x{:064x}", 88),
    });
    assert!(rpc.view(query).await.is_err());
    server.abort();
}

#[test]
fn endpoint_lists_are_validated_without_silent_limits() {
    for urls in [
        vec![],
        vec!["http://remote.invalid"],
        vec!["https://one.invalid"; 4],
        vec!["https://one.invalid", "https://one.invalid/"],
    ] {
        assert!(
            BaseRpc::with_fallbacks(
                &urls.into_iter().map(String::from).collect::<Vec<_>>(),
                12,
                120
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn exhaustion_and_cancellation_do_not_loop_or_contact_later_providers() {
    let (address, logs, server) = fixture::fixture().await;
    let urls = ["403", "429", "503"].map(|mode| format!("http://{address}/{mode}"));
    let error = BaseRpc::with_fallbacks(&urls, 12, 120)
        .unwrap()
        .view(fixture::query())
        .await
        .err()
        .unwrap();
    assert!(x402_treazury::rotation::base::safe_diagnostic(&error).contains("HTTP 503"));
    for mode in ["403", "429", "503"] {
        assert_eq!(
            logs.lock()
                .unwrap()
                .iter()
                .filter(|(m, v)| m == mode && v["method"] == "eth_chainId")
                .count(),
            1
        );
    }
    // Completed failover can leave already-dispatched sibling balance requests
    // reaching the old fixture. Give cancellation its own origin and capture.
    server.abort();
    let (address, logs, server) = fixture::fixture().await;
    let rpc = BaseRpc::with_fallbacks(
        &[
            format!("http://{address}/stall"),
            format!("http://{address}/ok"),
        ],
        12,
        120,
    )
    .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rpc.view(fixture::query())
        )
        .await
        .is_err()
    );
    assert!(logs.lock().unwrap().iter().all(|(mode, _)| mode == "stall"));
    server.abort();
}

#[tokio::test]
async fn progressing_complete_view_survives_former_deadline_without_fallback() {
    let (address, logs, server) = fixture::fixture().await;
    let rpc = BaseRpc::with_fallbacks(
        &[
            format!("http://{address}/slow"),
            format!("http://{address}/ok"),
        ],
        12,
        120,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let view = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        rpc.view(fixture::query()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(started.elapsed() > std::time::Duration::from_secs(15));
    assert_eq!(view.balances.len(), 2);
    let calls = logs.lock().unwrap();
    // Chain identity, two latest/confirmed anchor pairs, four wallet balance
    // reads and three canonical anchor rechecks (historical, stable, latest).
    assert_eq!(calls.len(), 12);
    assert!(calls.iter().all(|(mode, _)| mode == "slow"));
    server.abort();
}
