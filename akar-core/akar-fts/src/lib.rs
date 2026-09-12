//! Full-Text Search (FTS) extension for Akar.
//!
//! Enables full-text indexing and querying:
//! - `STEM` — stem words with Tantivy's `en_stem` (Snowball Porter2) tokenizer
//! - `TOKENIZE` — tokenize text into lowercased, stemmed word tokens
//!
//! FTS index creation and querying are handled **natively** via the DDL and
//! MATCH clause (`CREATE FTS INDEX`, `MATCH ... USING FTS INDEX`), which
//! bypass the extension function registry. The library functions below
//! (stem_word, tokenize, bm25, etc.) are called directly by the physical
//! operators in `Akar-processor`.

pub mod build;
pub mod index;
pub mod schema;
pub mod tokenizer;

use akar_extension::{Extension, ExtensionContext};
use std::sync::Arc;

/// The FTS extension adds full-text search capabilities to Akar.
pub struct FtsExtension;

impl Default for FtsExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl FtsExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for FtsExtension {
    fn name(&self) -> &'static str {
        "FTS"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use akar_common::types::Value;
        use akar_function::registry::ScalarFunction;

        // Register `stem(word)` — applies Tantivy en_stem (Porter2) stemming
        context.register_scalar_function(
            "stem",
            ScalarFunction::CustomScalar {
                name: "stem".into(),
                execute: Arc::new(|args: &[Value]| -> Result<Value, String> {
                    let word = match args.first() {
                        Some(Value::String(s)) => s.clone(),
                        _ => return Err("stem: expected 1 string argument".into()),
                    };
                    Ok(Value::String(stem_word(&word)))
                }),
            },
        );

        // Register `tokenize(text)` — splits text into lowercase word tokens
        context.register_scalar_function(
            "tokenize",
            ScalarFunction::CustomScalar {
                name: "tokenize".into(),
                execute: Arc::new(|args: &[Value]| -> Result<Value, String> {
                    let text = match args.first() {
                        Some(Value::String(s)) => s.clone(),
                        _ => return Err("tokenize: expected 1 string argument".into()),
                    };
                    let tokens: Vec<Value> = tokenize(&text).into_iter().map(Value::String).collect();
                    Ok(Value::List(tokens))
                }),
            },
        );

        // FTS index creation and querying are handled natively via:
        //   CREATE FTS INDEX ...  (DDL → PhysicalCreateFtsIndex)
        //   MATCH ... USING FTS INDEX ... (PhysicalFtsScan + BM25)
        // These extension table functions are informational stubs for
        // CALL-based discovery (e.g., `CALL show_functions()`).

        tracing::info!("FTS extension loaded: stem, tokenize (scalar) + native DDL/MATCH FTS pipeline");

        Ok(())
    }
}

// ==================== Stemming ====================

/// Stem a single English word using the Tantivy `en_stem` tokenizer
/// (Snowball Porter2-derived). Applied uniformly at index and query time by
/// the FTS physical operators in `Akar-processor`.
pub fn stem_word(word: &str) -> String {
    crate::tokenizer::stem(word)
}

// ==================== Tokenization ====================

/// Tokenize a text string into lowercased, stemmed word tokens.
///
/// Delegates to the Tantivy `en_stem` pipeline, so the tokens equal what a
/// full-text field indexed with `en_stem` produces.
pub fn tokenize(text: &str) -> Vec<String> {
    crate::tokenizer::tokenize(text)
}

/// Compute TF-IDF score for a term in a document.
pub fn tf_idf(term_freq: f64, doc_count: usize, total_docs: usize) -> f64 {
    if total_docs == 0 || doc_count == 0 {
        return 0.0;
    }
    let idf = ((total_docs as f64) / (doc_count as f64)).ln();
    term_freq * idf
}

