use super::*;
use crate::rotation::restriction::FundingRestriction;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn managed_config() -> String {
    r#"version=1
[treasury]
state_dir='state'
key_file='key'
indexer_url_env='INDEXER'
submission_url_env='SUBMIT'
max_refund_shielding_fee_zec='0.001'
[funding]
auto_fund=false
base_rpc_url_env='BASE'
base_rpc_fallback_url_envs=[]
[wallets.selected]
mode='zcash_rotation'
max_api_payment_usdc='5'
reserves={small={funding_amount_usdc='5'}}
max_funding_spend_zec='0.02'
max_conversion_overhead_percent=5
[wallets.unselected]
mode='zcash_rotation'
max_api_payment_usdc='5'
reserves={small={funding_amount_usdc='5'}}
max_funding_spend_zec='0.02'
max_conversion_overhead_percent=5
[sources.api]
spec='spec.json'
base_url='http://127.0.0.1:1'
probe_pricing=false
[servers.one]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='selected'
sources=['api']
"#
    .into()
}
fn env() -> BTreeMap<String, String> {
    [
        ("TOKEN", "fixture-token"),
        ("INDEXER", "http://127.0.0.1:1"),
        ("SUBMIT", "http://127.0.0.1:1"),
        ("BASE", "http://127.0.0.1:1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}
fn write_spec(dir: &Path) {
    std::fs::write(
        dir.join("spec.json"),
        r#"{"operations":[{"method":"GET","path":"/read"}]}"#,
    )
    .unwrap();
}

async fn reopen(state: &Path, key: &Path, id: &str) -> crate::treasury::Treasury {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match crate::treasury::Treasury::open(state.to_owned(), key.to_owned(), id.to_owned())
                .await
            {
                Ok(owner) => return owner,
                Err(error) if error.to_string().contains("state_in_use") => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("unexpected reopen failure: {error:#}"),
            }
        }
    })
    .await
    .expect("initialization retained treasury ownership")
}

#[tokio::test]
async fn serving_preflight_rejects_credentials_and_providers_before_state_or_requests() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/spec", listener.local_addr().unwrap());
    let app = axum::Router::new().fallback(move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            "unexpected request"
        }
    });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for missing_token in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        let config = managed_config().replace("spec='spec.json'", &format!("spec='{url}'"));
        // Lexically later invalid provider must also fail in preflight, before
        // the valid remote source has a chance to perform catalog I/O.
        let config = format!("{config}\n[sources.z_invalid]\nextends='missing-provider.toml'\n");
        std::fs::write(&path, config).unwrap();
        let mut env = env();
        if missing_token {
            env.remove("TOKEN");
        }
        let error = match Deployment::load_with_warnings(
            &path,
            true,
            Some((&env, FundingRestriction::Ordinary)),
        )
        .await
        {
            Ok(_) => panic!("invalid preflight accepted"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            error.contains(if missing_token {
                "TOKEN"
            } else {
                "No such file or directory"
            }),
            "{error}"
        );
        assert!(!dir.path().join("state").exists());
        assert!(!dir.path().join("key").exists());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    task.abort();
}

