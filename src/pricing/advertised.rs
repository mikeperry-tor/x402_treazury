//! Provider advertisements are advisory metadata, separate from validated offers.
use alloy_primitives::U256;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PricingFormat {
    PaymentInfo,
    X402,
    X402Resources,
    X402Discovery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingConfig {
    pub source: String,
    pub format: PricingFormat,
    pub key: Option<String>,
    pub unit: Option<String>,
    #[serde(default)]
    pub usd_as_usdc: bool,
}
impl PricingConfig {
    pub fn catalog_key(&self) -> Option<&str> {
        (self.source == "catalog").then(|| {
            self.key.as_deref().unwrap_or(match self.format {
                PricingFormat::X402 => "x-x402",
                _ => "x-payment-info",
            })
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.source == "catalog" {
            ensure!(
                matches!(
                    self.format,
                    PricingFormat::PaymentInfo | PricingFormat::X402
                ),
                "catalog pricing requires payment-info or x402 format"
            );
        } else {
            crate::network::provider_url(&self.source)?;
            ensure!(
                self.key.is_none(),
                "external pricing does not accept an operation key"
            );
            ensure!(
                matches!(
                    self.format,
                    PricingFormat::X402Resources | PricingFormat::X402Discovery
                ),
                "external pricing requires x402-resources or x402-discovery format"
            );
        }
        ensure!(
            self.key.as_ref().is_none_or(|k| !k.trim().is_empty()),
            "pricing.key must not be empty"
        );
        ensure!(
            self.unit
                .as_deref()
                .is_none_or(|u| matches!(u, "call" | "request")),
            "pricing.unit must be call or request"
        );
        Ok(())
    }
    pub fn legacy(key: &str) -> Self {
        Self {
            source: "catalog".into(),
            format: PricingFormat::PaymentInfo,
            key: Some(key.into()),
            unit: None,
            usd_as_usdc: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TariffKind {
    Unknown,
    Free,
    Fixed,
    Dynamic,
    Range,
    Other,
}
#[derive(Clone, Debug)]
pub struct NormalizedPrice {
    pub kind: TariffKind,
    pub currency: Option<String>,
    pub network: Option<String>,
    pub asset: Option<String>,
    pub unit: Option<String>,
    pub selection_amount: Option<U256>,
    description: String,
}
impl NormalizedPrice {
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn parse(config: &PricingConfig, value: &Value) -> Self {
        if value.is_null() {
            return Self::payment_info(config, value);
        }
        if matches!(
            config.format,
            PricingFormat::X402Resources | PricingFormat::X402Discovery
        ) {
            // Explicit ranges describe the tariff, not a fixed accepts sample.
            if explicit_range_block(value).is_some() {
                return Self::payment_info(config, value);
            }
            let mut result = Self::payment_requirements(&value["accepts"]);
            let price = value.get("price").filter(|price| price.is_object());
            let unit = price
                .and_then(|price| price.get("unit"))
                .or_else(|| value.get("unit"));
            let mode = price
                .and_then(|price| price.get("mode"))
                .or_else(|| value.get("mode"))
                .or_else(|| value.get("pricingMode"));
            let dynamic = mode.is_some_and(|mode| {
                !mode
                    .as_str()
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("fixed"))
            });
            if dynamic
                || unit.is_some_and(|unit| {
                    !unit
                        .as_str()
                        .is_some_and(|unit| matches!(unit, "call" | "request"))
                })
            {
                let quoted = result.selection_amount;
                result.selection_amount = None;
                result.kind = if dynamic {
                    TariffKind::Dynamic
                } else {
                    TariffKind::Other
                };
                result.unit = unit.and_then(Value::as_str).map(str::to_owned);
                result.description = if price.is_some_and(|price| price.get("amount").is_some()) {
                    Self::payment_info(config, value).description
                } else if let Some(amount) = quoted {
                    format!(
                        "Advertised amount: ~${} USDC; unit: {} [spec{}].",
                        crate::pricing::ObservedPrice { amount },
                        result.unit.as_deref().unwrap_or("unknown"),
                        if dynamic { ", dynamic" } else { "" }
                    )
                } else if dynamic {
                    "Pricing: dynamic [spec].".into()
                } else {
                    "Cost: unknown.".into()
                };
            }
            return result;
        }
        if config.format == PricingFormat::X402 {
            let mut normalized = value.clone();
            if let Some(price) = value["price"].as_str().and_then(|p| p.strip_prefix('$')) {
                normalized["amount"] = Value::String(price.into());
                if normalized.get("currency").is_none() {
                    normalized["currency"] = Value::String("USD".into());
                }
                normalized.as_object_mut().unwrap().remove("price");
            }
            return Self::payment_info(config, &normalized);
        }
        Self::payment_info(config, value)
    }
    fn payment_info(config: &PricingConfig, p: &Value) -> Self {
        let mut result = Self {
            kind: TariffKind::Unknown,
            currency: None,
            network: None,
            asset: None,
            unit: None,
            selection_amount: None,
            description: "Cost: unknown.".into(),
        };
        if p.is_null() || p == &serde_json::json!({}) {
            return result;
        }
        let range = explicit_range_block(p);
        if p["authMode"] == "free" && p.get("price").is_none() && range.is_none() {
            result.kind = TariffKind::Free;
            result.selection_amount = Some(U256::ZERO);
            result.description = "Cost: free [spec].".into();
            return result;
        }
        let block = p.get("price").filter(|v| v.is_object()).unwrap_or(p);
        let block = block
            .get("fixed")
            .filter(|v| v.is_object())
            .unwrap_or(block);
        let block = range.unwrap_or(block);
        let metadata = |key| {
            if range.is_some() {
                enclosing_field(p, block, key)
            } else {
                block.get(key).or_else(|| p.get(key))
            }
        };
        // A fixed estimate requires currency in its own amount block.
        let currency = if range.is_some() {
            metadata("currency")
        } else {
            block.get("currency")
        };
        result.currency = currency.and_then(Value::as_str).map(str::to_owned);
        result.network = metadata("network")
            .and_then(Value::as_str)
            .map(str::to_owned);
        result.asset = metadata("asset").and_then(Value::as_str).map(str::to_owned);
        let declared_unit = if range.is_some() {
            metadata("unit")
        } else {
            block
                .get("unit")
                .or_else(|| p.get("price").and_then(|v| v.get("unit")))
                .or_else(|| p.get("unit"))
        };
        result.unit = match declared_unit {
            Some(value) => value.as_str().map(str::to_owned),
            None => config.unit.clone(),
        };
        let unit = block["unit"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(config.unit.as_deref())
            .unwrap_or("call");
        let show = |amount: String| match currency.and_then(Value::as_str) {
            Some(c) if c.eq_ignore_ascii_case("USD") || c.eq_ignore_ascii_case("USDC") => {
                format!("${amount}")
            }
            Some(c) => format!("{amount} {c}"),
            None => amount,
        };
        if let Some(range) = range {
            if let (Some(lo), Some(hi)) = (money(&range["min"]), money(&range["max"]))
                && declared_unit
                    .is_none_or(|unit| unit.as_str().is_some_and(|unit| !unit.is_empty()))
            {
                result.kind = TariffKind::Range;
                result.description = format!("Cost: ~{}–{}/{unit} [spec].", show(lo), show(hi));
            } else {
                result.kind = TariffKind::Other;
                result.description = format!("Pricing: {p} [spec].");
            }
        } else if let Some(amount) = money(&block["amount"]) {
            let declared_mode = block
                .get("mode")
                .or_else(|| p.get("price").and_then(|v| v.get("mode")))
                .or_else(|| p.get("mode"))
                .or_else(|| p.get("pricingMode"));
            let dynamic = declared_mode
                .is_some_and(|v| !v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("fixed")));
            result.kind = if dynamic {
                TariffKind::Dynamic
            } else {
                TariffKind::Fixed
            };
            result.description = format!(
                "Cost: ~{}/{unit} [spec{}].",
                show(amount),
                if dynamic { ", dynamic" } else { "" }
            );
            let currency = result.currency.as_deref();
            if !dynamic
                && result
                    .unit
                    .as_deref()
                    .is_some_and(|u| matches!(u, "call" | "request"))
                && currency.is_some_and(|c| {
                    c.eq_ignore_ascii_case("USDC")
                        || config.usd_as_usdc && c.eq_ignore_ascii_case("USD")
                })
                && compatible_rail(p)
            {
                result.selection_amount = decimal(&value_text(&block["amount"]));
            }
        } else {
            result.kind = TariffKind::Other;
            result.description = format!("Pricing: {p} [spec].");
        }
        result
    }
    pub fn payment_requirements(accepts: &Value) -> Self {
        let amount = accepts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| {
                a["scheme"] == "exact"
                    && a["network"] == "eip155:8453"
                    && a["asset"].as_str().is_some_and(|s| {
                        s.eq_ignore_ascii_case("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913")
                    })
                    && a.pointer("/extra/decimals")
                        .is_none_or(|v| v.as_u64() == Some(6))
            })
            .filter_map(|a| {
                a["amount"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|s| s.parse::<U256>().ok())
            })
            .min();
        Self {
            kind: if amount.is_some() {
                TariffKind::Fixed
            } else {
                TariffKind::Unknown
            },
            currency: Some("USDC".into()),
            network: Some("eip155:8453".into()),
            asset: Some("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".into()),
            unit: Some("call".into()),
            selection_amount: amount,
            description: amount.map_or_else(
                || format!("Pricing: {accepts} [spec]."),
                |amount| {
                    format!(
                        "Cost: ~${}/call [spec].",
                        crate::pricing::ObservedPrice { amount }
                    )
                },
            ),
        }
    }
}
/// Inner tariff wrappers take display precedence, but an explicit outer range
/// still prevents an inner fixed sample from becoming a selection estimate.
fn explicit_range_block(value: &Value) -> Option<&Value> {
    [
        value.pointer("/price/fixed"),
        value.get("price"),
        value.get("fixed"),
        Some(value),
    ]
    .into_iter()
    .flatten()
    .find(|block| block.is_object() && (block.get("min").is_some() || block.get("max").is_some()))
}

/// Metadata follows the selected block outward; nested samples cannot supply it.
fn enclosing_field<'a>(root: &'a Value, block: &'a Value, key: &str) -> Option<&'a Value> {
    block
        .get(key)
        .or_else(|| {
            root.pointer("/price/fixed")
                .filter(|fixed| std::ptr::eq(*fixed, block))
                .and_then(|_| root["price"].get(key))
        })
        .or_else(|| root.get(key))
}

fn supported_network(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| matches!(s, "eip155:8453" | "base"))
}
fn supported_asset(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.eq_ignore_ascii_case("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913")
            || s.eq_ignore_ascii_case("USDC")
    })
}
fn compatible_rail(value: &Value) -> bool {
    // Explicit contrary rail information never becomes a supported estimate.
    for key in ["network", "caip2Network", "x402Network"] {
        if let Some(v) = value.get(key)
            && !supported_network(v)
        {
            return false;
        }
    }
    if let Some(v) = value.get("asset")
        && !supported_asset(v)
    {
        return false;
    }
    if let Some(v) = value.get("price").filter(|v| v.is_object())
        && !compatible_rail(v)
    {
        return false;
    }
    if let Some(v) = value.get("fixed").filter(|v| v.is_object())
        && !compatible_rail(v)
    {
        return false;
    }
    if let Some(protocols) = value.get("protocols").and_then(Value::as_array) {
        return protocols
            .iter()
            .any(|p| p.as_str() == Some("x402") || p.get("x402").is_some_and(compatible_rail));
    }
    true
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}
pub(crate) fn money(value: &Value) -> Option<String> {
    if !(value.is_string() || value.is_u64() || value.is_i64()) {
        return None;
    }
    let text = value_text(value).trim().to_owned();
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\d+(?:\.\d+)?$").expect("constant regex"));
    if !RE.is_match(&text) {
        return None;
    }
    Some(if let Some((whole, fraction)) = text.split_once('.') {
        format!("{whole}.{:0<2}", fraction.trim_end_matches('0'))
    } else {
        text
    })
}
pub fn decimal(text: &str) -> Option<U256> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || text.contains('.') && fraction.is_empty()
    {
        return None;
    }
    whole
        .parse::<U256>()
        .ok()?
        .checked_mul(U256::from(1_000_000))?
        .checked_add(format!("{fraction:0<6}").parse::<U256>().ok()?)
}
#[derive(Debug)]
pub struct Acquisition(tokio::task::AbortHandle);
impl Acquisition {
    pub fn new(abort: tokio::task::AbortHandle) -> Self {
        Self(abort)
    }
}
impl Drop for Acquisition {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Clone, Debug)]
pub struct Advertisement {
    pub price: NormalizedPrice,
    pub source: String,
    pub fresh_until: Option<Instant>,
}
#[derive(Clone, Debug, Default)]
pub struct AdvertisedPricing {
    value: Arc<RwLock<Option<Advertisement>>>,
    owner: Option<Arc<Acquisition>>,
}
impl AdvertisedPricing {
    /// Original catalog or metadata URL, distinct from observed payment offers.
    pub fn source(&self) -> Option<String> {
        self.value
            .read()
            .expect("advertised pricing poisoned")
            .as_ref()
            .map(|value| value.source.clone())
    }

