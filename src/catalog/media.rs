//! Build-time JSON representation selection, with no network negotiation.
use super::{contracts, expansion};
use anyhow::Result;
use serde_json::{Value, json};

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
fn matches(range: &mime::Mime, actual: &mime::Mime) -> bool {
    (range.type_() == mime::STAR || range.type_() == actual.type_())
        && ((range.subtype() == mime::STAR && range.suffix().is_none())
            || (range.subtype() == actual.subtype() && range.suffix() == actual.suffix()))
        && range
            .params()
            .all(|(name, value)| actual.get_param(name).is_some_and(|other| other == value))
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
    let offers: Vec<_> = content
        .iter()
        .filter_map(|(name, value)| {
            name.parse::<mime::Mime>()
                .ok()
                .map(|media| (name, media, value))
        })
        .collect();
    let mut candidates = Vec::new();
    for (name, media, _) in &offers {
        if media.type_() == mime::STAR || media.subtype() == mime::STAR {
            if media.suffix().is_none()
                && (media.type_() == mime::APPLICATION
                    || media.type_() == mime::STAR && media.subtype() == mime::STAR)
            {
                let parameters = name.find(';').map_or("", |index| &name[index..]);
                candidates.push(format!("application/json{parameters}"));
            }
        } else {
            candidates.push(name.to_string());
        }
    }
    candidates.sort_by_key(|value| {
        (
            !value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json"),
            value.clone(),
        )
    });
    candidates.dedup();
    let empty = json!({});
    let mut reason = contracts::Unsupported::RequestMedia;
    for candidate in candidates {
        let Ok(actual) = json_type(&candidate) else {
            continue;
        };
        let selected = offers
            .iter()
            .filter(|(_, range, _)| matches(range, &actual))
            .max_by_key(|(_, range, _)| {
                (
                    usize::from(range.type_() != mime::STAR),
                    usize::from(range.subtype() != mime::STAR),
                    range.params().count(),
                )
            });
        let Some((_, _, offer)) = selected else {
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
        let mut body = json!({"required":source["required"]==true,"media_type":candidate});
        body["schema"] = projected;
        return Ok(body);
    }
    Ok(json!({"required":source["required"]==true,"issue":reason}))
}
