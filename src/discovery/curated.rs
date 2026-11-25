//! Accepted provider contracts, independent of the currently shipped bundle.
use crate::catalog::{self, Config};
use anyhow::{Context, Result};
use pulldown_cmark::{Event, LinkType, Parser, Tag};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, ops::Range, sync::OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedProvider {
    pub id: String,
    pub settings: Config,
}

impl AcceptedProvider {
    pub fn current(id: &str) -> Result<Self> {
        let provider = crate::providers::get(id).context("unknown_provider_id")?;
        Ok(Self::new(id, provider.config.clone()))
    }
    pub fn new(id: &str, mut settings: Config) -> Self {
        // Accepted bytes replace static discovery, warming and probe machinery.
        settings.http_cache_enabled = false;
        settings.use_recommended_tools = false;
        settings.cover_traffic = None;
        Self {
            id: id.into(),
            settings,
        }
    }
}

/// Select only an unambiguous full-specification match; guidance follows exposed routes.
pub(super) fn select_spec_match(
    matches: &[&crate::providers::Provider],
    local_available: bool,
) -> Result<Option<AcceptedProvider>> {
    match matches {
        [] => Ok(None),
        [provider] => Ok(Some(AcceptedProvider::new(
            provider.id,
            provider.config.clone(),
        ))),
        _ if local_available => anyhow::bail!(
            "ambiguous specification; use providers_add_local with provider_id from: {}",
            matches.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
        ),
        _ => anyhow::bail!(
            "ambiguous specification; ask the operator to resolve the bundled definition selection"
        ),
    }
}

pub(super) fn local_document(provider: &AcceptedProvider) -> Result<Vec<u8>> {
    let shipped = crate::providers::get(&provider.id).context("unknown_provider_id")?;
    anyhow::ensure!(
        shipped.config.spec == provider.settings.spec,
        "bundled_schema_reference_changed"
    );
    Ok(shipped
        .local_schema
        .context("bundled_schema_missing")?
        .as_bytes()
        .to_vec())
}

pub(super) fn build(
    provider: &AcceptedProvider,
    name: &str,
    root: &Value,
) -> Result<(catalog::CatalogBuild, Option<String>)> {
    super::import::validate_source_name(name)?;
    crate::config::validate(&provider.settings)?;
    let mut settings = provider.settings.clone();
    settings.use_recommended_tools = false;
    let canonical = settings.prefix.as_deref().unwrap_or(&provider.id);
    let mut built = catalog::compile_catalog(&settings, root, canonical)?;
    let tools = &mut built.tools;
    let prefix = format!("{canonical}_");
    let dynamic = super::import::tool_prefix(name)?;
    let mut names = BTreeMap::from([(format!("{canonical}_*"), format!("{dynamic}*"))]);
    for tool in tools.iter() {
        let suffix = tool
            .name
            .strip_prefix(&prefix)
            .context("provider tool prefix mismatch")?;
        names.insert(tool.name.clone(), format!("{dynamic}{suffix}"));
    }
    for tool in tools.iter_mut() {
        tool.name = names[&tool.name].clone();
        let replace = |text: &str| rebind(text, &names);
        if let Some(rendering) = &mut tool.description_rendering {
            rendering.rebind(replace);
        }
        catalog::rebind_schema_descriptions(&mut tool.input_schema, &replace);
    }
    built.render_descriptions();
    let instructions = settings
        .instructions_text
        .as_ref()
        .map(|text| rebind(text, &names));
    Ok((built, instructions))
}

