//! Full-Text Search (FTS) extension for Akar.
//!
//! Enables full-text indexing and querying:
//! - `STEM` — stem words using a Porter-style English stemmer
//! - `TOKENIZE` — tokenize text into words
//!
//! FTS index creation and querying are handled **natively** via the DDL and
//! MATCH clause (`CREATE FTS INDEX`, `MATCH ... USING FTS INDEX`), which
//! bypass the extension function registry. The library functions below
//! (stem_word, tokenize, bm25, etc.) are called directly by the physical
//! operators in `Akar-processor`.

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

        // Register `stem(word)` — applies Porter-style stemming
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

/// A minimal Porter-style stemmer for English.
///
/// Removes common suffixes: -ing, -ly, -ed, -es, -s, -ment, -ness, -tion, -able, -ible, -al, -ial, -ful, -ous, -ive, -ize, -er, -or, -ion
pub fn stem_word(word: &str) -> String {
    let word = word.trim().to_lowercase();
    if word.len() < 3 {
        return word;
    }

    let w = word;

    // Rule 1a: -sses → -ss, -ies → -i, -es → -e, -s → (remove s if not -ss)
    let w = if w.ends_with("sses") {
        w[..w.len() - 2].to_string()
    } else if w.ends_with("ies") && w.len() > 4 {
        format!("{}i", &w[..w.len() - 3])
    } else if w.ends_with("es") && w.len() > 4 && !w.ends_with("aes") && !w.ends_with("ees") && !w.ends_with("oes") {
        format!("{}e", &w[..w.len() - 2])
    } else if w.ends_with("s") && !w.ends_with("ss") && w.len() > 3 {
        w[..w.len() - 1].to_string()
    } else {
        w
    };

    // Rule 1b: -eed → -ee if root has consonant before
    //          -ed → remove (if root has vowel)
    //          -ing → remove (if root has vowel)
    let w = if w.ends_with("eed") && w.len() > 3 {
        format!("{}ee", &w[..w.len() - 3])
    } else if w.ends_with("ed") && w.len() > 3 && contains_vowel(&w[..w.len() - 2]) {
        let stem = &w[..w.len() - 2];
        handle_double_consonant(stem)
    } else if w.ends_with("ing") && w.len() > 4 && contains_vowel(&w[..w.len() - 3]) {
        let stem = &w[..w.len() - 3];
        handle_double_consonant(stem)
    } else if w.ends_with("ingly") && w.len() > 5 && contains_vowel(&w[..w.len() - 5]) {
        let stem = &w[..w.len() - 5];
        handle_double_consonant(stem)
    } else if w.ends_with("edly") && w.len() > 4 && contains_vowel(&w[..w.len() - 4]) {
        let stem = &w[..w.len() - 4];
        handle_double_consonant(stem)
    } else {
        w
    };

    // Rule 2: -ational → -ate, -ization → -ize, -iveness → -ive, etc.

    if w.ends_with("ational") && w.len() > 7 {
        format!("{}ate", &w[..w.len() - 7])
    } else if w.ends_with("ization") && w.len() > 8 {
        format!("{}ize", &w[..w.len() - 8])
    } else if w.ends_with("iveness") && w.len() > 7 {
        format!("{}ive", &w[..w.len() - 7])
    } else if w.ends_with("fulness") && w.len() > 7 {
        format!("{}ful", &w[..w.len() - 7])
    } else if w.ends_with("ousness") && w.len() > 7 {
        format!("{}ous", &w[..w.len() - 7])
    } else if w.ends_with("biliti") && w.len() > 6 {
        format!("{}ble", &w[..w.len() - 6])
    } else if w.ends_with("ation") && w.len() > 5 {
        format!("{}ate", &w[..w.len() - 5])
    } else if w.ends_with("ment") && w.len() > 4 {
        w[..w.len() - 4].to_string()
    } else if w.ends_with("ness") && w.len() > 4 {
        w[..w.len() - 4].to_string()
    } else if w.ends_with("able") && w.len() > 4 {
        w[..w.len() - 4].to_string()
    } else if w.ends_with("ible") && w.len() > 4 {
        w[..w.len() - 4].to_string()
    } else if w.ends_with("ment") && w.len() > 4 {
        w[..w.len() - 4].to_string()
    } else if w.ends_with("ful") && w.len() > 3 {
        w[..w.len() - 3].to_string()
    } else if w.ends_with("al") && w.len() > 3 {
        w[..w.len() - 2].to_string()
    } else if w.ends_with("ive") && w.len() > 3 {
        w[..w.len() - 3].to_string()
    } else if w.ends_with("ize") && w.len() > 3 {
        w[..w.len() - 3].to_string()
    } else if w.ends_with("er") && w.len() > 3 {
        w[..w.len() - 2].to_string()
    } else if w.ends_with("or") && w.len() > 3 {
        w[..w.len() - 2].to_string()
    } else if w.ends_with("ion") && w.len() > 3 {
        w[..w.len() - 3].to_string()
    } else if w.ends_with("ly") && w.len() > 3 {
        w[..w.len() - 2].to_string()
    } else {
        w
    }
}

/// Check if a string slice contains a vowel.
fn contains_vowel(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u'))
}

/// Handle double consonant at end of stem: if stem ends in double consonant,
/// remove one (e.g., "runn" → "run").
fn handle_double_consonant(stem: &str) -> String {
    if stem.len() >= 2 {
        let chars: Vec<char> = stem.chars().collect();
        let last = chars.len() - 1;
        if last > 0 && chars[last] == chars[last - 1] {
            return chars[..last].iter().collect();
        }
    }
    stem.to_string()
}

// ==================== Tokenization ====================

/// Tokenize a text string into words.
/// Splits on whitespace and punctuation, lowercases all tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"[a-zA-Z0-9]+([''][a-zA-Z]+)?").unwrap();
    re.find_iter(text).map(|m| m.as_str().to_lowercase()).collect()
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
        assert_eq!(stem_word("quickly"), "quick");
        assert_eq!(stem_word("happily"), "happi");
        assert_eq!(stem_word("slowly"), "slow");
    }

    #[test]
    fn test_stem_ment_ness() {
        assert_eq!(stem_word("enjoyment"), "enjoy");
        assert_eq!(stem_word("happiness"), "happi");
        assert_eq!(stem_word("goodness"), "good");
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
        let tokens = tokenize("It's a nice day, isn't it?");
        assert!(tokens.iter().any(|t| t == "it's"));
        assert!(tokens.iter().any(|t| t == "isn't"));
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
        // "justification" → "justificate" (Porter step 2: -ation → -ate)
        // A full Porter2 stemmer would further reduce this to "justif"
        assert!(stem_word("justification").starts_with("justif"));
    }

    #[test]
    fn test_fts_extension_name() {
        let ext = FtsExtension::new();
        assert_eq!(ext.name(), "FTS");
    }
}