    pub fn new(price: NormalizedPrice, source: String, fresh_until: Option<Instant>) -> Self {
        let result = Self::default();
        result.replace(price, source, fresh_until);
        result
    }
    pub fn amount(&self) -> Option<U256> {
        let value = self.value.read().expect("advertised pricing poisoned");
        value
            .as_ref()
            .filter(|v| v.fresh_until.is_none_or(|until| Instant::now() < until))
            .and_then(|v| v.price.selection_amount)
    }
    pub fn replace(&self, price: NormalizedPrice, source: String, fresh_until: Option<Instant>) {
        *self.value.write().expect("advertised pricing poisoned") = Some(Advertisement {
            price,
            source,
            fresh_until,
        });
    }
    pub fn set_fresh_until(&self, fresh_until: Option<Instant>) {
        if let Some(value) = self
            .value
            .write()
            .expect("advertised pricing poisoned")
            .as_mut()
        {
            value.fresh_until = fresh_until;
        }
    }
    pub fn set_expires(&self, expires: u64) {
        let remaining = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(expires))
            .and_then(|expires| expires.duration_since(std::time::SystemTime::now()).ok());
        let now = Instant::now();
        self.set_fresh_until(Some(
            remaining
                .and_then(|duration| now.checked_add(duration))
                .unwrap_or(now),
        ));
    }
    pub fn attach_owner(&mut self, owner: Arc<Acquisition>) {
        self.owner = Some(owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn config() -> PricingConfig {
        PricingConfig {
            source: "catalog".into(),
            format: PricingFormat::PaymentInfo,
            key: None,
            unit: Some("call".into()),
            usd_as_usdc: true,
        }
    }
    #[test]
    fn exact_selection_requires_explicit_currency_unit_and_fixed_amount() {
        let cfg = config();
        let normalized = NormalizedPrice::parse(
            &cfg,
            &json!({"price":{"amount":"0.010000","currency":"USD"}}),
        );
        assert_eq!(normalized.selection_amount, Some(U256::from(10_000)));
        assert_eq!(normalized.description(), "Cost: ~$0.01/call [spec].");
        for value in [
            json!({"amount":"0.01"}),
            json!({"amount":"0.01","currency":"USD","unit":"record"}),
            json!({"amount":"0.01","currency":"USD","unit":null}),
            json!({"unit":"record","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
            json!({"amount":"0.01","currency":"USD","mode":"dynamic"}),
            json!({"amount":"0.01","currency":"USD","mode":"quote"}),
            json!({"amount":"0.0000001","currency":"USD"}),
            json!({"min":"0.01","max":"0.02","currency":"USD"}),
            json!({"amount":"0.01","currency":"USDC","network":"solana:mainnet"}),
            json!({"price":{"amount":"0.01","currency":"USD"},"protocols":[{"x402":{"network":"solana:mainnet"}}]}),
        ] {
            assert_eq!(
                NormalizedPrice::parse(&cfg, &value).selection_amount,
                None,
                "{value}"
            );
        }
        let legacy = PricingConfig::legacy("x-price");
        assert_eq!(
            NormalizedPrice::parse(&legacy, &json!({"amount":"0.01","currency":"USD"}))
                .selection_amount,
            None
        );
        assert_eq!(
            NormalizedPrice::parse(&cfg, &json!({"amount":"0","currency":"USDC"})).selection_amount,
            Some(U256::ZERO)
        );
        assert_eq!(
            NormalizedPrice::parse(&cfg, &json!({"authMode":"free"})).kind,
            TariffKind::Free
        );
    }
    #[test]
    fn nested_effective_modes_render_consistent_dynamic_labels() {
        for value in [
            json!({"mode":"dynamic","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
            json!({"pricingMode":"quote","price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
            json!({"price":{"mode":"dynamic","fixed":{"amount":"0.01","currency":"USD"}}}),
            json!({"price":{"mode":"fixed","fixed":{"amount":"0.01","currency":"USD","mode":"quote"}}}),
        ] {
            let normalized = NormalizedPrice::parse(&config(), &value);
            assert_eq!(normalized.kind, TariffKind::Dynamic);
            assert_eq!(normalized.selection_amount, None);
            assert_eq!(
                normalized.description(),
                "Cost: ~$0.01/call [spec, dynamic]."
            );
        }
        let fixed = NormalizedPrice::parse(
            &config(),
            &json!({"mode":"dynamic","price":{"mode":"fixed","fixed":{"amount":"0.01","currency":"USD"}}}),
        );
        assert_eq!(fixed.kind, TariffKind::Fixed);
        assert_eq!(fixed.selection_amount, Some(U256::from(10_000)));
        assert_eq!(fixed.description(), "Cost: ~$0.01/call [spec].");
    }

    fn range_wrappers(range: Value) -> Vec<Value> {
        vec![
            range.clone(),
            json!({"price":range.clone()}),
            json!({"price":{"fixed":range.clone()}}),
            json!({"fixed":range.clone()}),
            json!({"min":range["min"],"max":range["max"],"currency":"USD",
                "price":{"fixed":{"amount":"0.01","currency":"USD"}}}),
            json!({"price":{"min":range["min"],"max":range["max"],"currency":"USD",
                "fixed":{"amount":"0.01","currency":"USD"}}}),
        ]
    }

    #[test]
    fn explicit_ranges_override_fixed_samples_across_catalog_and_external_formats() {
        let range = json!({"amount":"0.01","min":"0.01","max":"1","currency":"USD"});
        for format in [
            PricingFormat::PaymentInfo,
            PricingFormat::X402,
            PricingFormat::X402Resources,
            PricingFormat::X402Discovery,
        ] {
            let mut cfg = config();
            cfg.format = format;
            for mut value in range_wrappers(range.clone()) {
                value["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
                    "asset":crate::payment::USDC,"amount":"1000"}]);
                let normalized = NormalizedPrice::parse(&cfg, &value);
                assert_eq!(normalized.kind, TariffKind::Range, "{value}");
                assert_eq!(normalized.selection_amount, None, "{value}");
                assert_eq!(
                    normalized.description(),
                    "Cost: ~$0.01–$1/call [spec].",
                    "{value}"
                );
            }
        }
    }

    #[test]
    fn range_metadata_follows_selected_block_and_enclosing_wrappers() {
        for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
            let mut cfg = config();
            cfg.format = format;
            let mut outer = json!({"min":"0.01","max":"1","currency":"TOK","unit":"token",
                "network":"networkA","asset":"assetA",
                "price":{"fixed":{"amount":"0.01","currency":"USD","unit":"call",
                    "network":"networkB","asset":"assetB"}},
                "accepts":[{"scheme":"exact","network":"eip155:8453",
                    "asset":crate::payment::USDC,"amount":"1000"}]});
            let normalized = NormalizedPrice::parse(&cfg, &outer);
            assert_eq!(
                normalized.description(),
                "Cost: ~0.01 TOK–1 TOK/token [spec]."
            );
            assert_eq!(normalized.currency.as_deref(), Some("TOK"));
            assert_eq!(normalized.unit.as_deref(), Some("token"));
            assert_eq!(normalized.network.as_deref(), Some("networkA"));
            assert_eq!(normalized.asset.as_deref(), Some("assetA"));
            assert_eq!(normalized.selection_amount, None);
            for key in ["currency", "unit", "network", "asset"] {
                outer.as_object_mut().unwrap().remove(key);
            }
            let sparse = NormalizedPrice::parse(&cfg, &outer);
            assert_eq!(sparse.description(), "Cost: ~0.01–1/call [spec].");
            assert_eq!(sparse.currency, None);
            assert_eq!(sparse.network, None);
            assert_eq!(sparse.asset, None);
            assert_eq!(sparse.selection_amount, None);
            let inner = NormalizedPrice::parse(
                &cfg,
                &json!({
                    "currency":"ROOT","unit":"root-unit","network":"root-network","asset":"root-asset",
                    "price":{"currency":"TOK","unit":"token","network":"parent-network","asset":"parent-asset",
                        "fixed":{"min":"0.01","max":"1"}}
                }),
            );
            assert_eq!(inner.description(), "Cost: ~0.01 TOK–1 TOK/token [spec].");
            assert_eq!(inner.currency.as_deref(), Some("TOK"));
            assert_eq!(inner.unit.as_deref(), Some("token"));
            assert_eq!(inner.network.as_deref(), Some("parent-network"));
            assert_eq!(inner.asset.as_deref(), Some("parent-asset"));
            assert_eq!(inner.selection_amount, None);
        }
        let fixed = NormalizedPrice::parse(
            &config(),
            &json!({
                "currency":"USD","price":{"fixed":{"amount":"0.01"}}
            }),
        );
        assert_eq!(fixed.currency, None);
        assert_eq!(fixed.selection_amount, None);
    }

    #[test]
    fn malformed_range_units_preserve_details_without_inventing_call_unit() {
        for unit in [json!(null), json!(false), json!({}), json!("")] {
            let value = json!({"min":"0.01","max":"1","currency":"USD","unit":unit,
                "price":{"fixed":{"amount":"0.01","currency":"USD","unit":"call"}}});
            let normalized = NormalizedPrice::parse(&config(), &value);
            assert_eq!(normalized.kind, TariffKind::Other);
            assert_eq!(normalized.selection_amount, None);
            assert_eq!(
                normalized.description(),
                format!("Pricing: {value} [spec].")
            );
        }
    }

    #[test]
    fn partial_and_malformed_ranges_never_become_fixed_estimates() {
        for range in [
            json!({"min":"0.01"}),
            json!({"max":"1"}),
            json!({"min":null,"max":"1"}),
            json!({"min":"bad","max":"1"}),
            json!({"min":"0.01","max":{}}),
            json!({"min":false,"max":[]}),
        ] {
            let mut range = range;
            range["amount"] = json!("0.01");
            range["currency"] = json!("USD");
            for format in [PricingFormat::PaymentInfo, PricingFormat::X402Resources] {
                let mut cfg = config();
                cfg.format = format;
                for mut value in range_wrappers(range.clone()) {
                    value["accepts"] = json!([{"scheme":"exact","network":"eip155:8453",
                        "asset":crate::payment::USDC,"amount":"1000"}]);
                    let normalized = NormalizedPrice::parse(&cfg, &value);
                    assert_eq!(normalized.kind, TariffKind::Other, "{value}");
                    assert_eq!(normalized.selection_amount, None, "{value}");
                    assert!(normalized.description().starts_with("Pricing: "), "{value}");
                }
            }
        }
        let mut free = json!({"authMode":"free","min":null});
        assert_eq!(
            NormalizedPrice::parse(&config(), &free).selection_amount,
            None
        );
        free.as_object_mut().unwrap().remove("min");
        assert_eq!(
            NormalizedPrice::parse(&config(), &free).selection_amount,
            Some(U256::ZERO)
        );
    }

    #[test]
    fn x402_dollar_forwarding_preserves_explicit_range_restrictions() {
        let mut cfg = config();
        cfg.format = PricingFormat::X402;
        for range in [
            json!({"min":"0.01","max":"1"}),
            json!({"min":"0.01"}),
            json!({"max":null}),
        ] {
            let mut value = range;
            value["price"] = json!("$0.01");
            let normalized = NormalizedPrice::parse(&cfg, &value);
            assert_eq!(normalized.selection_amount, None, "{value}");
            if value.get("max").is_some_and(Value::is_string) {
                assert_eq!(normalized.kind, TariffKind::Range);
                assert_eq!(normalized.description(), "Cost: ~$0.01–$1/call [spec].");
            } else {
                assert_eq!(normalized.kind, TariffKind::Other);
            }
        }
    }

    #[test]
    fn atomic_alternatives_require_supported_asset_and_choose_one_offer() {
        let base = json!({"scheme":"exact","network":"eip155:8453","asset":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","amount":"4000"});
        let mut lower = base.clone();
        lower["amount"] = json!("1000");
        assert_eq!(
            NormalizedPrice::payment_requirements(&json!([base.clone(), lower])).selection_amount,
            Some(U256::from(1000))
        );
        for (key, value) in [
            ("asset", json!("USDC")),
            ("network", json!("base")),
            ("amount", json!("0.001")),
            ("scheme", json!("upto")),
            ("extra", json!({"decimals":18})),
        ] {
            let mut altered = base.clone();
            altered[key] = value;
            assert_eq!(
                NormalizedPrice::payment_requirements(&json!([altered])).selection_amount,
                None
            );
        }
    }
    #[test]
    fn handle_clones_share_fresh_advertisements_independently_of_observed_cache() {
        let price = NormalizedPrice::parse(&config(), &json!({"amount":"0.01","currency":"USD"}));
        let handle = AdvertisedPricing::new(price.clone(), "local.json".into(), None);
        let clone = handle.clone();
        assert_eq!(clone.amount(), Some(U256::from(10_000)));
        handle.set_expires(0);
        assert_eq!(clone.amount(), None);
        handle.replace(price, "metadata".into(), None);
        assert_eq!(clone.amount(), Some(U256::from(10_000)));
        assert_eq!(AdvertisedPricing::default().amount(), None);
    }
    #[test]
    fn x402_dollar_format_and_decimal_parsing_are_exact() {
        let mut cfg = config();
        cfg.format = PricingFormat::X402;
        assert_eq!(
            NormalizedPrice::parse(&cfg, &json!({"price":"$0.015","network":"eip155:8453"}))
                .selection_amount,
            Some(U256::from(15_000))
        );
        for text in ["-1", "1e-3", "1.", ".1", " 1", "1.0000000"] {
            assert_eq!(decimal(text), None);
        }
        assert_eq!(decimal("0.000001"), Some(U256::from(1)));
        assert_eq!(decimal(&U256::MAX.to_string()), None);
    }
}