// The parser supplies editable spans. Replacements preserve every source byte
// outside a complete, known tool reference.
fn rebind(text: &str, names: &BTreeMap<String, String>) -> String {
    if !text.contains('_') {
        return text.to_owned();
    }
    let prose = Prose::parse(text);
    let mut output = String::with_capacity(text.len());
    let mut offset = 0;
    let mut previous_reference_end = None;
    for range in &prose.ranges {
        if range.bytes.end <= offset {
            continue;
        }
        let start = range.bytes.start.max(offset);
        output.push_str(&text[offset..start]);
        offset = start;
        while let Some(captures) = symbols().captures_at(&text[..range.bytes.end], offset) {
            let token = captures.get(0).expect("matched token");
            output.push_str(&text[offset..token.start()]);
            let (end, replacement) =
                prose.edit_token(text, names, range, &captures, previous_reference_end.take());
            offset = end;
            if let Some(replacement) = replacement {
                output.push_str(&replacement);
                previous_reference_end = Some(end);
            } else {
                output.push_str(&text[token.start()..end]);
            }
            if offset >= range.bytes.end {
                break;
            }
        }
        if offset < range.bytes.end {
            output.push_str(&text[offset..range.bytes.end]);
            offset = range.bytes.end;
        }
    }
    output.push_str(&text[offset..]);
    output
}

struct ProseRange {
    bytes: Range<usize>,
    scope_end: usize,
    code: bool,
}

#[derive(Default)]
struct Prose {
    ranges: Vec<ProseRange>,
    address_boundaries: Vec<usize>,
    link_labels: BTreeMap<usize, usize>,
}

