//! Provider advertisements are advisory metadata, separate from validated offers.
use alloy_primitives::U256;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub(crate) const UNKNOWN_PRICE: &str = "Cost: unknown.";

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
    description: Option<String>,
    legacy_listing: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct TariffText {
    pub(crate) text: String,
    pub(crate) usdc: bool,
    pub(crate) legacy_listing: Option<String>,
    pub(crate) exact_offer: Option<U256>,
}

impl NormalizedPrice {
    pub fn description(&self) -> &str {
        self.description.as_deref().unwrap_or(UNKNOWN_PRICE)
    }

    pub(crate) fn tariff_text(&self) -> Option<TariffText> {
        Some(TariffText {
            text: self.description.as_ref()?.clone(),
            usdc: self
                .currency
                .as_deref()
                .is_some_and(|currency| currency.eq_ignore_ascii_case("USDC")),
            legacy_listing: self.legacy_listing.clone(),
            exact_offer: None,
        })
    }

    pub(crate) fn unknown() -> Self {
        Self {
            kind: TariffKind::Unknown,
            currency: None,
            network: None,
            asset: None,
            unit: None,
            selection_amount: None,
            description: None,
            legacy_listing: None,
        }
    }

    pub fn parse(config: &PricingConfig, value: &Value) -> Self {
        let origin = Origin::from_format(&config.format);
        let dollar = if config.format == PricingFormat::X402 {
            dollar_price(value)
        } else {
            None
        };
        let price = dollar.as_ref().unwrap_or(value);
        let range = explicit_range_block(price);
        let block = range.unwrap_or_else(|| tariff_block(price));
        let mode = tariff_mode(price, block);
        let free = tariff_blocks(price)
            .into_iter()
            .flatten()
            .any(|block| block["authMode"] == "free");
        let mut result =
            Self::payment_info(config, price, block, range.is_some(), origin, free, mode);
        let paid = free
            && (tariff_blocks(value)
                .into_iter()
                .flatten()
                .any(competes_with_free)
                || match result.kind {
                    TariffKind::Unknown => false,
                    TariffKind::Fixed if is_zero_amount(&block["amount"]) => {
                        opaque_price(price, origin.is_external(), true).is_some()
                    }
                    _ => true,
                });
        if free && !paid {
            let mut description = format!("Listed: free [{}].", origin.label());
            append_billing_terms(&mut description, price, origin);
            return Self {
                kind: TariffKind::Free,
                selection_amount: Some(U256::ZERO),
                description: Some(description),
                legacy_listing: result.legacy_listing,
                ..Self::unknown()
            };
        }

        let unrendered_terms = free && result.kind == TariffKind::Unknown;
        let supports_estimate =
            enclosing_blocks(price, block).all(allows_request_estimate) && compatible_rail(price);
        if origin.is_external() {
            result = result.with_external_offer(price, block, supports_estimate && !paid);
        } else {
            result.selection_amount = result.selection_amount.filter(|_| supports_estimate);
        }
        if free {
            result.selection_amount = None;
            const CONFLICT: &str = "Conflicting free and paid metadata.";
            let description = result.append_description(CONFLICT);
            if unrendered_terms {
                append_billing_terms(description, price, origin);
            }
        }
        result
    }

