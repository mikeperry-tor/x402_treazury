use super::*;
use rand::{SeedableRng, rngs::StdRng};
fn dist(text: &str) -> Distribution {
    toml::from_str(text).unwrap()
}
#[test]
fn deadline_completion_requires_received_bytes_not_reserved_capacity() {
    let mut config = example_config();
    config.volume = dist("distribution='uniform'\nmin_bytes=1024\nmax_bytes=1024");
    config.start_delay = dist("distribution='uniform'\nmin_ms=0\nmax_ms=0");
    let now = tokio::time::Instant::now();
    let mut episode = episode::Episode::new(&config, now, &mut StdRng::seed_from_u64(1)).unwrap();
    let reserved = episode.reserve(&config, now, 1024).unwrap();
    assert_eq!(episode.reserved, episode.target);
    assert_eq!(episode.deadline_reason(), "cover_budget_unspent");
    episode.finish(reserved, 512, true);
    assert_eq!(episode.deadline_reason(), "cover_budget_unspent");
    let reserved = episode.reserve(&config, now, 1024).unwrap();
    assert_eq!(reserved, 512);
    assert_eq!(episode.deadline_reason(), "cover_budget_unspent");
    episode.finish(reserved, 512, true);
    assert_eq!(episode.deadline_reason(), "cover_budget_completed");
}
#[test]
fn configured_example_is_valid() {
    let config = example_config();
    config.validate().unwrap();
    let text = include_str!("../../docs/cover-traffic.md")
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let table: toml::Table = toml::from_str(text).unwrap();
    let network: crate::network::NetworkPolicy = table["network"].clone().try_into().unwrap();
    network.validate().unwrap();
    assert!(
        config
            .validate_origin("https://api.example.com/v1/paid")
            .is_ok()
    );
    assert!(
        config
            .validate_origin("https://cdn.example.com/openapi.json")
            .is_err()
    );
}
#[test]
fn padding_only_needs_an_active_request_kind_and_agents_cannot_opt_in() {
    let mut config = example_config();
    config.ranges_enabled = false;
    config.validate().unwrap();
    config.padding.as_mut().unwrap().on_api_requests = false;
    config.padding.as_mut().unwrap().on_cover_requests = true;
    assert!(config.validate().is_err());
    let mut candidate =
        serde_json::json!({"name":"test","spec_url":"https://api.example.com/openapi.json"});
    assert!(serde_json::from_value::<crate::discovery::Candidate>(candidate.clone()).is_ok());
    candidate["cover_traffic"] = serde_json::to_value(config).unwrap();
    assert!(serde_json::from_value::<crate::discovery::Candidate>(candidate).is_err());
    assert!(
        serde_json::from_value::<crate::discovery::Selection>(
            serde_json::json!({"cover_traffic":{}})
        )
        .is_err()
    );
}
#[test]
fn sampled_families_are_bounded_and_non_degenerate() {
    for extra in [
        "distribution='uniform'",
        "distribution='exponential'\nmean_bytes=40.0",
        "distribution='weibull'\nscale_bytes=40.0\nshape=0.8",
        "distribution='log_normal'\nmedian_bytes=40.0\nsigma=0.7",
        "distribution='weighted_discrete'\nvalues_bytes=[10,50,100]\nweights=[1,2,1]",
    ] {
        let d = dist(&format!("{extra}\nmin_bytes=10\nmax_bytes=100"));
        d.validate(Unit::Bytes, false).unwrap();
        let mut rng = StdRng::seed_from_u64(15);
        let mut samples = Vec::new();
        for _ in 0..10000 {
            let x = d.sample(Unit::Bytes, &mut rng).unwrap();
            assert!((10..=100).contains(&x));
            samples.push(x);
        }
        assert!(samples.iter().any(|x| *x < 30));
        assert!(samples.iter().any(|x| *x > 70));
        if extra.contains("weighted_discrete") {
            let n = samples.iter().filter(|x| **x == 50).count();
            assert!((4800..5200).contains(&n));
        }
    }
}
#[test]
fn exponential_and_weibull_shape_one_have_identical_quantiles() {
    let a = dist("distribution='exponential'\nmean_ms=100.0\nmin_ms=5\nmax_ms=500");
    let b = dist("distribution='weibull'\nscale_ms=100.0\nshape=1.0\nmin_ms=5\nmax_ms=500");
    let mut r1 = StdRng::seed_from_u64(99);
    let mut r2 = r1.clone();
    for _ in 0..1000 {
        assert_eq!(
            a.sample(Unit::Milliseconds, &mut r1).unwrap(),
            b.sample(Unit::Milliseconds, &mut r2).unwrap()
        );
    }
}
#[test]
fn invalid_parameters_and_sampling_exhaustion_are_explicit() {
    for text in [
        "distribution='uniform'\nmean_bytes=1.0\nmin_bytes=1\nmax_bytes=2",
        "distribution='exponential'\nmean_bytes=nan\nmin_bytes=1\nmax_bytes=2",
        "distribution='uniform'\nmin_bytes=0\nmax_bytes=9007199254740992",
        "distribution='weighted_discrete'\nmin_bytes=1\nmax_bytes=2\nvalues_bytes=[1,1]\nweights=[1,2]",
    ] {
        assert!(dist(text).validate(Unit::Bytes, false).is_err());
    }
    let d =
        dist("distribution='log_normal'\nmin_bytes=1\nmax_bytes=2\nmedian_bytes=1e100\nsigma=0.01");
    assert!(
        d.sample(Unit::Bytes, &mut StdRng::seed_from_u64(1))
            .unwrap_err()
            .to_string()
            .contains("sampling_exhausted")
    );
    let d = dist("distribution='uniform'\nmin_ms=0\nmax_ms=0");
    assert_eq!(
        d.sample(Unit::Milliseconds, &mut StdRng::seed_from_u64(1))
            .unwrap(),
        0
    );
    assert!(d.validate(Unit::Bytes, true).is_err());
}

