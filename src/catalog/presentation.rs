use super::{Config, ToolSpec};
use crate::{
    output::ObservedPrice,
    pricing::advertised::{NormalizedPrice, TariffText, UNKNOWN_PRICE, external_offer},
};
use serde_json::Value;

/// The compiled fields are local to a catalog generation. A serialized tool
/// retains only its rendered canonical description.
#[derive(Clone, Debug)]
pub struct DescriptionRendering {
    pub(crate) prose: String,
    pub(crate) authored_price: Option<String>,
    advertised: Option<TariffText>,
    credit_terms: Option<String>,
    max_chars: Option<usize>,
    local_catalog: bool,
    help: bool,
}

impl DescriptionRendering {
    pub(super) fn operation(
        cfg: &Config,
        op: &Value,
        credit_terms: Option<String>,
        advertised: &NormalizedPrice,
        covered_credit_tariff: bool,
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
        let tariff = advertised.tariff_text();
        let combined = credit_terms.as_ref().map(|credits| {
            tariff.as_ref().map_or_else(
                || credits.clone(),
                |tariff| format!("{} {credits}", tariff.text),
            )
        });
        let prose = combined
            .as_ref()
            .and_then(|suffix| text.strip_suffix(suffix))
            .or_else(|| text.strip_suffix(advertised.description()))
            .or_else(|| {
                let tariff = tariff.as_ref()?;
                let start = text.rfind("Cost: ")?;
                let (prose, suffix) = text.split_at(start);
                (tariff
                    .strip_duplicate(suffix)
                    .is_some_and(|rest| rest.is_empty())
                    && (prose.is_empty() || prose.ends_with(char::is_whitespace)))
                .then_some(prose)
            })
            .map_or(text.as_str(), str::trim_end)
            .to_owned();
        Self {
            prose,
            advertised: tariff.filter(|_| !covered_credit_tariff),
            authored_price: None,
            credit_terms,
            max_chars: cfg.max_description_chars,
            local_catalog: !cfg.spec.starts_with("http://") && !cfg.spec.starts_with("https://"),
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
            local_catalog: true,
            help: true,
        }
    }

    pub(crate) fn canonical(&self, tool: &str) -> String {
        let rendered = self.compose(self.advertised.as_ref(), true, None);
        if let Some(max) = self.max_chars
            && rendered.chars().count() > max
        {
            tracing::warn!(
                category = "tool_description_truncated",
                tool,
                setting = "max_description_chars",
                limit_chars = max,
                "The description for {tool} exceeded max_description_chars={max}. It includes a truncation marker"
            );
        }
        truncate_description(&rendered, self.max_chars)
    }

    fn compose(
        &self,
        tariff: Option<&TariffText>,
        use_credits: bool,
        observed: Option<ObservedPrice>,
    ) -> String {
        let mut text = self.prose.clone();
        if self.help {
            return text;
        }
        let credits = self.credit_terms.as_deref().filter(|_| use_credits);
        let note = self
            .authored_price
            .as_deref()
            .filter(|note| !note.trim().is_empty());
        if let Some(tariff) = tariff {
            append(&mut text, &tariff.text);
        } else if note.is_none() && credits.is_none() && observed.is_none() {
            append(&mut text, UNKNOWN_PRICE);
        }
        if let Some(note) = note
            .map(|note| {
                tariff
                    .and_then(|tariff| tariff.strip_duplicate(note))
                    .unwrap_or(note)
            })
            .filter(|note| !note.is_empty())
        {
            if (tariff.is_some() || credits.is_some() || observed.is_some())
                && contains_dollar_amount(note)
            {
                append(&mut text, &format!("Configured pricing: {note}"));
            } else {
                append(&mut text, note);
            }
        }
        if let Some(credits) = credits {
            append(&mut text, credits);
        }
        append_sample(&mut text, observed);
        text
    }
}