    fn payment_info(
        config: &PricingConfig,
        price: &Value,
        block: &Value,
        range: bool,
        origin: Origin,
        free: bool,
        mode: Option<&Value>,
    ) -> Self {
        let mut result = Self::unknown();
        let currency = if range {
            enclosing_field(price, block, "currency")
        } else {
            block.get("currency")
        };
        result.currency = currency.and_then(Value::as_str).map(str::to_owned);
        result.network = enclosing_field(price, block, "network")
            .and_then(Value::as_str)
            .map(str::to_owned);
        result.asset = enclosing_field(price, block, "asset")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let declared_unit = enclosing_field(price, block, "unit");
        result.unit = effective_unit(config, declared_unit);
        let legacy_unit = result.unit.as_deref();
        let dynamic = mode.is_some_and(is_dynamic_mode);
        let label = evidence_label(origin, mode, result.unit.is_none());
        let suffix = unit_suffix(result.unit.as_deref());
        let show = |amount| display_amount(amount, result.currency.as_deref());
        if range {
            result.kind = TariffKind::Range;
            let (min, max) = (&block["min"], &block["max"]);
            let legacy_unit = legacy_unit.filter(|_| !min.is_f64() && !max.is_f64());
            let terms = match (range_bound(min), range_bound(max)) {
                (Some(min), Some(max)) => {
                    let terms = format!("~{}–{}", show(min), show(max));
                    result.legacy_listing =
                        legacy_unit.map(|unit| format!("Cost: {terms}/{unit} [spec]."));
                    Some(terms)
                }
                (Some(min), None) => Some(format!("from ~{}", show(min))),
                (None, Some(max)) => Some(format!("up to ~{}", show(max))),
                (None, None) => None,
            };
            result.description = Some(terms.map_or_else(
                || format!("Published price range unavailable [{label}]."),
                |terms| format!("Listed: {terms}{suffix} [{label}]."),
            ));
        } else if let Some(amount) = money(&block["amount"]).filter(|_| !has_variable_terms(price))
        {
            result.kind = if dynamic {
                TariffKind::Dynamic
            } else {
                TariffKind::Fixed
            };
            let amount = show(amount);
            result.description = Some(format!("Listed: ~{amount}{suffix} [{label}]."));
            // Legacy external records display a listed amount for dynamic tariffs
            // and explicit non-call units. Other external records display the offer.
            let legacy_listed = !origin.is_external()
                || dynamic
                || declared_unit.is_some_and(|unit| !call_unit(unit.as_str()));
            let legacy_mode = if dynamic { ", dynamic" } else { "" };
            result.legacy_listing = legacy_unit
                .filter(|_| legacy_listed)
                .map(|unit| format!("Cost: ~{amount}/{unit} [spec{legacy_mode}]."));
            if !origin.is_external()
                && !dynamic
                && call_unit(result.unit.as_deref())
                && result.currency.as_deref().is_some_and(|currency| {
                    currency.eq_ignore_ascii_case("USDC")
                        || config.usd_as_usdc && currency.eq_ignore_ascii_case("USD")
                })
            {
                result.selection_amount = decimal(&value_text(&block["amount"]));
            }
        } else if let Some(opaque) = opaque_price(price, origin.is_external(), free) {
            result.kind = TariffKind::Other;
            result.description = Some(format!(
                "Pricing: {opaque} [{}].",
                evidence_label(origin, mode, false)
            ));
        } else if dynamic {
            result.kind = TariffKind::Dynamic;
            let kind = if mode
                .and_then(Value::as_str)
                .is_some_and(|mode| mode.eq_ignore_ascii_case("metered"))
            {
                "metered"
            } else {
                "dynamic"
            };
            result.description = Some(format!("Pricing: {kind} [{}].", origin.label()));
        }
        if matches!(
            result.kind,
            TariffKind::Range | TariffKind::Fixed | TariffKind::Dynamic
        ) && let Some(description) = &mut result.description
        {
            append_billing_terms(description, price, origin);
        }
        result
    }

    fn with_external_offer(
        mut self,
        price: &Value,
        block: &Value,
        supports_estimate: bool,
    ) -> Self {
        let quoted = exact_base_usdc_amount(&price["accepts"]);
        self.selection_amount = quoted.filter(|_| {
            supports_estimate
                && matches!(self.kind, TariffKind::Fixed | TariffKind::Unknown)
                && (call_unit(self.unit.as_deref())
                    || self.unit.is_none() && enclosing_field(price, block, "unit").is_none())
        });
        let Some(quoted) = quoted else {
            if price["accepts"]
                .as_array()
                .is_some_and(|offers| offers.iter().any(has_advertised_offer))
            {
                const NO_ESTIMATE: &str = "Advertised offers supply no exact Base USDC price estimate [external pricing].";
                if self.kind == TariffKind::Unknown {
                    self.kind = TariffKind::Other;
                }
                self.append_description(NO_ESTIMATE);
            }
            return self;
        };

        let eligible = self.selection_amount.is_some();
        let sample_unit = self
            .unit
            .as_deref()
            .filter(|unit| !call_unit(Some(unit)) && money(&block["amount"]).is_none());
        let billing_unit = sample_unit.map(|unit| format!("Billing unit: {unit}."));
        if self.kind == TariffKind::Unknown {
            let mut offer = Self::request_offer(quoted, eligible);
            if let Some(billing_unit) = billing_unit {
                offer.append_description(&billing_unit);
                offer.unit = self.unit;
            }
            return offer;
        }
        self.append_description(&external_offer(quoted, eligible));
        if self.kind == TariffKind::Dynamic
            && let Some(billing_unit) = billing_unit
        {
            self.append_description(&billing_unit);
        }
        self
    }

