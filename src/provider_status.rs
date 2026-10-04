//! Operator-authored observations, independent of OpenAPI selection tags.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ReliabilityTag {
    SlowPricing,
    UpstreamTimeout,
    UpstreamRateLimited,
    IntermittentResponseBody,
    #[serde(rename = "catalog_http_403")]
    CatalogHttp403,
    IntermittentCatalogConnection,
    IntermittentHelpConnection,
    DegradedUpstreamData,
    QuestionableResultRelevance,
}
impl ReliabilityTag {
    fn warning(self) -> (&'static str, &'static str) {
        match self {
            Self::SlowPricing => (
                "slow_pricing",
                "Unsigned pricing discovery has shown long response waits and may delay startup. This tag does not describe catalog or paid-call performance.",
            ),
            Self::UpstreamTimeout => (
                "upstream_timeout",
                "Provider has returned upstream timeout errors, including inside successful HTTP responses.",
            ),
            Self::UpstreamRateLimited => (
                "upstream_rate_limited",
                "Provider has reported rate limiting at an upstream dependency; paid tool delivery may fail.",
            ),
            Self::IntermittentResponseBody => (
                "intermittent_response_body",
                "Response delivery has failed after payment; a failed download may still be charged. Do not automatically replay paid calls.",
            ),
            Self::CatalogHttp403 => (
                "catalog_http_403",
                "Catalog retrieval has repeatedly returned HTTP 403; live catalog startup may fail. This does not establish a provider-wide or Tor-specific block.",
            ),
            Self::IntermittentCatalogConnection => (
                "intermittent_catalog_connection",
                "Catalog connections have intermittently timed out; live catalog startup may fail.",
            ),
            Self::IntermittentHelpConnection => (
                "intermittent_help_connection",
                "Help retrieval has intermittently timed out even when API calls worked.",
            ),
            Self::DegradedUpstreamData => (
                "degraded_upstream_data",
                "Successful API responses have contained missing or failed upstream inputs; inspect completeness markers.",
            ),
            Self::QuestionableResultRelevance => (
                "questionable_result_relevance",
                "Returned result metadata has not matched the requested query; verify relevance before relying on results.",
            ),
        }
    }
}

pub fn warn(source: &str, cfg: &crate::catalog::Config) {
    for tag in cfg
        .reliability_tags
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let (tag, warning) = tag.warning();
        tracing::warn!(
            source,
            reliability_tag = tag,
            evidence = %cfg.reliability_note,
            "Provider reliability observation: {warning}"
        );
    }
}
