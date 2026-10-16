//! Display-only conversion of structured credit tariffs. Never payment authority.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreditPricing {
    pub credit_cost_key: String,
    /// Explicit decimal USDC estimate per credit, with at most six decimal places.
    pub usdc_per_credit: String,
}
impl CreditPricing {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.credit_cost_key.trim().is_empty(),
            "credit_pricing.credit_cost_key must not be empty"
        );
        self.rate()?;
        Ok(())
    }
    fn rate(&self) -> Result<u64> {
        let (whole, fraction) = self
            .usdc_per_credit
            .split_once('.')
            .unwrap_or((&self.usdc_per_credit, ""));
        ensure!(
            !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && fraction.len() <= 6
                && fraction.bytes().all(|b| b.is_ascii_digit()),
            "credit_pricing.usdc_per_credit must be a positive decimal with at most six decimal places"
        );
        let units = whole
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(1_000_000))
            .and_then(|n| n.checked_add(format!("{fraction:0<6}").parse().ok()?))
            .filter(|n| *n > 0)
            .context("credit_pricing.usdc_per_credit is zero or out of range")?;
        Ok(units)
    }
    pub(super) fn describe(&self, metadata: &Value) -> (String, Option<&'static str>) {
        let rate = self.rate().expect("validated credit pricing");
        match self.estimate(metadata, rate) {
            Ok(line) => (format!("Estimated cost: {line} [conf]."), None),
            Err(reason) => (
                "Per-call USDC estimate unavailable [conf].".into(),
                Some(reason),
            ),
        }
    }
    fn estimate(&self, metadata: &Value, rate: u64) -> std::result::Result<String, &'static str> {
        if metadata.is_null() {
            return Err("missing_metadata");
        }
        let p: Tariff = serde_json::from_value(metadata.clone())
            .map_err(|_| "malformed_or_unsupported_schema")?;
        if p.version != 1 {
            return Err("unsupported_version");
        }
        if p.max_credits < p.base_credits || p.normalization_failure_credits > p.max_credits {
            return Err("inconsistent_credit_bounds");
        }
        if p.batch.is_some() && p.metered.is_some() {
            return Err("conflicting_billing_modes");
        }
        if p.settlement
            .as_deref()
            .is_some_and(|s| s != "actual_credits")
        {
            return Err("unsupported_settlement");
        }
        let mut line = if let Some(m) = &p.metered {
            m.estimate(p.base_credits, p.max_credits, rate)
                .ok_or("invalid_or_unsupported_metered_tariff")?
        } else if let Some(b) = &p.batch {
            b.estimate(p.base_credits, p.max_credits, rate)
                .ok_or("invalid_or_unsupported_batch_tariff")?
        } else if p.base_credits == p.max_credits && p.surcharges.is_empty() {
            format!("{}/request", cost(p.base_credits, rate))
        } else {
            format!(
                "{} base; maximum {}/request",
                cost(p.base_credits, rate),
                cost(p.max_credits, rate)
            )
        };
        p.append_charges(&mut line, rate)
            .ok_or("invalid_or_unsupported_surcharge")?;
        Ok(line)
    }
}
fn cost(credits: u64, rate: u64) -> String {
    format!("{} USDC", amount(credits as u128 * rate as u128))
}

fn amount(units: u128) -> String {
    let text = format!("{}.{:06}", units / 1_000_000, units % 1_000_000);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tariff {
    version: u64,
    base_credits: u64,
    max_credits: u64,
    normalization_failure_credits: u64,
    surcharges: Vec<Surcharge>,
    batch: Option<Batch>,
    metered: Option<Metered>,
    settlement: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Batch {
    unit: String,
    max_units: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metered {
    charged_units: String,
    credits_per_unit: u64,
    default_units: u64,
    max_units: u64,
    query_param: String,
    unit: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Surcharge {
    credits: u64,
    label: String,
    query_param: String,
    when: String,
    #[serde(rename = "badge")]
    _badge: Option<String>,
}

impl Metered {
    fn estimate(&self, base_credits: u64, max_credits: u64, rate: u64) -> Option<String> {
        if self.charged_units != "returned_records"
            || self.unit != "record"
            || self.default_units > self.max_units
            || base_credits.checked_add(self.credits_per_unit.checked_mul(self.max_units)?)?
                != max_credits
        {
            return None;
        }
        Some(format!(
            "{} base + {} per returned record ({} defaults to {}, maximum {} records); maximum {}/request",
            cost(base_credits, rate),
            cost(self.credits_per_unit, rate),
            self.query_param,
            self.default_units,
            self.max_units,
            cost(max_credits, rate)
        ))
    }
}
impl Batch {
    fn estimate(&self, base_credits: u64, max_credits: u64, rate: u64) -> Option<String> {
        if self.unit != "url"
            || self.max_units == 0
            || base_credits.checked_mul(self.max_units)? != max_credits
        {
            return None;
        }
        Some(format!(
            "up to {}/URL; maximum {} URLs and {}/request",
            cost(base_credits, rate),
            self.max_units,
            cost(max_credits, rate)
        ))
    }
}
impl Tariff {
    fn append_charges(&self, line: &mut String, rate: u64) -> Option<()> {
        for surcharge in &self.surcharges {
            let condition = match surcharge.when.as_str() {
                "boolean_true" => "is true",
                "present" => "is supplied",
                _ => return None,
            };
            if surcharge.credits > self.max_credits {
                return None;
            }
            // The structured credit value can be a cap (e.g. 24 assets × 2 credits),
            // so never interpret it as a per-item or necessarily fixed charge.
            line.push_str(&format!(
                "; up to {} extra when {} {} ({})",
                cost(surcharge.credits, rate),
                surcharge.query_param,
                condition,
                surcharge.label
            ));
        }
        if self.normalization_failure_credits > 0 {
            line.push_str(&format!(
                "; normalization-failure charge {}",
                cost(self.normalization_failure_credits, rate)
            ));
        }
        Some(())
    }
}

/// Build-local failures, reported only for tools retained by final name filters.
#[derive(Default)]
pub(super) struct Diagnostics {
    failures: std::collections::BTreeMap<String, &'static str>,
}
impl Diagnostics {
    pub(super) fn record(&mut self, tool: &str, reason: Option<&'static str>) {
        if let Some(reason) = reason {
            self.failures.insert(tool.to_owned(), reason);
        }
    }
    pub(super) fn report(
        &self,
        provider: &str,
        pricing: &CreditPricing,
        tools: &[super::ToolSpec],
    ) {
        let mut categories = std::collections::BTreeMap::<&str, usize>::new();
        let mut affected_operations = 0usize;
        for tool in tools {
            if let Some(reason) = self.failures.get(&tool.name) {
                affected_operations += 1;
                *categories.entry(reason).or_default() += 1;
                tracing::debug!(
                    code = "credit_pricing_operation_unrecognized", provider,
                    tool = %tool.name, method = %tool.method,
                    metadata_key = %pricing.credit_cost_key, reason,
                    "Per-call USDC display estimate unavailable for operation"
                );
            }
        }
        if affected_operations > 0 {
            tracing::warn!(
                code = "credit_pricing_unrecognized", provider,
                metadata_key = %pricing.credit_cost_key,
                affected_operations, categories = ?categories,
                "Catalog credit pricing incomplete: per-call USDC display estimates unavailable for affected operations; retaining original pricing and configured conversion. Payment limits are unchanged; enable DEBUG for per-operation reasons"
            );
        }
    }
}