    pub fn payment_requirements(accepts: &Value) -> Self {
        exact_base_usdc_amount(accepts)
            .map_or_else(Self::unknown, |amount| Self::request_offer(amount, true))
    }

    fn append_description(&mut self, fragment: &str) -> &mut String {
        let text = self.description.get_or_insert_with(String::new);
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(fragment);
        text
    }

    fn request_offer(amount: U256, eligible: bool) -> Self {
        Self {
            kind: if eligible {
                TariffKind::Fixed
            } else {
                TariffKind::Other
            },
            currency: Some("USDC".into()),
            network: Some("eip155:8453".into()),
            asset: Some(crate::payment::USDC.to_ascii_lowercase()),
            unit: Some("request".into()),
            selection_amount: eligible.then_some(amount),
            description: Some(external_offer(amount, eligible)),
            legacy_listing: None,
        }
    }
}

pub(crate) fn external_offer(amount: U256, eligible: bool) -> String {
    let heading = if eligible {
        "Advertised exact offer"
    } else {
        "Advertised sample offer"
    };
    let amount = crate::pricing::ObservedPrice { amount };
    format!("{heading}: ~${amount}/request [external pricing].")
}

fn has_advertised_offer(offer: &Value) -> bool {
    ["scheme", "network", "asset"]
        .into_iter()
        .all(|field| offer[field].as_str().is_some_and(|value| !value.is_empty()))
}

fn exact_base_usdc_amount(accepts: &Value) -> Option<U256> {
    accepts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| {
            a["scheme"] == "exact"
                && a["network"] == "eip155:8453"
                && a["asset"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(crate::payment::USDC))
                && a.pointer("/extra/decimals")
                    .is_none_or(|v| v.as_u64() == Some(6))
        })
        .filter_map(|a| {
            a["amount"]
                .as_str()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<U256>().ok())
        })
        .min()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Spec,
    External,
}

impl Origin {
    fn from_format(format: &PricingFormat) -> Self {
        match format {
            PricingFormat::X402Resources | PricingFormat::X402Discovery => Self::External,
            _ => Self::Spec,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::External => "external pricing",
        }
    }

    fn is_external(self) -> bool {
        self == Self::External
    }
}

const VARIABLE_TERMS: [(&str, &str); 2] = [
    ("percentage", "Percentage fee"),
    ("minimum", "Minimum charge"),
];

fn append_billing_terms(description: &mut String, price: &Value, origin: Origin) {
    use std::fmt::Write;

    let mut included = Vec::new();
    for scope in tariff_blocks(price).into_iter().flatten() {
        for (key, label) in [
            ("pricingTerms", "Billing terms"),
            ("conditions", "Billing conditions"),
        ]
        .into_iter()
        .chain(VARIABLE_TERMS)
        {
            let Some(terms) = scope
                .get(key)
                .filter(|value| meaningful_billing_value(value))
            else {
                continue;
            };
            if included.contains(&(key, terms)) {
                continue;
            }
            let content = terms
                .as_str()
                .map_or_else(|| terms.to_string(), str::to_owned);
            included.push((key, terms));
            let _ = write!(description, " {label} [{}] {content}", origin.label());
        }
    }
}

fn has_variable_terms(price: &Value) -> bool {
    tariff_blocks(price).into_iter().flatten().any(|block| {
        VARIABLE_TERMS
            .iter()
            .any(|(key, _)| block.get(*key).is_some_and(meaningful_billing_value))
    })
}

fn opaque_price(value: &Value, external: bool, free: bool) -> Option<String> {
    if value.is_string() || value.is_number() {
        return Some(value.to_string());
    }
    // Keep unhandled tariff declarations visible. Authentication capability
    // flags and payment requirements alone supply no tariff text.
    let terms: std::collections::BTreeMap<_, _> = value
        .as_object()?
        .iter()
        .filter(|(key, value)| {
            !value.is_null()
                && (!matches!(key.as_str(), "pricingTerms" | "conditions")
                    || meaningful_billing_value(value))
                && if external {
                    VARIABLE_TERMS.iter().any(|(term, _)| key.as_str() == *term)
                        || matches!(
                            key.as_str(),
                            "price"
                                | "fixed"
                                | "amount"
                                | "pricingTerms"
                                | "conditions"
                                | "mode"
                                | "pricingMode"
                                | "currency"
                                | "unit"
                                | "network"
                                | "asset"
                        )
                } else {
                    !matches!(
                        key.as_str(),
                        "authMode"
                            | "paymentRequired"
                            | "securitySchemes"
                            | "protocols"
                            | "accepts"
                    )
                }
        })
        .collect();
    if !terms
        .iter()
        .any(|(key, value)| has_tariff_term(key, value, free))
    {
        return None;
    }
    if terms.len() == 1
        && let Some((key, scalar)) = terms.first_key_value()
        && key.as_str() == "price"
        && (scalar.is_string() || scalar.is_number())
    {
        return Some(scalar.to_string());
    }
    Some(serde_json::to_string(&terms).expect("serializable price metadata"))
}

