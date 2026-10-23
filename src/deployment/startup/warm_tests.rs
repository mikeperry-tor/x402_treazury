use super::*;
use crate::http_cache::{Entry, Metadata, Slot};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn warm_prefers_fresh_relay_catalog_without_endpoint_requests() {
    let dir = tempfile::tempdir().unwrap();
    let (relay, fixture, task) =
        crate::discovery_relay::tests::fixture(serde_json::json!({}), false, false).await;
    let mut config: MetaConfig = toml::from_str(
        r#"
version=1
[treasury]
state_dir='unused'
max_refund_shielding_fee_zec='0.001'
[wallets.w]
mode='static'
private_key_env='UNUSED'
max_api_payment_usdc='0.01'
[sources.api]
spec='http://127.0.0.1:1/spec'
base_url='http://127.0.0.1:1'
[servers.test]
listen='127.0.0.1:0'
bearer_token_env='UNUSED'
wallet='w'
sources=['api']
"#,
    )
    .unwrap();
    config.treasury.as_mut().unwrap().state_dir = dir.path().to_owned();
    let path = dir.path().join("deployment.toml");
    let mut cfg = crate::config::resolve(config.sources["api"].provider.clone(), &path)
        .await
        .unwrap()
        .settings;
    cfg.discovery_source = Some("api".into());
    cfg.discovery_relay = Some(relay.clone());
    cfg.http_cache_directory = Some(dir.path().join("http-cache"));
    let headers = reqwest::header::HeaderMap::from_iter([(
        reqwest::header::CACHE_CONTROL,
        "max-age=600".parse().unwrap(),
    )]);
    let metadata = Metadata::from_headers(&headers, Duration::ZERO, None).unwrap();
    let catalog = Entry {
        metadata: metadata.clone(),
        data:
            br#"{"operations":[{"method":"GET","path":"/read"},{"method":"GET","path":"/read"}]}"#
                .to_vec(),
    };
    let direct_catalog = Slot::new(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    let relay_catalog =
        crate::discovery_relay::slot(&cfg, &cfg.spec, "catalog", cfg.max_spec_bytes).unwrap();
    direct_catalog.write(Some(catalog.clone())).await;
    relay_catalog.write(Some(catalog.clone())).await;
    let selected = ["api".to_owned()].into_iter().collect();
    let relays = BTreeMap::from([("api".to_owned(), relay)]);
    for relay_catalog_fresh in [true, false] {
        if !relay_catalog_fresh {
            let mut stale = catalog.clone();
            stale.metadata.expires = 0;
            relay_catalog.write(Some(stale)).await;
        }
        let results = warm(&config, &path, &selected, false, relays.clone())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].catalog_cache,
            if relay_catalog_fresh {
                "fresh_via_relay"
            } else {
                "fresh"
            }
        );
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.signed.load(Ordering::SeqCst), 0);
    }
    task.abort();
}