impl ToolSpec {
    /// Present evidence from the current generation alongside canonical fields.
    pub fn render_description(&self, observed: Option<ObservedPrice>) -> String {
        if self.help_url.is_some() {
            return self.description.clone();
        }
        let (current, retired) = self.advertised_pricing.with_fresh(|advertised, retired| {
            (
                advertised.map(|tariff| (tariff.external, tariff.external_tariff())),
                retired,
            )
        });
        let embedded = matches!(current, Some((false, _)));
        let external = current.as_ref().and_then(|(_, tariff)| tariff.as_ref());
        let observed = observed.filter(|_| !retired);
        if let Some(rendering) = &self.description_rendering {
            if embedded && observed.is_none() {
                return self.description.clone();
            }
            let tariff = if embedded {
                rendering.advertised.as_ref()
            } else {
                external
            };
            let credits_current = !retired && (current.is_some() || rendering.local_catalog);
            let text = rendering.compose(tariff, credits_current, observed);
            truncate_description(&text, rendering.max_chars)
        } else {
            let mut text = self.description.clone();
            if let Some(external) = external {
                append(&mut text, &external.text);
            }
            append_sample(&mut text, observed);
            text
        }
    }
}

impl TariffText {
    fn strip_duplicate<'a>(&self, note: &'a str) -> Option<&'a str> {
        if let Some(rest) = self.after_generated_fragment(note, &self.text) {
            return Some(rest);
        }

        let listed = self.text.strip_prefix("Listed: ")?;
        let (headline, tail) = listed.split_once("].")?;
        let header = &listed[..headline.len() + "].".len()];
        let rest = note
            .strip_prefix("Listed: ")
            .and_then(|note| after_fragment(note, header))
            .or_else(|| {
                self.legacy_listing
                    .as_deref()
                    .and_then(|legacy| after_fragment(note, legacy))
            })
            .or_else(|| after_legacy_listed_price(note, headline, self.usdc))?;
        Some(
            self.after_generated_fragment(rest, tail.trim_start())
                .unwrap_or(rest),
        )
    }

    fn after_generated_fragment<'a>(&self, note: &'a str, fragment: &str) -> Option<&'a str> {
        if let Some(rest) = after_fragment(note, fragment) {
            return Some(rest);
        }
        let amount = self.exact_offer?;
        let terms = fragment.strip_suffix(&external_offer(amount, true))?;
        // Match a copied offer at the start of the note or after identical terms.
        let note = after_fragment(note, terms.trim_end()).unwrap_or(note);
        let amount = format!("~${}", ObservedPrice { amount });
        if let Some(legacy) = note.strip_prefix("Cost: ") {
            return after_fragment(legacy.strip_prefix(&amount)?, "/call [spec].");
        }
        let note = note
            .strip_prefix("Advertised exact offer: ")?
            .strip_prefix(&amount)?;
        let tags = note
            .strip_prefix("/request [external pricing")
            .or_else(|| note.strip_prefix("/call [external pricing"))?;
        after_fragment(tags.strip_prefix(", USDC").unwrap_or(tags), "].")
    }
}

fn after_legacy_listed_price<'a>(
    note: &'a str,
    headline: &str,
    tariff_usdc: bool,
) -> Option<&'a str> {
    // Recognize the old generated heading and labels for existing overrides.
    // Keep any additional vendor terms and require the complete price to match.
    let (legacy, rest) = note.strip_prefix("Cost: ")?.split_once("].")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (amount, tags) = legacy.rsplit_once(" [")?;
    let (listed_amount, listed_tags) = headline.rsplit_once(" [")?;
    let (amount, listed_tags) = match amount.strip_suffix(" (unit unspecified)") {
        Some(amount) => (amount, listed_tags.strip_suffix(", no unit")?),
        None => (amount, listed_tags),
    };
    let tags = tags.split(", ");
    (amount == listed_amount
        && (tariff_usdc || !tags.clone().any(|tag| tag == "USDC"))
        && tags
            .filter(|tag| *tag != "USDC")
            .eq(listed_tags.split(", ")))
    .then(|| rest.trim_start())
}

fn after_fragment<'a>(text: &'a str, fragment: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(fragment)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}

fn contains_dollar_amount(note: &str) -> bool {
    note.match_indices('$').any(|(offset, _)| {
        note.as_bytes()
            .get(offset + 1)
            .is_some_and(u8::is_ascii_digit)
    })
}

fn append_sample(text: &mut String, observed: Option<ObservedPrice>) {
    if let Some(observed) = observed {
        append(text, &format!("Sample offer: ${observed} for one request."));
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
