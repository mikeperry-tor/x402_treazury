//! Shared request-local BM25 over the caller's already-scoped search candidates.
use std::collections::{BTreeMap, BTreeSet};

const K1: f64 = 1.2;
const B: f64 = 0.75;

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

struct Document<T> {
    item: T,
    literal: String,
    frequencies: BTreeMap<String, usize>,
    length: usize,
}
impl<T> Document<T> {
    fn new(item: T, literal: String, additional: String) -> Self {
        let literal = literal.to_lowercase();
        let mut frequencies = BTreeMap::new();
        for term in tokenize(&literal).into_iter().chain(tokenize(&additional)) {
            *frequencies.entry(term).or_default() += 1;
        }
        let length = frequencies.values().sum();
        Self {
            item,
            literal,
            frequencies,
            length,
        }
    }

    fn score(&self, weights: &[(String, f64)], average_length: f64) -> f64 {
        weights
            .iter()
            .filter_map(|(term, idf)| {
                let frequency = *self.frequencies.get(term)? as f64;
                // A positive frequency implies a positive corpus average length.
                Some(
                    idf * frequency * (K1 + 1.0)
                        / (frequency + K1 * (1.0 - B + B * self.length as f64 / average_length)),
                )
            })
            .sum()
    }
}

/// Project literal-match text plus additional token-only text; ties use the stable key.
/// Blank queries preserve enumeration without projecting or scoring candidates.
pub(crate) fn rank<T>(
    items: Vec<T>,
    query: &str,
    text: impl Fn(&T) -> (String, String),
    key: impl Fn(&T) -> &str,
) -> Vec<T> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || items.is_empty() {
        return items;
    }
    let documents: Vec<_> = items
        .into_iter()
        .map(|item| {
            let (literal, additional) = text(&item);
            Document::new(item, literal, additional)
        })
        .collect();
    let average_length =
        documents.iter().map(|doc| doc.length).sum::<usize>() as f64 / documents.len() as f64;
    let terms: BTreeSet<_> = tokenize(&query).into_iter().collect();
    let weights: Vec<_> = terms
        .into_iter()
        .map(|term| {
            let frequency = documents
                .iter()
                .filter(|doc| doc.frequencies.contains_key(&term))
                .count() as f64;
            // Positive IDF variant, also used by Lucene BM25Similarity.
            let idf = (1.0 + (documents.len() as f64 - frequency + 0.5) / (frequency + 0.5)).ln();
            (term, idf)
        })
        .collect();
    let mut matches: Vec<_> = documents
        .into_iter()
        .map(|doc| {
            let literal = doc.literal.contains(&query);
            let score = doc.score(&weights, average_length);
            (doc.item, literal, score)
        })
        .filter(|(_, literal, score)| *literal || *score > 0.0)
        .collect();
    matches.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.2.total_cmp(&a.2))
            .then_with(|| key(&a.0).cmp(key(&b.0)))
    });
    matches.into_iter().map(|(item, _, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenization_splits_unicode_alphanumeric_terms() {
        assert_eq!(
            tokenize("BlueSky_FOO-bar/地域"),
            ["bluesky", "foo", "bar", "地域"]
        );
    }

    #[test]
    fn bm25_score_matches_reference_frequency_and_length_normalization() {
        let document = Document::new((), "alpha yelp yelp reviews".into(), "business_id".into());
        // Six terms, yelp frequency two, average length five, N=3 and df=2:
        // positive IDF ln(1.6), saturated TF 4.4 / 3.38 for k1=1.2, b=.75.
        let score = document.score(&[("yelp".into(), 1.6_f64.ln())], 5.0);
        assert!((score - 1.6_f64.ln() * 4.4 / 3.38).abs() < 1e-12);
    }
}
