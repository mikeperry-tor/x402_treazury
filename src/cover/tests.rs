use super::*;
use rand::{SeedableRng, rngs::StdRng};
fn dist(text: &str) -> Distribution {
    toml::from_str(text).unwrap()
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
