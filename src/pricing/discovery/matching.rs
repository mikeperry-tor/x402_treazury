//! Parsed identities and candidate buckets for the original operation inventory.
use super::Operation;
use std::collections::BTreeMap;

type Group = (String, Option<String>, Option<String>);
type Anchor = (usize, Option<(usize, String)>);

struct Template {
    identity: usize,
    segments: Vec<(String, bool)>,
}

#[derive(Default)]
struct Bucket {
    exact: BTreeMap<String, Vec<usize>>,
    templates: BTreeMap<Anchor, Vec<usize>>,
}

#[derive(Default)]
pub(super) struct Index {
    groups: BTreeMap<Group, Bucket>,
    templates: Vec<Template>,
    #[cfg(test)]
    pub parses: std::cell::Cell<usize>,
    #[cfg(test)]
    pub visits: std::cell::Cell<usize>,
}

fn segments(url: &reqwest::Url) -> Option<Vec<&str>> {
    if url.fragment().is_some() {
        return None;
    }
    Some(url.path_segments()?.collect())
}

fn parameter(segment: &str) -> bool {
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    decoded.starts_with('{') && decoded.ends_with('}') && decoded.len() > 2
}

impl Index {
    fn parse(&self, resource: &str) -> Option<reqwest::Url> {
        #[cfg(test)]
        self.parses.set(self.parses.get() + 1);
        reqwest::Url::parse(resource).ok()
    }

    pub fn new(operations: &[Operation]) -> Self {
        let mut index = Self::default();
        let parsed = operations
            .iter()
            .enumerate()
            .filter_map(|(identity, operation)| {
                let url = index.parse(&operation.resource)?;
                let segments = segments(&url)?
                    .iter()
                    .map(|segment| ((*segment).to_owned(), parameter(segment)))
                    .collect::<Vec<_>>();
                Some((identity, url, segments))
            })
            .collect::<Vec<_>>();
        let mut frequencies = BTreeMap::<Anchor, usize>::new();
        for (_, _, segments) in &parsed {
            if segments.iter().any(|(_, parameter)| *parameter) {
                for (position, (literal, parameter)) in segments.iter().enumerate() {
                    if !parameter {
                        *frequencies
                            .entry((segments.len(), Some((position, literal.clone()))))
                            .or_default() += 1;
                    }
                }
            }
        }
        for (identity, url, segments) in parsed {
            let template = segments.iter().any(|(_, parameter)| *parameter).then(|| {
                // Choose the least shared static segment, rather than a common
                // API prefix. All-placeholder templates necessarily overlap.
                let anchor = segments
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, parameter))| !parameter)
                    .map(|(position, (literal, _))| {
                        (segments.len(), Some((position, literal.clone())))
                    })
                    .min_by_key(|anchor| frequencies[anchor]);
                let id = index.templates.len();
                let key = anchor.unwrap_or((segments.len(), None));
                index.templates.push(Template { identity, segments });
                (key, id)
            });
            let operation = &operations[identity];
            for method in [None, Some(operation.method.clone())] {
                let group = (
                    url.origin().ascii_serialization(),
                    url.query().map(str::to_owned),
                    method,
                );
                let bucket = index.groups.entry(group).or_default();
                if let Some((anchor, id)) = &template {
                    bucket
                        .templates
                        .entry(anchor.clone())
                        .or_default()
                        .push(*id);
                } else {
                    let matches = bucket.exact.entry(url.path().to_owned()).or_default();
                    // Retain enough identities to prove ambiguity, including duplicates.
                    if matches.len() < 2 {
                        matches.push(identity);
                    }
                }
            }
        }
        index
    }

    pub fn candidates(&self, resource: &str, method: Option<&str>) -> Vec<usize> {
        let Some(url) = self.parse(resource) else {
            return Vec::new();
        };
        let Some(segments) = segments(&url) else {
            return Vec::new();
        };
        let key = (
            url.origin().ascii_serialization(),
            url.query().map(str::to_owned),
            method.map(str::to_ascii_uppercase),
        );
        let Some(bucket) = self.groups.get(&key) else {
            return Vec::new();
        };
        let mut matches = bucket.exact.get(url.path()).cloned().unwrap_or_default();
        if matches.len() == 2 {
            return matches;
        }
        let values = segments
            .iter()
            .map(|value| {
                let ordinary = !value.is_empty()
                    && !percent_encoding::percent_decode_str(value)
                        .decode_utf8_lossy()
                        .contains(['{', '}']);
                (*value, ordinary)
            })
            .collect::<Vec<_>>();
        let anchors =
            std::iter::once((segments.len(), None)).chain(segments.iter().enumerate().map(
                |(position, literal)| (segments.len(), Some((position, (*literal).to_owned()))),
            ));
        for anchor in anchors {
            let Some(candidates) = bucket.templates.get(&anchor) else {
                continue;
            };
            for candidate in candidates {
                #[cfg(test)]
                self.visits.set(self.visits.get() + 1);
                let template = &self.templates[*candidate];
                if template.segments.iter().zip(&values).all(
                    |((literal, parameter), (value, ordinary))| {
                        literal.as_str() == *value || (*parameter && *ordinary)
                    },
                ) {
                    matches.push(template.identity);
                    if matches.len() == 2 {
                        return matches;
                    }
                }
            }
        }
        matches
    }
}
