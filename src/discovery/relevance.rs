//! Request-local BM25 over the caller's already-filtered catalog snapshot.
use crate::catalog_state::BoundTool;
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

struct Document<'a> {
    tool: &'a BoundTool,
    literal: String,
    frequencies: BTreeMap<String, usize>,
    length: usize,
}
impl<'a> Document<'a> {
    fn new(tool: &'a BoundTool) -> Self {
        let literal = format!("{} {}", tool.tool.name, tool.tool.description).to_lowercase();
        let parameters = tool.tool.input_schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|properties| properties.keys())
            .flat_map(|name| tokenize(name));
        let mut frequencies = BTreeMap::new();
        for term in tokenize(&literal).into_iter().chain(parameters) {
            *frequencies.entry(term).or_default() += 1;
        }
        let length = frequencies.values().sum();
        Self {
            tool,
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

pub(super) fn rank<'a>(tools: Vec<&'a BoundTool>, query: &str) -> Vec<&'a BoundTool> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || tools.is_empty() {
        return tools;
    }
    let documents: Vec<_> = tools.into_iter().map(Document::new).collect();
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
            (doc.tool, literal, score)
        })
        .filter(|(_, literal, score)| *literal || *score > 0.0)
        .collect();
    matches.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.2.total_cmp(&a.2))
            .then_with(|| a.0.tool.name.cmp(&b.0.tool.name))
    });
    matches.into_iter().map(|(tool, _, _)| tool).collect()
}

#[cfg(test)]
#[path = "relevance_tests.rs"]
mod tests;