pub(super) fn example_config() -> Config {
    let plan = include_str!("../../docs/cover-traffic.md");
    let text = plan
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let table: toml::Table = toml::from_str(text).unwrap();
    let config: Config = table["cover_traffic"].clone().try_into().unwrap();
    config
}

#[test]
fn tor_default_and_explicit_network_overrides() {
    use crate::network::{NetworkContext, NetworkPolicy};
    for (mode, override_value, expected) in [
        ("direct", "", false),
        ("tor", "", true),
        ("tor", "cover_traffic_enabled=false", false),
        ("direct", "cover_traffic_enabled=true", true),
    ] {
        let socks = if mode == "tor" {
            "socks_endpoint='127.0.0.1:9150'"
        } else {
            ""
        };
        let p: NetworkPolicy =
            toml::from_str(&format!("mode='{mode}'\n{socks}\n{override_value}")).unwrap();
        assert_eq!(p.cover_enabled(), expected);
        assert_eq!(p.inspection()["cover_traffic_enabled"], expected);
        assert_eq!(NetworkContext::new(p).unwrap().cover.is_some(), expected);
    }
}

#[test]
fn automatic_catalog_cover_and_provider_opt_out() {
    let base = "https://api.example.com/v1";
    let mut cfg = crate::catalog::Config {
        spec: "https://api.example.com/openapi.json".into(),
        ..Default::default()
    };
    cfg.resolve_cover(base, true).unwrap();
    let profile = cfg.cover_traffic.as_ref().unwrap();
    profile.validate().unwrap();
    assert_eq!(profile.url, cfg.spec);
    assert_eq!(profile.concurrency, 1);
    assert!(profile.padding.is_none());
    cfg.cover_traffic_enabled = Some(false);
    cfg.resolve_cover(base, true).unwrap();
    assert!(cfg.cover_traffic.is_none());
    cfg.cover_traffic_enabled = Some(true);
    cfg.resolve_cover(base, false).unwrap();
    assert!(cfg.cover_traffic.is_none()); // Provider cannot override global off.
    for spec in [
        "local.json",
        "https://cdn.example.com/openapi.json",
        "http://api.example.com/openapi.json",
    ] {
        cfg.spec = spec.into();
        cfg.resolve_cover(base, true).unwrap();
        assert!(cfg.cover_traffic.is_none());
    }
    cfg.cover_traffic = Some(example_config());
    cfg.resolve_cover(base, true).unwrap();
    assert!(cfg.cover_traffic.as_ref().unwrap().padding.is_some());
    cfg.cover_traffic_enabled = Some(false);
    cfg.cover_traffic.as_mut().unwrap().concurrency = 0;
    assert!(cfg.resolve_cover(base, true).is_err());
}

#[tokio::test]
async fn source_can_disable_inherited_cover_without_replacing_profile() {
    let dir = tempfile::tempdir().unwrap();
    let provider = crate::catalog::Config {
        spec: "https://api.example.com/openapi.json".into(),
        cover_traffic: Some(example_config()),
        ..Default::default()
    };
    std::fs::write(
        dir.path().join("provider.toml"),
        toml::to_string(&provider).unwrap(),
    )
    .unwrap();
    let source = dir.path().join("source.toml");
    std::fs::write(
        &source,
        "extends='provider.toml'\ncover_traffic_enabled=false",
    )
    .unwrap();
    let mut cfg = crate::config::load(&source).await.unwrap().settings;
    assert!(cfg.cover_traffic.is_some());
    assert_eq!(cfg.cover_traffic_enabled, Some(false));
    cfg.resolve_cover("https://api.example.com", true).unwrap();
    assert!(cfg.cover_traffic.is_none());
}

#[test]
fn fallback_is_same_origin_and_automatic_help_is_reviewed() {
    let mut cfg = crate::catalog::Config {
        spec: "https://api.example.com/openapi.json".into(),
        help_url: Some("https://api.example.com/llms.txt".into()),
        ..Default::default()
    };
    cfg.resolve_cover("https://api.example.com/v1", true)
        .unwrap();
    let cover = cfg.cover_traffic.as_mut().unwrap();
    assert_eq!(
        cover.fallback_url.as_deref(),
        Some("https://api.example.com/llms.txt")
    );
    cover.validate().unwrap();
    for url in [
        "https://other.example.com/llms.txt",
        "http://api.example.com/llms.txt",
        "https://user@api.example.com/llms.txt",
        "https://api.example.com/llms.txt#fragment",
    ] {
        cover.fallback_url = Some(url.into());
        assert!(cover.validate().is_err());
    }
    cfg.cover_traffic = None;
    cfg.spec = "local.json".into();
    cfg.resolve_cover("https://api.example.com", true).unwrap();
    assert_eq!(
        cfg.cover_traffic.as_ref().unwrap().url,
        "https://api.example.com/llms.txt"
    );
    assert!(cfg.cover_traffic.as_ref().unwrap().fallback_url.is_none());
}