fn has_tariff_term(key: &str, value: &Value, free: bool) -> bool {
    if value.is_null() {
        return false;
    }
    match key {
        "authMode" | "paymentRequired" | "securitySchemes" | "protocols" | "accepts" | "mode"
        | "pricingMode" | "currency" | "unit" | "network" | "asset" => false,
        // Conditions can describe access to a free operation. A separate price
        // declaration, including unstructured pricing terms, still conflicts.
        "conditions" if free => false,
        "amount" if free => !is_zero_amount(value),
        "pricingTerms" | "conditions" => meaningful_billing_value(value),
        "price" | "fixed" => value.as_object().map_or_else(
            || {
                value.is_number()
                    || free && value.is_boolean()
                    || value.as_str().is_some_and(|text| !text.trim().is_empty())
                    || value.as_array().is_some_and(|terms| !terms.is_empty())
            },
            |fields| {
                fields
                    .iter()
                    .any(|(key, value)| has_tariff_term(key, value, free))
            },
        ),
        _ => true,
    }
}

fn is_zero_amount(value: &Value) -> bool {
    value.as_f64() == Some(0.0)
        || money(value).is_some_and(|amount| amount.bytes().all(|byte| matches!(byte, b'0' | b'.')))
}

fn competes_with_free(block: &Value) -> bool {
    block
        .get("amount")
        .is_some_and(|amount| !is_zero_amount(amount))
        || block["paymentRequired"] == true
        || matches!(block["authMode"].as_str(), Some("paid" | "x402"))
        || block["accepts"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
        || mode_of(block).is_some_and(is_dynamic_mode)
}

fn meaningful_billing_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(terms) => !terms.is_empty(),
        Value::Object(terms) => !terms.is_empty(),
        _ => true,
    }
}

fn dollar_price(value: &Value) -> Option<Value> {
    let text = value["price"].as_str()?;
    let amount = text.strip_prefix('$')?;
    money(&Value::String(amount.into()))?;
    let mut metadata = value.as_object().cloned().unwrap_or_default();
    // Preserve any independent range declaration when normalizing the fixed value.
    metadata.remove("price");
    metadata
        .entry("currency")
        .or_insert_with(|| Value::String("USD".into()));
    metadata.insert("amount".into(), Value::String(amount.into()));
    Some(Value::Object(metadata))
}

fn effective_unit(config: &PricingConfig, declared_unit: Option<&Value>) -> Option<String> {
    match declared_unit {
        Some(unit) => unit
            .as_str()
            .filter(|unit| !unit.is_empty())
            .map(str::to_owned),
        None => config.unit.clone(),
    }
}

fn call_unit(unit: Option<&str>) -> bool {
    unit.is_some_and(|unit| matches!(unit, "call" | "request"))
}

fn unit_suffix(unit: Option<&str>) -> String {
    unit.map_or_else(String::new, |unit| format!("/{unit}"))
}

fn display_amount(amount: String, currency: Option<&str>) -> String {
    match currency {
        Some(code) if code.eq_ignore_ascii_case("USD") || code.eq_ignore_ascii_case("USDC") => {
            format!("${amount}")
        }
        Some(code) => format!("{amount} {code}"),
        None => amount,
    }
}

fn evidence_label(origin: Origin, mode: Option<&Value>, no_unit: bool) -> String {
    let mut label = origin.label().to_owned();
    if let Some(mode) = mode {
        match mode.as_str() {
            Some(mode) if mode.eq_ignore_ascii_case("fixed") => {}
            Some(mode) if mode.eq_ignore_ascii_case("metered") => label.push_str(", metered"),
            _ => label.push_str(", dynamic"),
        }
    }
    if no_unit {
        label.push_str(", no unit");
    }
    label
}

