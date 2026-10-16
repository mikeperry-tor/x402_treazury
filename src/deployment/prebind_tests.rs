use super::*;
use crate::rotation::restriction::FundingRestriction;
use std::path::PathBuf;

fn fixture(dir: &Path, managed: bool) -> PathBuf {
    super::initialization_tests::write_spec(dir);
    let config = if managed {
        super::initialization_tests::managed_config().replace("auto_fund=false", "auto_fund=true")
    } else {
        "version=1\n[wallets.selected]\nmode='static'\nprivate_key_env='KEY'\n[sources.api]\nspec='spec.json'\nbase_url='https://example.com'\nprobe_pricing=false\n[servers.one]\nlisten='127.0.0.1:0'\nbearer_token_env='TOKEN'\nwallet='selected'\nsources=['api']\n".into()
    };
    let provider = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers/curl/provider.toml");
    let path = dir.join("deployment.toml");
    std::fs::write(
        &path,
        format!(
            "{config}\n[discovery_relay]\nprovider='{}'\nwallet='selected'\nserve=true\n",
            provider.display()
        ),
    )
    .unwrap();
    path
}

#[tokio::test]
async fn relay_second_bind_failure_precedes_wallet_open_and_releases_first_socket() {
    for managed in [false, true] {
        for restriction in [
            FundingRestriction::default(),
            FundingRestriction::DenyNewFunding,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = fixture(dir.path(), managed);
            let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = first.local_addr().unwrap();
            drop(first);
            let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = std::fs::read_to_string(&path)
                .unwrap()
                .replace("listen='127.0.0.1:0'", &format!("listen='{address}'"));
            std::fs::write(&path, format!("{config}\n[servers.two]\nlisten='{}'\nbearer_token_env='TOKEN'\nwallet='selected'\nsources=['api']\n", second.local_addr().unwrap())).unwrap();
            // No static key or treasury exists: wallet initialization would fail.
            let error = Deployment::load_for_serving_with_relay(
                &path,
                &super::initialization_tests::env(),
                restriction,
            )
            .await
            .err()
            .unwrap();
            assert!(
                format!("{error:#}").contains("server two: cannot bind"),
                "{error:#}"
            );
            TcpListener::bind(address).await.unwrap();
            assert!(!dir.path().join("state").exists());
            assert!(!dir.path().join("key").exists());
        }
    }
}

#[tokio::test]
async fn relay_binding_reuses_ephemeral_socket_and_setup_failure_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path(), false);
    let mut env = super::initialization_tests::env();
    env.insert("KEY".into(), format!("{:064x}", 1));
    let deployment = Deployment::load_for_serving_with_relay(&path, &env, Default::default())
        .await
        .unwrap();
    let address = deployment.prebound["one"].local_addr().unwrap();
    let running = deployment.bind(&env).await.unwrap();
    assert_eq!(running.addresses(), vec![("one".into(), address)]);
    drop(running);
    TcpListener::bind(address).await.unwrap();

    // A later missing source document must also drop the already reserved socket.
    let config = std::fs::read_to_string(&path)
        .unwrap()
        .replace("listen='127.0.0.1:0'", &format!("listen='{address}'"))
        .replace("spec='spec.json'", "spec='missing.json'");
    std::fs::write(&path, config).unwrap();
    assert!(
        Deployment::load_for_serving_with_relay(&path, &env, Default::default())
            .await
            .is_err()
    );
    TcpListener::bind(address).await.unwrap();
}
