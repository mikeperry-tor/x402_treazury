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
    pub(super) fn describe(&self, metadata: &Value) -> String {
        let rate = self.rate().expect("validated credit pricing");
        let provenance = format!(
            " [spec credits × configured {} USDC/credit; estimate, payment challenge is authoritative].",
            amount(rate as u128)
        );
        match self.estimate(metadata, rate) {
            Some(line) => format!("Estimated cost: {line}{provenance}"),
            None => {
                tracing::warn!(
                    code = "credit_pricing_unrecognized",
                    "Credit pricing metadata is missing or unsupported; per-call USDC estimate unavailable, retaining original pricing and configured conversion"
                );
                format!("Per-call USDC estimate unavailable{provenance}")
            }
        }
    }
    fn estimate(&self, metadata: &Value, rate: u64) -> Option<String> {
        let p: Tariff = serde_json::from_value(metadata.clone()).ok()?;
        if p.version != 1
            || p.max_credits < p.base_credits
            || p.normalization_failure_credits > p.max_credits
            || (p.batch.is_some() && p.metered.is_some())
            || p.settlement
                .as_deref()
                .is_some_and(|s| s != "actual_credits")
        {
            return None;
        }
        let cost = |credits: u64| format!("{} USDC", amount(credits as u128 * rate as u128));
        let mut line = if let Some(m) = p.metered {
            if m.charged_units != "returned_records"
                || m.unit != "record"
                || m.default_units > m.max_units
                || p.base_credits
                    .checked_add(m.credits_per_unit.checked_mul(m.max_units)?)?
                    != p.max_credits
            {
                return None;
            }
            format!(
                "{} base + {} per returned record ({} defaults to {}, maximum {} records); maximum {}/request",
                cost(p.base_credits),
                cost(m.credits_per_unit),
                m.query_param,
                m.default_units,
                m.max_units,
                cost(p.max_credits)
            )
        } else if let Some(b) = p.batch {
            if b.unit != "url"
                || b.max_units == 0
                || p.base_credits.checked_mul(b.max_units)? != p.max_credits
            {
                return None;
            }
            format!(
                "up to {}/URL; maximum {} URLs and {}/request",
                cost(p.base_credits),
                b.max_units,
                cost(p.max_credits)
            )
        } else if p.base_credits == p.max_credits && p.surcharges.is_empty() {
            format!("{}/request", cost(p.base_credits))
        } else {
            format!(
                "{} base; maximum {}/request",
                cost(p.base_credits),
                cost(p.max_credits)
            )
        };
        for surcharge in p.surcharges {
            let condition = match surcharge.when.as_str() {
                "boolean_true" => "is true",
                "present" => "is supplied",
                _ => return None,
            };
            if surcharge.credits > p.max_credits {
                return None;
            }
            // The structured credit value can be a cap (e.g. 24 assets × 2 credits),
            // so never interpret it as a per-item or necessarily fixed charge.
            line.push_str(&format!(
                "; up to {} extra when {} {} ({})",
                cost(surcharge.credits),
                surcharge.query_param,
                condition,
                surcharge.label
            ));
        }
        if p.normalization_failure_credits > 0 {
            line.push_str(&format!(
                "; normalization-failure charge {}",
                cost(p.normalization_failure_credits)
            ));
        }
        Some(line)
    }
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
