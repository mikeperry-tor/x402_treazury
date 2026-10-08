//! Numeric discovery evidence; no request URLs, challenge prose or payment authority.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Discovered,
    /// Successful HTTP response without a 402 challenge; price is unknown, not free.
    NoPaymentChallenge,
    UnexpectedHttpStatus,
    MissingHeader,
    MalformedChallenge,
    UnusableOffer,
    HttpConnect,
    HttpTimeout,
    HttpTransport,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePath {
    Initialized,
    /// Process initializer reused a fresh persisted estimate; no HTTP observation.
    Disk,
    Hit,
    Shared,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub enabled: bool,
    pub selected_tools: usize,
    pub skipped_help: usize,
    pub skipped_method: usize,
    pub skipped_template: usize,
    pub skipped_embedded: usize,
    pub skipped_duplicate: usize,
    pub eligible: usize,
    pub capped: usize,
    pub observed: usize,
    pub available_prices: usize,
    pub expired: usize,
    pub cache: BTreeMap<CachePath, usize>,
    pub outcomes: BTreeMap<Outcome, usize>,
    pub http_statuses: BTreeMap<u16, usize>,
}
impl Evidence {
    pub fn validate(&self) -> Result<()> {
        let sum = |v: Vec<usize>| -> Result<usize> {
            v.into_iter().try_fold(0usize, |a, b| {
                a.checked_add(b)
                    .ok_or_else(|| anyhow::anyhow!("pricing evidence counter overflow"))
            })
        };
        ensure!(
            sum(self.cache.values().copied().collect())? == self.observed
                && sum(self.outcomes.values().copied().collect())? == self.observed,
            "pricing observation totals disagree"
        );
        ensure!(
            self.observed.checked_add(self.capped) == Some(self.eligible)
                && self.available_prices <= self.observed
                && self.expired <= self.observed,
            "pricing eligibility/price totals disagree"
        );
        ensure!(
            self.available_prices <= *self.outcomes.get(&Outcome::Discovered).unwrap_or(&0),
            "available price lacks successful discovery"
        );
        ensure!(
            self.available_prices <= self.observed - self.expired,
            "expired prices cannot be available"
        );
        let count = |outcome| *self.outcomes.get(&outcome).unwrap_or(&0);
        let transport = sum(vec![
            count(Outcome::HttpConnect),
            count(Outcome::HttpTimeout),
            count(Outcome::HttpTransport),
        ])?;
        let headers = sum(vec![
            count(Outcome::Discovered),
            count(Outcome::MissingHeader),
            count(Outcome::MalformedChallenge),
            count(Outcome::UnusableOffer),
        ])?;
        let statuses = sum(self.http_statuses.values().copied().collect())?;
        ensure!(
            statuses.checked_add(transport) == Some(self.observed)
                && *self.http_statuses.get(&402).unwrap_or(&0) == headers
                && sum(vec![
                    headers,
                    count(Outcome::UnexpectedHttpStatus),
                    count(Outcome::NoPaymentChallenge),
                ])? == statuses
                && count(Outcome::NoPaymentChallenge)
                    <= sum(self
                        .http_statuses
                        .iter()
                        .filter(|(status, _)| (200..300).contains(*status))
                        .map(|(_, count)| *count)
                        .collect())?,
            "pricing HTTP outcomes disagree with status counts"
        );
        ensure!(
            self.http_statuses.keys().all(|s| (100..=999).contains(s))
                && sum(self.http_statuses.values().copied().collect())? <= self.observed,
            "invalid pricing HTTP status totals"
        );
        let classified = sum(vec![
            self.skipped_help,
            self.skipped_method,
            self.skipped_template,
            self.skipped_embedded,
            self.skipped_duplicate,
            self.eligible,
        ])?;
        ensure!(
            if self.enabled {
                classified == self.selected_tools
            } else {
                classified == 0 && self.observed == 0
            },
            "pricing selected-tool accounting mismatch"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub method: String,
    pub path: String,
    pub line: String,
}
pub fn price_map(rows: &[Price]) -> Result<BTreeMap<(String, String), String>> {
    let mut map = BTreeMap::new();
    for row in rows {
        ensure!(
            row.method == "GET" && !row.path.is_empty() && !row.line.is_empty(),
            "invalid recorded discovery price"
        );
        ensure!(
            map.insert((row.method.clone(), row.path.clone()), row.line.clone())
                .is_none(),
            "duplicate recorded discovery price"
        );
    }
    Ok(map)
}