impl Prose {
    fn parse(text: &str) -> Self {
        use pulldown_cmark::TagEnd;

        let mut prose = Self::default();
        let mut emphasis = Vec::new();
        let mut protected_labels = Vec::new();
        for (event, span) in Parser::new(text).into_offset_iter() {
            match event {
                Event::Start(Tag::Emphasis | Tag::Strong) => emphasis.push(span.end),
                Event::End(TagEnd::Emphasis | TagEnd::Strong) => {
                    emphasis.pop();
                }
                Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. }) => {
                    prose.address_boundaries.push(span.start);
                    protected_labels.push(
                        protected_labels.last() == Some(&true)
                            || !matches!(link_type, LinkType::Inline | LinkType::Reference),
                    );
                }
                Event::End(TagEnd::Link | TagEnd::Image) => {
                    prose.address_boundaries.push(span.end);
                    if protected_labels.pop() == Some(false)
                        && let Some(last) = prose.ranges.last()
                        && last.bytes.start >= span.start
                    {
                        prose.link_labels.insert(span.end, last.bytes.end);
                    }
                }
                Event::Code(_) if protected_labels.last() != Some(&true) => {
                    prose.address_boundaries.extend([span.start, span.end]);
                    let fence = text[span.clone()]
                        .bytes()
                        .take_while(|&b| b == b'`')
                        .count();
                    prose.ranges.push(ProseRange {
                        bytes: span.start + fence..span.end - fence,
                        scope_end: span.end - fence,
                        code: true,
                    });
                }
                Event::InlineHtml(_) | Event::Html(_) => {
                    prose.address_boundaries.extend([span.start, span.end]);
                }
                Event::Text(_) if protected_labels.last() != Some(&true) => {
                    let scope_end = emphasis.last().copied().unwrap_or(text.len());
                    if let Some(last) = prose.ranges.last_mut()
                        && last.scope_end == scope_end
                        && !last.code
                        && (last.bytes.end == span.start
                            || text.get(last.bytes.end..span.start) == Some("\\"))
                    {
                        last.bytes.end = span.end;
                    } else {
                        prose.ranges.push(ProseRange {
                            bytes: span,
                            scope_end,
                            code: false,
                        });
                    }
                }
                _ => {}
            }
        }
        prose.address_boundaries.sort_unstable();
        prose
    }

    // Return the boundary to copy for an address, or the replacement for a
    // complete reference. Address scanning can span text leaves within a scope.
    fn edit_token<'a>(
        &self,
        text: &str,
        names: &'a BTreeMap<String, String>,
        range: &ProseRange,
        captures: &regex::Captures<'_>,
        previous: Option<usize>,
    ) -> (usize, Option<std::borrow::Cow<'a, str>>) {
        let token = captures.get(0).expect("matched token");
        let end = token.end();
        let tail = &text[end..range.bytes.end];
        if let Some(address) = captures.name("address") {
            if address.as_str() == "/"
                && self.joins_references(text, names, previous, token.start())
            {
                return (end, None);
            }
        } else if let Some(path) = captures.name("path") {
            if (!path_suffix(tail)
                || tail.starts_with('/') && self.joins_references(text, names, Some(end), end))
                && let Some(references) = replace_path(names, path.as_str(), !range.code)
            {
                return (end, Some(references.into()));
            }
        } else if !tail.starts_with('/')
            || self.address_boundaries.binary_search(&(end + 1)).is_ok()
            || self.joins_references(text, names, Some(end), end)
        {
            return (end, replacement(names, token.as_str(), !range.code));
        }
        (self.address_end(text, range, end), None)
    }

    // Parser text can split a raw URL at punctuation, entities, or emphasis
    // within the URL. Links, inline code, and HTML also define address
    // boundaries, even when the URL starts outside them.
    fn address_end(&self, text: &str, range: &ProseRange, start: usize) -> usize {
        let next = self
            .address_boundaries
            .partition_point(|&boundary| boundary < start);
        let end = self
            .address_boundaries
            .get(next)
            .copied()
            .unwrap_or(text.len());
        let url = &text[start..end.min(range.scope_end)];
        start
            + url
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '}'))
                .unwrap_or(url.len())
    }

    fn joins_references(
        &self,
        text: &str,
        names: &BTreeMap<String, String>,
        previous: Option<usize>,
        slash: usize,
    ) -> bool {
        let Some(previous) = previous else {
            return false;
        };
        let left_end = self.link_labels.get(&slash).copied().unwrap_or(slash);
        if previous > left_end || !inline_delimiters(&text[previous..left_end]) {
            return false;
        }
        let next = self
            .ranges
            .partition_point(|range| range.bytes.end <= slash + 1);
        let Some(range) = self.ranges.get(next) else {
            return false;
        };
        let start = range.bytes.start.max(slash + 1);
        if !inline_delimiters(&text[slash + 1..start]) {
            return false;
        }
        let Some(captures) = symbols().captures_at(&text[..range.bytes.end], start) else {
            return false;
        };
        let token = captures.get(0).expect("matched token");
        let untrimmed = &text[token.end()..];
        let (tail, closes_inline) = if token.end() == range.bytes.end {
            let trimmed = untrimmed.trim_start_matches(['`', '*', '_']);
            (trimmed, trimmed.len() < untrimmed.len())
        } else {
            (&text[token.end()..range.bytes.end], false)
        };
        token.start() == start
            && captures.name("address").is_none()
            && (!path_suffix(tail) || closes_inline && tail.starts_with('/'))
            && if let Some(path) = captures.name("path") {
                replace_path(names, path.as_str(), !range.code).is_some()
            } else {
                replacement(names, token.as_str(), !range.code).is_some()
            }
    }
}

fn path_suffix(text: &str) -> bool {
    text.starts_with(['?', '#', '/', '%', '@'])
        || text.starts_with(char::is_alphanumeric)
        || text
            .strip_prefix('.')
            .is_some_and(|suffix| suffix.starts_with(char::is_alphanumeric))
}

fn inline_delimiters(text: &str) -> bool {
    text.bytes().all(|byte| matches!(byte, b'`' | b'*' | b'_'))
}

fn replacement<'a>(
    names: &'a BTreeMap<String, String>,
    token: &str,
    escaped_wildcard: bool,
) -> Option<std::borrow::Cow<'a, str>> {
    if let Some(name) = names.get(token) {
        return Some(std::borrow::Cow::Borrowed(name));
    }
    let prefix = escaped_wildcard
        .then(|| token.strip_suffix(r"\*"))
        .flatten()?;
    let name = names.get(&format!("{prefix}*"))?;
    Some(std::borrow::Cow::Owned(format!(
        "{}\\*",
        name.strip_suffix('*')?
    )))
}

