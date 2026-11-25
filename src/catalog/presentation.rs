use super::{Config, DEFAULT_PRICE, ToolSpec};
use crate::{
    output::ObservedPrice,
    pricing::advertised::{NormalizedPrice, TariffKind},
};
use serde_json::Value;

/// The compiled fields are local to a catalog generation. A serialized tool
/// retains only its rendered canonical description.
#[derive(Clone, Debug)]
pub struct DescriptionRendering {
    pub(crate) prose: String,
    pub(crate) authored_price: Option<String>,
    advertised: Option<String>,
    credit_terms: Option<String>,
    max_chars: Option<usize>,
    help: bool,
}

impl DescriptionRendering {
    pub(super) fn operation(
        cfg: &Config,
        op: &Value,
        credit_terms: Option<String>,
        advertised: &NormalizedPrice,
    ) -> Self {
        let text = op["description"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| op["summary"].as_str().filter(|s| !s.is_empty()))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    op["method"].as_str().unwrap().to_uppercase(),
                    op["path"].as_str().unwrap()
                )
            });
        let tariff = match advertised.kind {
            TariffKind::Other
                if !op["pricing"].is_string() && !op["pricing"]["price"].is_string() =>
            {
                DEFAULT_PRICE
            }
            _ => advertised.description(),
        };
        let combined = credit_terms.as_ref().map(|credits| {
            if tariff == DEFAULT_PRICE {
                credits.clone()
            } else {
                format!("{tariff} {credits}")
            }
        });
        let prose = combined
            .as_ref()
            .and_then(|suffix| text.strip_suffix(suffix))
            .or_else(|| text.strip_suffix(tariff))
            .map_or(text.as_str(), str::trim_end)
            .to_owned();
        Self {
            prose,
            advertised: (tariff != DEFAULT_PRICE).then(|| tariff.to_owned()),
            authored_price: None,
            credit_terms,
            max_chars: cfg.max_description_chars,
            help: false,
        }
    }

    pub(super) fn help(cfg: &Config, prose: String) -> Self {
        Self {
            prose,
            advertised: None,
            authored_price: None,
            credit_terms: None,
            max_chars: cfg.max_description_chars,
            help: true,
        }
    }

    pub(super) fn canonical(&self) -> String {
        let mut text = self.prose.clone();
        if self.help {
            return truncate_description(&text, self.max_chars);
        }
        if let Some(tariff) = &self.advertised {
            append(&mut text, tariff);
        } else if self.authored_price.is_none() && self.credit_terms.is_none() {
            append(&mut text, DEFAULT_PRICE);
        }
        if let Some(note) = &self.authored_price {
            if self.advertised.as_ref() == Some(note) {
                append(&mut text, "[also configured]");
            } else {
                append(&mut text, &format!("Configured pricing: {note}"));
            }
        }
        if let Some(credits) = &self.credit_terms {
            append(&mut text, credits);
        }
        if let Some(max) = self.max_chars
            && text.chars().count() > max
        {
            tracing::warn!(
                category = "tool_description_truncated",
                limit_chars = max,
                "The configured character limit shortened a tool description; the description includes a truncation marker"
            );
        }
        truncate_description(&text, self.max_chars)
    }
}

impl ToolSpec {
    /// Present a scoped observation without changing the canonical catalog.
    pub fn render_description(&self, observed: Option<ObservedPrice>) -> String {
        if self.advertised_pricing.invalidated() {
            return self.description_rendering.as_ref().map_or_else(
                || self.description.clone(),
                |rendering| truncate_description(&rendering.prose, rendering.max_chars),
            );
        }
        let fragment = observed
            .map(|observed| format!("Observed cost: ~${observed}/call."))
            .or_else(|| self.advertised_pricing.external_description());
        let Some(fragment) = fragment else {
            return self.description.clone();
        };
        let (base, max) = self
            .description_rendering
            .as_ref()
            .map_or((self.description.as_str(), None), |rendering| {
                (rendering.prose.as_str(), rendering.max_chars)
            });
        let mut text = base.to_owned();
        append(&mut text, &fragment);
        truncate_description(&text, max)
    }
}

fn append(text: &mut String, fragment: &str) {
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(fragment);
}

fn truncate_description(description: &str, max: Option<usize>) -> String {
    match max {
        Some(max) if description.chars().count() > max => format!(
            "{}\n[truncated by max_description_chars={max}]",
            description.chars().take(max).collect::<String>()
        ),
        _ => description.to_owned(),
    }
}
