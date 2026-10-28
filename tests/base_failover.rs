#[path = "support/base_failover.rs"]
mod fixture;
use x402_treazury::rotation::base::BaseRpc;

#[tokio::test]
async fn availability_failure_discards_partial_evidence_and_restarts_the_entire_view() {
    let (address, logs, server) = fixture::fixture().await;
    for mode in [
        "403",
        "429",
        "503",
        "limited",
        "malformed",
        "malformed-chain",
        "oversized",
        "conflict", // One hash at distinct heights is a malformed provider tuple.
    ] {
        logs.lock().unwrap().clear();
        let rpc = BaseRpc::with_fallbacks(
            &[
                format!("http://{address}/{mode}"),
                format!("http://{address}/ok"),
            ],
            12,
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
async fn identity_and_authorization_failures_do_not_use_a_fallback() {
    let (address, logs, server) = fixture::fixture().await;
    for mode in ["wrong-chain", "401"] {
        logs.lock().unwrap().clear();
        let rpc = BaseRpc::with_fallbacks(
            &[
                format!("http://{address}/{mode}"),
                format!("http://{address}/ok"),
            ],
            12,
        )
        .unwrap();
        assert!(rpc.view(fixture::query()).await.is_err());
        assert!(logs.lock().unwrap().iter().all(|(seen, _)| seen == mode));
    }
    // A configured fallback must supply its own internally coherent headers.
    let rpc = BaseRpc::with_fallbacks(
        &[
            format!("http://{address}/403"),
            format!("http://{address}/conflict"),
        ],
        12,
    )
    .unwrap();
    assert!(rpc.view(fixture::query()).await.is_err());
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
            BaseRpc::with_fallbacks(&urls.into_iter().map(String::from).collect::<Vec<_>>(), 12)
                .is_err()
        );
    }
}

#[tokio::test]
async fn exhaustion_and_cancellation_do_not_loop_or_contact_later_providers() {
    let (address, logs, server) = fixture::fixture().await;
    let urls = ["403", "429", "503"].map(|mode| format!("http://{address}/{mode}"));
    let error = BaseRpc::with_fallbacks(&urls, 12)
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
    // Chain identity, latest/finalized/confirmed headers, four wallet balance reads,
    // and two distinct post-state canonical anchor rechecks.
    assert_eq!(calls.len(), 10);
    assert!(calls.iter().all(|(mode, _)| mode == "slow"));
    server.abort();
}

#[tokio::test]
async fn exact_balance_evidence_preserves_both_anchors_amounts_and_original_lifetime() {
    use alloy_primitives::U256;
    let (address, _, server) = fixture::fixture().await;
    let view = BaseRpc::new(&format!("http://{address}/exact"), 12)
        .unwrap()
        .view(fixture::query())
        .await
        .unwrap();
    assert_eq!(view.observations.len(), 2);
    for (wallet, observation) in &view.observations {
        assert_eq!(&observation.wallet, wallet);
        assert_eq!(observation.confirmed.anchor.height, 88);
        assert_eq!(observation.latest.anchor.height, 100);
        assert_eq!(observation.confirmed.amount, U256::from(9));
        assert_eq!(observation.latest.amount, U256::from(4));
        assert_eq!(observation.available(), U256::from(4));
        assert_eq!(observation.remainder(), U256::from(9));
        assert_eq!(view.balances[wallet], observation.available());

        assert_eq!(observation.endpoint.len(), 64);
        assert!(!observation.endpoint.contains(&address.to_string()));
    }
    server.abort();
}

#[tokio::test]
async fn history_uses_its_original_proof_without_requiring_a_fresh_balance_tip() {
    use x402_treazury::rotation::base::{PendingAuthorization, now};
    let (address, logs, server) = fixture::fixture().await;
    let mut query = fixture::query();
    let (wallet, payer) = query.wallets.remove(0);
    query.wallets.clear();
    query.pending.push(PendingAuthorization {
        id: "history".into(),
        wallet,
        payer,
        nonce: format!("0x{:064x}", 1),
        valid_before: now().unwrap() - 2000,
    });
    let view = BaseRpc::new(&format!("http://{address}/history-stale"), 12)
        .unwrap()
        .view(query.clone())
        .await
        .unwrap();
    assert_eq!(view.released, vec!["history"]);
    assert!(view.observations.is_empty());
    assert_eq!(
        logs.lock()
            .unwrap()
            .iter()
            .filter(|(_, call)| call["method"] == "eth_getBlockByNumber"
                && call["params"][0] == "latest")
            .count(),
        1
    );
    logs.lock().unwrap().clear();
    let rpc = BaseRpc::with_fallbacks(
        &[
            format!("http://{address}/malformed-nonce"),
            format!("http://{address}/ok"),
        ],
        12,
    )
    .unwrap();
    assert_eq!(rpc.view(query).await.unwrap().released, vec!["history"]);
    assert!(logs.lock().unwrap().iter().any(|(mode, _)| mode == "ok"));
    server.abort();
}