fn replace_path(names: &BTreeMap<String, String>, path: &str, escapes: bool) -> Option<String> {
    let references = path
        .split('/')
        .map(|name| replacement(names, name, escapes))
        .collect::<Option<Vec<_>>>()?;
    Some(references.join("/"))
}

fn symbols() -> &'static regex::Regex {
    const IDENTIFIER: &str = r"[A-Za-z0-9_-]+(?:\\?\*[A-Za-z0-9_-]*)*";
    static SYMBOLS: OnceLock<regex::Regex> = OnceLock::new();
    SYMBOLS.get_or_init(|| {
        regex::Regex::new(
            &[
                r"(?x)
                (?P<address>
                    [A-Za-z][A-Za-z0-9+.-]*://
                    | (?i:mailto):
                    | \./|\.\./|/
                    | (?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,}(?::[0-9]+)?[/\#?]
                    | [A-Za-z0-9_.+-]+@(?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,}
                )
                | (?P<path>
            ",
                IDENTIFIER,
                "(?:/",
                IDENTIFIER,
                ")+) | ",
                IDENTIFIER,
            ]
            .concat(),
        )
        .expect("tool reference and URL tokens")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_spans_own_the_bounds_of_a_raw_address() {
        let names = BTreeMap::from([
            ("original_read".into(), "dyn_named__read".into()),
            ("original_help".into(), "dyn_named__help".into()),
            ("original_*".into(), "dyn_named__*".into()),
        ]);
        for (input, expected) in [
            (
                "`https://docs.test/original_read`,`original_help`",
                "`https://docs.test/original_read`,`dyn_named__help`",
            ),
            (
                "*https://docs.test/original_read*original_help",
                "*https://docs.test/original_read*dyn_named__help",
            ),
            (
                "[https://docs.test/original_read](relative)original_help",
                "[https://docs.test/original_read](relative)dyn_named__help",
            ),
            (
                "https://docs.test/path[original_read](relative)original_help",
                "https://docs.test/path[dyn_named__read](relative)dyn_named__help",
            ),
            (
                "https://docs.test/v1/*original_read*&amp;tool=original_help",
                "https://docs.test/v1/*original_read*&amp;tool=original_help",
            ),
            (
                "[![original_read](icon)](relative)/original_help",
                "[![dyn_named__read](icon)](relative)/dyn_named__help",
            ),
            (
                "original_read/[original_help](relative)",
                "dyn_named__read/[dyn_named__help](relative)",
            ),
            (
                "original_read/original_help.json. original_read/original_help.",
                "original_read/original_help.json. dyn_named__read/dyn_named__help.",
            ),
            (
                "`original_read`/original_help?option=x",
                "`dyn_named__read`/original_help?option=x",
            ),
            (
                "original_read/original_help/original_read",
                "dyn_named__read/dyn_named__help/dyn_named__read",
            ),
            (
                "original_read/**original_help**/original_read",
                "dyn_named__read/**dyn_named__help**/dyn_named__read",
            ),
            (
                r"original_read/original_\* or original_\*/original_read or original_read/original_\*\*",
                r"dyn_named__read/dyn_named__\* or dyn_named__\*/dyn_named__read or original_read/original_\*\*",
            ),
            (
                "`original_read`/**original_help**.json",
                "`dyn_named__read`/**original_help**.json",
            ),
            (
                r"**original_\*** original_*suffix original_\*\* `original_\*`",
                r"**dyn_named__\*** original_*suffix original_\*\* `original_\*`",
            ),
            (
                "original_read@example.test docs.test:8080?tool=original_help MAILTO:?subject=original_read original_help",
                "original_read@example.test docs.test:8080?tool=original_help MAILTO:?subject=original_read dyn_named__help",
            ),
        ] {
            assert_eq!(rebind(input, &names), expected, "{input:?}");
        }
    }
}