/// Inner tariff wrappers take display precedence, but an explicit outer range
/// still prevents an inner fixed sample from becoming a selection estimate.
fn explicit_range_block(value: &Value) -> Option<&Value> {
    tariff_blocks(value).into_iter().flatten().find(|block| {
        block.is_object() && (block.get("min").is_some() || block.get("max").is_some())
    })
}

fn tariff_blocks(value: &Value) -> [Option<&Value>; 4] {
    [
        value.pointer("/price/fixed"),
        value.get("price"),
        value.get("fixed"),
        Some(value),
    ]
}

/// Both decimal tariffs and atomic alternatives use the same authored wrappers.
fn tariff_block(value: &Value) -> &Value {
    tariff_blocks(value)
        .into_iter()
        .flatten()
        .find(|block| block.is_object())
        .unwrap_or(value)
}

fn mode_of(value: &Value) -> Option<&Value> {
    let mode = value.get("mode");
    let legacy_mode = value.get("pricingMode");
    // A fixed marker cannot make a nonfixed declaration eligible.
    [mode, legacy_mode]
        .into_iter()
        .flatten()
        .find(|mode| is_dynamic_mode(mode))
        .or(mode)
        .or(legacy_mode)
}

fn tariff_mode<'a>(root: &'a Value, block: &'a Value) -> Option<&'a Value> {
    enclosing_value(root, block, mode_of)
}

// The nearest declaration supplies display terms. Every enclosing declaration
// must allow a fixed request estimate.
fn allows_request_estimate(block: &Value) -> bool {
    mode_of(block).is_none_or(|mode| !is_dynamic_mode(mode))
        && block
            .get("unit")
            .is_none_or(|unit| call_unit(unit.as_str()))
}

fn is_dynamic_mode(mode: &Value) -> bool {
    !mode
        .as_str()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("fixed"))
}

/// Metadata follows the selected block outward. Nested samples carry their own metadata.
fn enclosing_blocks<'a>(root: &'a Value, block: &'a Value) -> impl Iterator<Item = &'a Value> {
    let parent = root.get("price").filter(|price| {
        price
            .get("fixed")
            .is_some_and(|fixed| std::ptr::eq(fixed, block))
    });
    [
        Some(block),
        parent,
        (!std::ptr::eq(root, block)).then_some(root),
    ]
    .into_iter()
    .flatten()
}

fn enclosing_value<'a>(
    root: &'a Value,
    block: &'a Value,
    get: impl Fn(&'a Value) -> Option<&'a Value>,
) -> Option<&'a Value> {
    enclosing_blocks(root, block).find_map(get)
}

fn enclosing_field<'a>(root: &'a Value, block: &'a Value, key: &str) -> Option<&'a Value> {
    enclosing_value(root, block, |value| value.get(key))
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
    compatible_rail_fields(value)
        && value.get("protocols").is_none_or(compatible_protocols)
        && ["price", "fixed"]
            .into_iter()
            .filter_map(|key| value.get(key).filter(|child| child.is_object()))
            .all(compatible_rail)
}

fn compatible_protocols(value: &Value) -> bool {
    let Some(protocols) = value.as_array() else {
        return false;
    };
    let mut declared = protocols
        .iter()
        .filter_map(|protocol| protocol.get("x402"))
        .peekable();
    if declared.peek().is_some() {
        declared.any(compatible_x402_protocol)
    } else {
        protocols.iter().any(|protocol| protocol == "x402")
    }
}

fn compatible_rail_fields(value: &Value) -> bool {
    // Explicit contrary rail information never becomes a supported estimate.
    ["network", "caip2Network", "x402Network"]
        .into_iter()
        .all(|key| value.get(key).is_none_or(supported_network))
        && value.get("asset").is_none_or(supported_asset)
}

fn compatible_x402_protocol(value: &Value) -> bool {
    value.is_object()
        && value.get("scheme").is_none_or(|scheme| scheme == "exact")
        && compatible_rail(value)
        && value.get("networks").is_none_or(|networks| {
            networks.as_array().is_some_and(|networks| {
                networks.iter().any(|entry| {
                    entry.get("network").is_some()
                        && entry.get("asset").is_some()
                        && entry.get("scheme").is_none_or(|scheme| scheme == "exact")
                        && compatible_rail_fields(entry)
                })
            })
        })
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
    format_money(value_text(value).trim())
}