/// Calculate BM25 score for a term.
///
/// Reference closed-form implementation. The live FTS scan delegates scoring
/// to Tantivy's own BM25 (k1=1.2, b=0.75); scoring parity is asserted in
/// `index::tests` (`test_bm25_scoring_parity_plan_case`, `test_bm25_length_normalization`).
/// Note that Tantivy scores with a *quantized* per-doc length (fieldnorm) and
/// the raw mean token count as avgdl — see those tests for the exact mapping.
pub fn bm25(
    term_freq: f64,
    doc_length: f64,
    avg_doc_length: f64,
    doc_freq: f64,
    total_docs: f64,
    k1: f64,
    b: f64,
) -> f64 {
    if doc_freq == 0.0 || total_docs == 0.0 {
        return 0.0;
    }
    let idf = ((total_docs - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln();
    let tf_part = (term_freq * (k1 + 1.0)) / (term_freq + k1 * (1.0 - b + b * doc_length / avg_doc_length));
    idf * tf_part
}

/// Common English stop words to filter out during indexing.
pub const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "by", "with", "from", "as", "is", "was",
    "are", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would", "can", "could",
    "shall", "should", "may", "might", "must", "not", "no", "nor", "none", "i", "you", "he", "she", "it", "we", "they",
    "me", "him", "her", "us", "them", "my", "your", "his", "its", "our", "their", "this", "that", "these", "those",
    "what", "which", "who", "whom", "whose", "when", "where", "why", "how", "all", "each", "every", "both", "few",
    "more", "most", "some", "any", "such", "only", "own", "same", "so", "than", "too", "very", "just", "about",
    "above", "after", "again", "against", "below", "between", "into", "through", "during", "before", "after", "then",
    "once", "here", "there",
];

/// Filter stop words from a list of tokens.
pub fn remove_stop_words(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .collect()
}

/// Build a term frequency map from tokenized text.
pub fn term_frequencies(tokens: &[String]) -> Vec<(String, usize)> {
    let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for token in tokens {
        *freq.entry(token.clone()).or_insert(0) += 1;
    }
    let mut result: Vec<(String, usize)> = freq.into_iter().collect();
    result.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stem_simple() {
        // Simplified stemmer
        assert_eq!(stem_word("running"), "run");
        assert_eq!(stem_word("walks"), "walk");
        assert_eq!(stem_word("walked"), "walk");
    }

    #[test]
    fn test_stem_ly() {
        // Tantivy Snowball-Porter2 keeps the trailing "-li" from step 1c (y→i).
        assert_eq!(stem_word("quickly"), "quick");
        assert_eq!(stem_word("happily"), "happili");
        assert_eq!(stem_word("slowly"), "slowli");
    }

    #[test]
    fn test_stem_ment_ness() {
        assert_eq!(stem_word("enjoyment"), "enjoy");
        assert_eq!(stem_word("happiness"), "happi");
        assert_eq!(stem_word("goodness"), "good");
    }

    #[test]
    fn test_stem_ingly_edly_after_ing() {
        // -ingly/-edly are separate branches and must still fire: a word ending
        // in "ingly" does not end in "ing" (last three chars are "gly").
        assert_eq!(stem_word("amazingly"), "amaz");
        assert_eq!(stem_word("reportedly"), "report");
    }

    #[test]
    fn test_stem_short_words() {
        assert_eq!(stem_word("is"), "is");
        assert_eq!(stem_word("be"), "be");
        assert_eq!(stem_word("go"), "go");
    }

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World! This is a test.");
        assert_eq!(tokens, vec!["hello", "world", "this", "is", "a", "test"]);
    }

    #[test]
    fn test_tokenize_with_punctuation() {
        // Tantivy SimpleTokenizer splits on non-alphanumeric chars (so
        // apostrophes split the word), then lowercases + stems each token.
        let tokens = tokenize("It's a nice day, isn't it?");
        assert_eq!(tokens, vec!["it", "s", "a", "nice", "day", "isn", "t", "it"]);
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_remove_stop_words() {
        let tokens = vec![
            "the".to_string(),
            "quick".to_string(),
            "brown".to_string(),
            "fox".to_string(),
        ];
        let filtered = remove_stop_words(tokens);
        assert_eq!(filtered, vec!["quick", "brown", "fox"]);
    }

    #[test]
    fn test_term_frequencies() {
        let tokens = vec!["a".into(), "b".into(), "a".into(), "c".into(), "a".into(), "b".into()];
        let freqs = term_frequencies(&tokens);
        assert_eq!(freqs[0], ("a".to_string(), 3));
        assert_eq!(freqs[1], ("b".to_string(), 2));
        assert_eq!(freqs[2], ("c".to_string(), 1));
    }

    #[test]
    fn test_bm25_score() {
        let score = bm25(2.0, 100.0, 80.0, 5.0, 1000.0, 1.2, 0.75);
        assert!(score > 0.0);
        assert!(score < 10.0);
    }

    #[test]
    fn test_tf_idf_score() {
        let score = tf_idf(2.0, 5, 100);
        assert!(score > 0.0);
        // Term appears in 5 of 100 docs → idf = ln(100/5) ≈ 3.0, tf=2 → score≈6.0
        assert!((score - 6.0).abs() < 1.0);
    }

    #[test]
    fn test_tf_idf_all_docs() {
        let score = tf_idf(1.0, 100, 100);
        assert!((score - 0.0).abs() < 1e-10); // ln(100/100) = 0
    }

    #[test]
    fn test_stem_complex() {
        assert_eq!(stem_word("happiness"), "happi");
        assert_eq!(stem_word("enjoyment"), "enjoy");
        // "justification" → "justif" (Tantivy Snowball-Porter2, step 2: -ation → -ate
        // then step 4: -ate → remove).
        assert_eq!(stem_word("justification"), "justif");
    }

    #[test]
    fn test_fts_extension_name() {
        let ext = FtsExtension::new();
        assert_eq!(ext.name(), "FTS");
    }
}
