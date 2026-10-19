//! Opt-in private qualification observations; normal serving retains no history.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Private SOCKS credentials and their canonical owner/origin association.
/// Observations describe constructed transports, including unused candidates;
/// they never establish successful routing or grant execution authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeObservation {
    pub canonical: (String, String),
    pub scope: (String, String),
    pub origin: String,
}

type Observer = dyn Fn(&ScopeObservation) -> Result<()> + Send + Sync;
static OBSERVER: OnceLock<Arc<Observer>> = OnceLock::new();

/// Install before constructing participating HTTP clients. Only explicitly
/// enabled qualification tooling should persist these private observations.
pub fn install_http_scope_observer(
    observer: impl Fn(&ScopeObservation) -> Result<()> + Send + Sync + 'static,
) -> Result<()> {
    ensure!(
        OBSERVER.set(Arc::new(observer)).is_ok(),
        "HTTP scope observer already installed"
    );
    Ok(())
}

pub(super) fn observe(observation: impl FnOnce() -> ScopeObservation) -> Result<()> {
    match OBSERVER.get() {
        Some(observer) => observer(&observation()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{
        HttpPolicy, IsolationId, Mode, NetworkContext, NetworkPolicy, SelectionPolicy,
    };
    use std::{collections::BTreeMap, sync::Mutex};

    #[tokio::test]
    async fn private_scope_observation_precedes_io_and_retains_no_normal_history() {
        for mode in ["absent", "record", "failure"] {
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .env_clear()
                .envs(std::env::var("LLVM_PROFILE_FILE").map(|v| ("LLVM_PROFILE_FILE", v)))
                .env("TREAZURY_SCOPE_OBSERVER_CHILD", mode)
                .args([
                    "--ignored",
                    "--exact",
                    "network::scope_observation::tests::scope_observation_child",
                    "--nocapture",
                ])
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[tokio::test]
    #[ignore = "private observer state; invoked by parent"]
    async fn scope_observation_child() {
        let mode = std::env::var("TREAZURY_SCOPE_OBSERVER_CHILD").unwrap();
        let observations = Arc::new(Mutex::new(Vec::<ScopeObservation>::new()));
        if mode != "absent" {
            let saved = observations.clone();
            let fail = mode == "failure";
            install_http_scope_observer(move |observation| {
                saved.lock().unwrap().push(observation.clone());
                ensure!(!fail, "fixture private observation unavailable");
                Ok(())
            })
            .unwrap();
            assert!(install_http_scope_observer(|_| Ok(())).is_err());
        }
        let app = axum::Router::new().fallback(|| async { "private observer fixture" });
        let (address, server) = crate::test_tls::serve(app).await;
        let socks = crate::test_socks::Socks::start(
            BTreeMap::from([("api.example.com".into(), address)]),
            crate::test_socks::Fault::None,
        )
        .await;
        let network = NetworkContext::new(NetworkPolicy {
            mode: Mode::Tor,
            socks_endpoint: Some(socks.address),
            ..Default::default()
        })
        .unwrap()
        .with_test_root(crate::test_tls::CA);
        let identity = IsolationId::evm("0x0000000000000000000000000000000000000001").unwrap();
        let acquire = || {
            network.http_participating(
                &identity,
                "https://api.example.com/private/path?private=query",
                None,
                false,
                HttpPolicy::COMPATIBLE,
            )
        };
        if mode == "failure" {
            for _ in 0..2 {
                let error = acquire().unwrap_err();
                assert!(format!("{error:#}").contains("fixture private observation unavailable"));
                assert_eq!(
                    network.http_cache_len(),
                    0,
                    "failed observation cannot publish an undisclosed transport"
                );
            }
            assert_eq!(observations.lock().unwrap().len(), 2);
        } else {
            let selected = acquire().unwrap();
            let reused = acquire().unwrap();
            assert!(Arc::ptr_eq(&selected, &reused));
            let candidate = network.http_candidate(&selected).unwrap();
            assert!(
                Arc::ptr_eq(&selected, &acquire().unwrap()),
                "private candidate cannot replace selected transport"
            );
            assert!(network.promote_http(&selected, &candidate));
            assert!(Arc::ptr_eq(&candidate, &acquire().unwrap()));
            if mode == "record" {
                let saved = observations.lock().unwrap();
                assert_eq!(
                    saved.len(),
                    2,
                    "cache hits and promotion create no extra scope observations"
                );
                assert_eq!(saved[0].canonical, network.credentials(&identity));
                assert_eq!(saved[1].canonical, saved[0].canonical);
                assert_eq!(saved[0].scope, network.credentials(&selected.scope));
                assert_eq!(saved[1].scope, network.credentials(&candidate.scope));
                assert_ne!(saved[0].scope, saved[1].scope);
                assert_ne!(saved[0].scope, saved[0].canonical);
                assert!(
                    saved
                        .iter()
                        .all(|event| event.origin == "https://api.example.com"),
                    "paths and queries never enter the private origin association"
                );
                let encoded = serde_json::to_vec(&saved[0]).unwrap();
                assert_eq!(
                    serde_json::from_slice::<ScopeObservation>(&encoded).unwrap(),
                    saved[0]
                );
            } else {
                assert!(observations.lock().unwrap().is_empty());
            }
        }
        let before = observations.lock().unwrap().len();
        network
            .http_transport(
                &identity,
                "https://spec.example.com",
                None,
                false,
                HttpPolicy::COMPATIBLE,
                SelectionPolicy::Legacy,
            )
            .unwrap();
        NetworkContext::new(NetworkPolicy::default())
            .unwrap()
            .http_participating(
                &identity,
                "https://api.example.com",
                None,
                false,
                HttpPolicy::COMPATIBLE,
            )
            .unwrap();
        assert_eq!(
            observations.lock().unwrap().len(),
            before,
            "legacy and direct constructors do not emit fresh-scope observations"
        );
        tokio::task::yield_now().await;
        assert_eq!(
            socks.accepted.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "observation and rejection precede all outbound I/O"
        );
        assert!(socks.records.lock().unwrap().is_empty());
        if mode == "record" {
            let winner = acquire().unwrap();
            let observed = observations.lock().unwrap()[1].clone();
            let response = winner
                .client
                .get("https://api.example.com/private/path?private=query")
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            assert_eq!(response.text().await.unwrap(), "private observer fixture");
            {
                let records = socks.records.lock().unwrap();
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].host, "api.example.com");
                assert_eq!(records[0].port, 443);
                assert_eq!(records[0].address_type, 3);
                assert_eq!(
                    (records[0].user.clone(), records[0].password.clone()),
                    observed.scope,
                    "actual SOCKS disclosure uses the scope recorded before I/O"
                );
            }
            assert_eq!(observed.canonical, network.credentials(&identity));
            assert_eq!(observations.lock().unwrap().len(), 2);
        }
        server.abort();
        let _ = server.await;
    }
}