// Fractional JSON numbers can describe a range without specifying an exact charge.
fn range_bound(value: &Value) -> Option<String> {
    money(value).or_else(|| {
        value.as_f64().filter(|number| *number >= 0.0)?;
        let text = value.to_string();
        Some(format_money(&text).unwrap_or(text))
    })
}

fn format_money(text: &str) -> Option<String> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\d+(?:\.\d+)?$").expect("constant regex"));
    if !RE.is_match(text) {
        return None;
    }
    Some(if let Some((whole, fraction)) = text.split_once('.') {
        format!("{whole}.{:0<2}", fraction.trim_end_matches('0'))
    } else {
        text.to_owned()
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
    pub(crate) external: bool,
}
impl Advertisement {
    pub(crate) fn external_tariff(&self) -> Option<TariffText> {
        let mut tariff = self.external.then(|| self.price.tariff_text()).flatten()?;
        tariff.exact_offer = self
            .price
            .selection_amount
            .filter(|_| self.price.kind == TariffKind::Fixed);
        Some(tariff)
    }
}

#[derive(Debug, Default)]
struct AdvertisementState {
    invalidated: bool,
    value: Option<Advertisement>,
}
#[derive(Clone, Debug, Default)]
pub struct AdvertisedPricing {
    value: Arc<RwLock<AdvertisementState>>,
    owner: Option<Arc<Acquisition>>,
}
impl AdvertisedPricing {
    #[cfg(test)]
    pub(crate) fn has_acquisition_owner(&self) -> bool {
        self.owner.is_some()
    }

    #[cfg(test)]
    pub(crate) fn acquisition_finished(&self) -> bool {
        self.owner
            .as_ref()
            .is_none_or(|owner| owner.0.is_finished())
    }

    /// Fresh external metadata can enrich presentation after source publication.
    pub fn external_description(&self) -> Option<String> {
        self.with_fresh(|value, _| Some(value?.external_tariff()?.text))
    }

    pub(crate) fn with_fresh<T>(&self, read: impl FnOnce(Option<&Advertisement>, bool) -> T) -> T {
        let state = self.value.read().expect("advertised pricing poisoned");
        let value = state
            .value
            .as_ref()
            .filter(|value| value.fresh_until.is_none_or(|until| Instant::now() < until));
        read(value, state.invalidated)
    }

    /// Original catalog or metadata URL, distinct from observed payment offers.
    pub fn source(&self) -> Option<String> {
        self.value
            .read()
            .expect("advertised pricing poisoned")
            .value
            .as_ref()
            .map(|value| value.source.clone())
    }

    pub fn new(price: NormalizedPrice, source: String, fresh_until: Option<Instant>) -> Self {
        Self {
            value: Arc::new(RwLock::new(AdvertisementState {
                invalidated: false,
                value: Some(Advertisement {
                    price,
                    source,
                    fresh_until,
                    external: false,
                }),
            })),
            owner: None,
        }
    }
    pub fn amount(&self) -> Option<U256> {
        self.with_fresh(|value, _| value.and_then(|value| value.price.selection_amount))
    }
    pub fn replace(&self, price: NormalizedPrice, source: String, fresh_until: Option<Instant>) {
        let mut state = self.value.write().expect("advertised pricing poisoned");
        if state.invalidated {
            return;
        }
        state.value = Some(Advertisement {
            price,
            source,
            fresh_until,
            external: true,
        });
    }
    pub fn set_fresh_until(&self, fresh_until: Option<Instant>) {
        if let Some(value) = self
            .value
            .write()
            .expect("advertised pricing poisoned")
            .value
            .as_mut()
        {
            value.fresh_until = fresh_until;
        }
    }
    pub fn set_expires(&self, expires: u64) {
        self.set_fresh_until(crate::http_cache::advertised_until(expires));
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.value.write().expect("advertised pricing poisoned");
        state.invalidated = true;
        state.value = None;
    }
    /// Serialize durable publication with this generation's terminal invalidation.
    pub(crate) fn while_current<T>(&self, publish: impl FnOnce() -> T) -> Option<T> {
        let state = self.value.read().expect("advertised pricing poisoned");
        (!state.invalidated).then(publish)
    }
    pub fn attach_owner(&mut self, owner: Arc<Acquisition>) {
        self.owner = Some(owner);
    }
}

#[cfg(test)]
#[path = "advertised_tests.rs"]
mod tests;
