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
        let units = crate::pricing::advertised::decimal(&self.usdc_per_credit).context(
            "credit_pricing.usdc_per_credit must be a positive decimal with at most six decimal places",
        )?;
        u64::try_from(units)
            .ok()
            .filter(|n| *n > 0)
            .context("credit_pricing.usdc_per_credit is zero or out of range")
    }
    pub(super) fn describe(
        &self,
        metadata: &Value,
        published: &Value,
    ) -> Result<(String, bool), &'static str> {
        let rate = self.rate().expect("validated credit pricing");
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
        let fixed = p.metered.is_none()
            && p.batch.is_none()
            && p.base_credits == p.max_credits
            && p.surcharges.is_empty();
        // Collapse only the vendor's bare fixed-credit sentence. Other prose
        // may include billing conditions absent from the structured schedule.
        let covered = fixed
            && published.as_str().is_some_and(|text| {
                let credits = p.base_credits;
                let unit = if credits == 1 { "credit" } else { "credits" };
                text == format!("{credits} {unit} per successful request.")
            });
        let mut line = if let Some(m) = &p.metered {
            m.estimate(p.base_credits, p.max_credits, rate)
                .ok_or("invalid_or_unsupported_metered_tariff")?
        } else if let Some(b) = &p.batch {
            b.estimate(p.base_credits, p.max_credits, rate)
                .ok_or("invalid_or_unsupported_batch_tariff")?
        } else if fixed {
            let base = cost(p.base_credits, rate);
            if covered {
                format!("{base} on success")
            } else {
                format!("{base}/request")
            }
        } else {
            format!(
                "{} base; maximum {}/request",
                cost(p.base_credits, rate),
                cost(p.max_credits, rate)
            )
        };
        p.append_charges(&mut line, rate)
            .ok_or("invalid_or_unsupported_charge")?;
        Ok((format!("Estimated cost: {line}."), covered))
    }
}
fn cost(credits: u64, rate: u64) -> String {
    format!("${}", amount(credits as u128 * rate as u128))
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
    #[serde(default)]
    billable_domain_errors: Vec<BillableDomainError>,
    batch: Option<Batch>,
    metered: Option<Metered>,
    settlement: Option<String>,
    // Recognized transport hints do not change credit amounts or payment authority.
    #[serde(rename = "x402ExactPolicy")]
    _x402_exact_policy: Option<ExactPolicy>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExactPolicy {
    Always,
    FixedPriceOnly,
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
    value: Option<String>,
    #[serde(rename = "badge")]
    _badge: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BillableDomainError {
    code: String,
    http_status: u64,
    credits: u64,
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
            let condition = match (surcharge.when.as_str(), surcharge.value.as_deref()) {
                ("boolean_true", None) => "is true".to_owned(),
                ("present", None) => "is supplied".to_owned(),
                ("includes", Some(value)) => format!("includes {value}"),
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
        for error in &self.billable_domain_errors {
            if error.credits > self.max_credits {
                return None;
            }
            line.push_str(&format!(
                "; {} for {} (HTTP {})",
                cost(error.credits, rate),
                error.code,
                error.http_status
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
    pub(super) fn record(&mut self, tool: &str, reason: &'static str) {
        self.failures.insert(tool.to_owned(), reason);
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
                "Some catalog tools omit price estimates because their credit schedules could not be interpreted. Enable DEBUG for the affected tools and reasons"
            );
        }
    }
}