#[tokio::test]
async fn selected_static_initialization_intersects_effective_dynamic_and_relay_wallets() {
    let dir = tempfile::tempdir().unwrap();
    write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    let mut config = String::from("version=1\n[source_management]\nwallet='dynamic'\n");
    for name in ["overridden", "effective", "dynamic", "relay", "unused"] {
        config.push_str(&format!(
            "[wallets.{name}]\nmode='static'\nprivate_key_env='{name}'\n"
        ));
    }
    config.push_str(
        r#"
[sources.api]
spec='spec.json'
base_url='http://127.0.0.1:1'
wallet='effective'
probe_pricing=false
[servers.api]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
wallet='overridden'
sources=['api']
[servers.dynamic]
listen='127.0.0.1:0'
bearer_token_env='TOKEN'
source_management=true
sources=[]
"#,
    );
    std::fs::write(&path, config).unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    let relays = BTreeSet::from(["relay".to_owned()]);
    for names in [
        vec!["effective", "dynamic", "relay"],
        vec!["dynamic", "relay"],
        vec!["unused"],
    ] {
        let only: BTreeSet<_> = names.iter().map(|n| (*n).to_owned()).collect();
        let env: BTreeMap<_, _> = names
            .iter()
            .filter(|n| **n != "unused")
            .map(|n| ((*n).to_owned(), format!("{:064x}", 1)))
            .collect();
        let initialized = deployment
            .initialize_wallets(
                &env,
                FundingRestriction::DenyNewFunding,
                Some(&only),
                &relays,
            )
            .await
            .unwrap();
        let expected: BTreeSet<_> = only.into_iter().filter(|n| n != "unused").collect();
        assert_eq!(
            initialized.wallets.keys().cloned().collect::<BTreeSet<_>>(),
            expected
        );
        assert!(initialized.treasury.is_none());
        assert!(initialized.funding_runtime.is_none());
        assert!(initialized.managed_pools.is_empty());
        initialized.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn managed_initialization_restricts_before_selected_pool_setup_and_releases_owner() {
    for (auto_fund, restriction) in [
        (false, FundingRestriction::Ordinary),
        (true, FundingRestriction::DenyNewFunding),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let key = dir.path().join("key");
        let owner = crate::treasury::Treasury::create(
            state.clone(),
            key.clone(),
            2_000_000,
            Some(zeroize::Zeroizing::new(SEED.into())),
        )
        .await
        .unwrap();
        let id = owner.status().await.unwrap().treasury_id;
        owner.close().await.unwrap();
        write_spec(dir.path());
        let path = dir.path().join("deployment.toml");
        std::fs::write(
            &path,
            managed_config().replace("auto_fund=false", &format!("auto_fund={auto_fund}")),
        )
        .unwrap();
        let deployment = Deployment::load(&path).await.unwrap();
        let only = BTreeSet::from(["selected".to_owned()]);
        for _ in 0..2 {
            let initialized = deployment
                .initialize_wallets(&env(), restriction, Some(&only), &BTreeSet::new())
                .await
                .unwrap();
            assert_eq!(
                initialized.wallets.keys().cloned().collect::<BTreeSet<_>>(),
                only
            );
            assert_eq!(initialized.managed_pools.len(), 1);
            assert!(initialized.funding_runtime.is_some());
            assert!(
                initialized.supervisor.is_none(),
                "initialization does not start workers"
            );
            let status = initialized
                .treasury
                .as_ref()
                .unwrap()
                .status()
                .await
                .unwrap();
            assert_eq!(status.pools.len(), 1);
            assert_eq!(status.pools[0].name, "selected");
            assert!(
                status.pools[0].addresses.is_empty(),
                "restriction precedes reserve allocation"
            );
            assert!(status.funding_jobs.is_empty());
            initialized.shutdown().await.unwrap();
            let reopened = reopen(&state, &key, &id).await;
            assert_eq!(reopened.status().await.unwrap().pools[0].name, "selected");
            reopened.close().await.unwrap();
        }
    }
}

#[tokio::test]
async fn relay_setup_failure_after_initialization_releases_treasury_without_allocations() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let key = dir.path().join("key");
    let owner = crate::treasury::Treasury::create(
        state.clone(),
        key.clone(),
        2_000_000,
        Some(zeroize::Zeroizing::new(SEED.into())),
    )
    .await
    .unwrap();
    let id = owner.status().await.unwrap().treasury_id;
    owner.close().await.unwrap();
    write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        format!(
            "{}\n[discovery_relay]\nprovider='missing-relay.toml'\nwallet='selected'\nserve=true\n",
            managed_config()
        ),
    )
    .unwrap();
    let error = match Deployment::load_with_warnings(
        &path,
        true,
        Some((&env(), FundingRestriction::DenyNewFunding)),
    )
    .await
    {
        Ok(_) => panic!("missing relay provider accepted"),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("No such file or directory"), "{error}");
    // An unstarted serialized store closes asynchronously after owner drop.
    let owner = reopen(&state, &key, &id).await;
    let status = owner.status().await.unwrap();
    assert_eq!(
        status
            .pools
            .iter()
            .map(|p| p.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["selected", "unselected"])
    );
    assert!(status.pools.iter().all(|p| p.addresses.is_empty()));
    assert!(status.funding_jobs.is_empty());
    owner.close().await.unwrap();
}

#[tokio::test]
async fn managed_connection_preflight_fails_before_opening_missing_state() {
    let dir = tempfile::tempdir().unwrap();
    write_spec(dir.path());
    let path = dir.path().join("deployment.toml");
    std::fs::write(
        &path,
        managed_config().replace("[funding]", "[funding]\nnear_api_key_env='NEAR'"),
    )
    .unwrap();
    let deployment = Deployment::load(&path).await.unwrap();
    for missing in ["INDEXER", "SUBMIT", "BASE", "NEAR"] {
        let mut env = env();
        env.insert("NEAR".into(), "unfunded-test-key".into());
        env.remove(missing);
        let error = match deployment
            .initialize_wallets(&env, FundingRestriction::Ordinary, None, &BTreeSet::new())
            .await
        {
            Ok(_) => panic!("missing {missing} accepted"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            error.contains(if missing == "BASE" {
                "Base RPC"
            } else {
                missing
            }),
            "{missing}: {error}"
        );
        assert!(!dir.path().join("state").exists());
        assert!(!dir.path().join("key").exists());
    }
}
