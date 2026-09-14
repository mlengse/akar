//! Tantivy-free BM25 scoring for the fused (native) full-text path.
//!
//! `akar-fts` uses Tantivy for index *construction*; the fused hybrid search
//! path must stay Tantivy-free at query time. This module provides a small,
//! dependency-free in-memory inverted index with a standard Okapi BM25 scorer
//! so hybrid scans can rank full-text hits without touching Tantivy.

use std::collections::HashMap;

/// Okapi BM25 tuning parameters. The defaults are the classic ones.
#[derive(Debug, Clone, Copy)]
pub struct Bm25Params {
    /// Term-frequency saturation (default 1.2).
    pub k1: f64,
    /// Document-length normalization (default 0.75).
    pub b: f64,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// One document's identity and length, as seen by the index.
#[derive(Debug, Clone, Copy)]
pub struct Bm25Doc {
    pub id: u64,
    pub doc_len: usize,
}

/// Postings for a single term: document id → term frequency.
type Postings = HashMap<u64, u32>;

/// A single term's inverted entry.
#[derive(Debug)]
struct TermIndex {
    df: usize,
    postings: Postings,
}

/// In-memory inverted index over tokenized documents.
#[derive(Debug, Default)]
pub struct NativeBm25Index {
    terms: HashMap<String, TermIndex>,
    /// doc id → number of tokens in that document.
    doc_lens: HashMap<u64, usize>,
    total_tokens: u64,
    num_docs: usize,
    avg_doc_len: f64,
}

/// Lowercase + split on non-alphanumeric characters. A lightweight tokenizer
/// suitable for the native path (fuzzy tokenization parity with Tantivy is
/// the job of `akar-fts`, not the fused path).
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

impl NativeBm25Index {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a pre-tokenized document.
    pub fn add_document(&mut self, id: u64, tokens: &[String]) {
        let mut local: HashMap<String, u32> = HashMap::new();
        for t in tokens {
            *local.entry(t.clone()).or_insert(0) += 1;
        }
        for (term, tf) in local {
            let entry = self.terms.entry(term).or_insert_with(|| TermIndex {
                df: 0,
                postings: Postings::new(),
            });
            if !entry.postings.contains_key(&id) {
                entry.df += 1;
            }
            entry.postings.insert(id, tf);
        }
        let doc_len = tokens.len();
        self.total_tokens += doc_len as u64;
        self.doc_lens.insert(id, doc_len);
        self.num_docs += 1;
        self.avg_doc_len = if self.num_docs > 0 {
            self.total_tokens as f64 / self.num_docs as f64
        } else {
            0.0
        };
    }

    /// Number of documents in the index.
    pub fn num_docs(&self) -> usize {
        self.num_docs
    }

    /// Average document length (in tokens) across the index.
    pub fn avg_doc_len(&self) -> f64 {
        self.avg_doc_len
    }

    /// Document frequency of a term (0 if absent).
    pub fn doc_freq(&self, term: &str) -> usize {
        self.terms.get(term).map(|t| t.df).unwrap_or(0)
    }

    /// Rank every document by BM25 score for a multi-term query.
    ///
    /// Returns `(doc_id, score)` pairs sorted by descending score. Documents
    /// that match no query term are omitted.
    pub fn score_docs(&self, query_terms: &[String], params: Bm25Params) -> Vec<(u64, f64)> {
        let n = self.num_docs.max(1) as f64;
        let avg_dl = if self.avg_doc_len > 0.0 { self.avg_doc_len } else { 1.0 };

        let mut scores: HashMap<u64, f64> = HashMap::new();
        for term in query_terms {
            let Some(entry) = self.terms.get(term) else { continue };
            if entry.postings.is_empty() {
                continue;
            }
            // BM25+ idf (additive smoothing avoids negative idf on dense terms).
            let df = entry.df as f64;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for (&doc_id, &tf) in &entry.postings {
                let dl = *self.doc_lens.get(&doc_id).unwrap_or(&1) as f64;
                let tf_norm =
                    tf as f64 * (params.k1 + 1.0) / (tf as f64 + params.k1 * (1.0 - params.b + params.b * dl / avg_dl));
                *scores.entry(doc_id).or_insert(0.0) += idf * tf_norm;
            }
        }

        let mut ranked: Vec<(u64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked
    }

    /// Convenience builder from `(id, tokens)` pairs.
    pub fn built_from(docs: &[(u64, &[String])]) -> Self {
        let mut index = Self::new();
        for (id, tokens) in docs {
            index.add_document(*id, tokens);
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(s: &str) -> Vec<String> {
        tokenize(s)
    }

    #[test]
    fn test_tokenize() {
        assert_eq!(tokens("Hello, World!"), vec!["hello", "world"]);
        assert_eq!(tokens("FooBar 123"), vec!["foobar", "123"]);
        assert_eq!(tokens("already-lower"), vec!["already", "lower"]);
        assert!(tokens("!@#$%").is_empty());
    }

    #[test]
    fn test_doc_freq_and_counts() {
        let mut index = NativeBm25Index::new();
        index.add_document(1, &tokens("the quick brown fox"));
        index.add_document(2, &tokens("the lazy dog"));
        assert_eq!(index.num_docs(), 2);
        assert_eq!(index.doc_freq("the"), 2);
        assert_eq!(index.doc_freq("fox"), 1);
        assert_eq!(index.doc_freq("missing"), 0);
        assert!((index.avg_doc_len() - 3.5).abs() < 1e-12);
    }

    #[test]
    fn test_score_docs_ranks_exact_term_first() {
        let mut index = NativeBm25Index::new();
        index.add_document(1, &tokens("the cat sat"));
        index.add_document(2, &tokens("the cat sat on the mat"));
        index.add_document(3, &tokens("completely unrelated content"));

        let results = index.score_docs(&tokens("cat sat"), Bm25Params::default());
        assert_eq!(results.len(), 2);
        // Doc 1 is shorter and both terms present → higher BM25.
        assert_eq!(results[0].0, 1);
        assert!(results[0].1 > results[1].1);
        assert!(!results.iter().any(|(id, _)| *id == 3));
    }

    #[test]
    fn test_score_docs_multi_term_accumulates() {
        let mut index = NativeBm25Index::new();
        index.add_document(1, &tokens("rust database"));
        index.add_document(2, &tokens("rust"));
        index.add_document(3, &tokens("nothing here"));
        // Doc 1 matches both terms → strictly higher than doc 2.
        let results = index.score_docs(&["rust".to_string(), "database".to_string()], Bm25Params::default());
        assert!(results[0].0 == 1, "expected doc 1 first, got {results:?}");
    }

    #[test]
    fn test_built_from() {
        let index = NativeBm25Index::built_from(&[(10, &tokens("alpha beta")), (20, &tokens("beta gamma"))]);
        assert_eq!(index.num_docs(), 2);
        assert_eq!(index.doc_freq("beta"), 2);
        assert_eq!(index.doc_freq("alpha"), 1);
    }

    #[test]
    fn test_no_matches_yields_empty() {
        let mut index = NativeBm25Index::new();
        index.add_document(1, &tokens("hello world"));
        assert!(index.score_docs(&tokens("zzz"), Bm25Params::default()).is_empty());
    }
}
