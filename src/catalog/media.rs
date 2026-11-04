//! Build-time JSON representation selection, with no network negotiation.
use super::{contracts, expansion};
use anyhow::Result;
use serde_json::{Value, json};
use std::{borrow::Cow, collections::BTreeMap};

pub(crate) fn json_type(value: &str) -> Result<mime::Mime> {
    let media: mime::Mime = value
        .parse()
        .map_err(|_| contracts::Unsupported::RequestMedia)?;
    if media.type_() == mime::STAR
        || media.subtype() == mime::STAR
        || !(media.essence_str().eq_ignore_ascii_case("application/json")
            || media.suffix() == Some(mime::JSON))
        || media.params().any(|(name, value)| {
            name == mime::CHARSET && !value.as_str().eq_ignore_ascii_case("utf-8")
        })
    {
        return Err(contracts::Unsupported::RequestMedia.into());
    }
    reqwest::header::HeaderValue::from_str(value)
        .map_err(|_| contracts::Unsupported::RequestMedia)?;
    Ok(media)
}
fn matches(
    range: &mime::Mime,
    actual: &mime::Mime,
    budget: &mut expansion::Budget,
) -> Result<bool> {
    budget.comparison(range.essence_str(), actual.essence_str())?;
    if !((range.type_() == mime::STAR || range.type_() == actual.type_())
        && ((range.subtype() == mime::STAR && range.suffix().is_none())
            || (range.subtype() == actual.subtype() && range.suffix() == actual.suffix())))
    {
        return Ok(false);
    }
    for (name, value) in range.params() {
        let mut found = false;
        // Match get_param's first-duplicate semantics, charging each inspected
        // name rather than hiding a nested scan inside an uncharged lookup.
        for (other_name, other_value) in actual.params() {
            budget.comparison(name.as_str(), other_name.as_str())?;
            if name == other_name {
                budget.comparison(value.as_str(), other_value.as_str())?;
                if value != other_value {
                    return Ok(false);
                }
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn body(root: &Value, source: &Value, budget: &mut expansion::Budget) -> Result<Value> {
    let source = match contracts::reference(root, source, budget) {
        Ok(source) => source,
        Err(error) => {
            let Some(reason) = error.downcast_ref::<contracts::Unsupported>() else {
                return Err(error);
            };
            return Ok(json!({"required":source["required"]==true,"issue":reason}));
        }
    };
    let Some(content) = source["content"].as_object() else {
        return Ok(
            json!({"required":source["required"]==true,"issue":contracts::Unsupported::RequestMedia}),
        );
    };
    let mut offers = Vec::new();
    for (name, value) in content {
        // Shared request bodies may revisit even malformed/unsupported media
        // names. Charge before the parser traverses and owns their text.
        budget.copying_text(name, 0)?;
        if let Ok(media) = name.parse::<mime::Mime>() {
            offers.push((name, media, value));
        }
    }
    // Mime parsing canonicalizes the essence/type casing. Keep global ordinals
    // because equally specific declarations previously selected the last key.
    let mut exact = BTreeMap::<&str, Vec<usize>>::new();
    let mut type_wildcards = BTreeMap::<&str, Vec<usize>>::new();
    let mut any_type = Vec::new();
    for (ordinal, (_, media, _)) in offers.iter().enumerate() {
        if media.type_() == mime::STAR {
            any_type.push(ordinal);
        } else if media.subtype() == mime::STAR {
            type_wildcards
                .entry(media.type_().as_str())
                .or_default()
                .push(ordinal);
        } else {
            exact.entry(media.essence_str()).or_default().push(ordinal);
        }
    }
    let mut candidates = Vec::<Cow<'_, str>>::new();
    for (name, media, _) in &offers {
        if media.type_() == mime::STAR || media.subtype() == mime::STAR {
            if media.suffix().is_none()
                && (media.type_() == mime::APPLICATION
                    || media.type_() == mime::STAR && media.subtype() == mime::STAR)
            {
                let parameters = name.find(';').map_or("", |index| &name[index..]);
                budget.copying_text("application/json", 0)?;
                budget.copying_text(parameters, 0)?;
                candidates.push(Cow::Owned(format!("application/json{parameters}")));
            }
        } else {
            candidates.push(Cow::Borrowed(name.as_str()));
        }
    }
    let order = |value: &str| {
        !value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    };
    candidates
        .sort_by(|left, right| (order(left), left.as_ref()).cmp(&(order(right), right.as_ref())));
    candidates.dedup();
    let empty = json!({});
    let mut reason = contracts::Unsupported::RequestMedia;
    for candidate in candidates {
        budget.copying_text(&candidate, 0)?;
        let Ok(actual) = json_type(&candidate) else {
            continue;
        };
        // Only ranges that can match this essence need consideration. Parameter
        // subset matching within these buckets remains part of media precedence.
        let mut selected = None;
        for ordinal in exact
            .get(actual.essence_str())
            .into_iter()
            .flatten()
            .chain(
                type_wildcards
                    .get(actual.type_().as_str())
                    .into_iter()
                    .flatten(),
            )
            .chain(any_type.iter())
            .copied()
        {
            let (_, range, offer) = &offers[ordinal];
            if !matches(range, &actual, budget)? {
                continue;
            }
            let priority = (
                usize::from(range.type_() != mime::STAR),
                usize::from(range.subtype() != mime::STAR),
                range.params().count(),
                ordinal,
            );
            if selected.as_ref().is_none_or(|(old, _)| priority > *old) {
                selected = Some((priority, offer));
            }
        }
        let Some((_, offer)) = selected else {
            continue;
        };
        let projected = match contracts::schema(root, offer.get("schema").unwrap_or(&empty), budget)
        {
            Ok(schema) => schema,
            Err(error) => {
                if let Some(unsupported) = error.downcast_ref::<contracts::Unsupported>() {
                    reason = *unsupported;
                    continue;
                }
                return Err(error);
            }
        };
        budget.copying_text(&candidate, 1)?;
        let mut body = json!({"required":source["required"]==true,"media_type":candidate});
        body["schema"] = projected;
        return Ok(body);
    }
    Ok(json!({"required":source["required"]==true,"issue":reason}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_lookup_amplification_uses_the_shared_work_budget() {
        let parameters: Vec<_> = (0..1024).map(|i| format!("; p{i:04}=value")).collect();
        let range: mime::Mime = format!("application/*{}", parameters.concat())
            .parse()
            .unwrap();
        let actual: mime::Mime = format!(
            "application/json{}",
            parameters.into_iter().rev().collect::<String>()
        )
        .parse()
        .unwrap();
        assert!(
            matches(&range, &actual, &mut expansion::Budget::default())
                .unwrap_err()
                .is::<expansion::ComplexityLimit>()
        );
    }

    #[test]
    fn parameter_matching_keeps_case_and_first_duplicate_semantics() {
        let range: mime::Mime = "application/*; profile=kept; charset=UTF-8"
            .parse()
            .unwrap();
        for (actual, expected) in [
            ("application/json; PROFILE=kept; charset=utf-8", true),
            ("application/json; profile=KEPT; charset=utf-8", false),
            ("application/json; profile=other; profile=kept", false),
            (
                "application/json; profile=kept; profile=other; charset=utf-8",
                true,
            ),
        ] {
            assert_eq!(
                matches(
                    &range,
                    &actual.parse().unwrap(),
                    &mut expansion::Budget::default()
                )
                .unwrap(),
                expected,
                "{actual}"
            );
        }
    }
}
