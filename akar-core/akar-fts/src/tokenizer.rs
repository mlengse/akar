//! Tantivy tokenizer integration.
//!
//! Registers the `en_stem` tokenizer used for full-text fields and exposes
//! Tantivy-backed [`stem`]/[`tokenize`] primitives for the FTS extension.
//!
//! The pipeline mirrors Tantivy's built-in `en_stem` tokenizer (split on
//! whitespace/punctuation → remove tokens longer than 40 bytes → lowercase →
//! Snowball-Porter2 stemming). We build it explicitly so index-time and
//! query-time processing always agree, and so per-language variants (P109) can
//! be added without touching the callers.

use tantivy::tokenizer::{
    Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer, TokenizerManager,
};

/// Name of the default English-stemming tokenizer registered on every
/// [`super::index::TantivyIndex`]. Grammar references this name via
/// `CREATE FTS INDEX ... WITH TOKENIZER('en_stem')` (P109).
pub const EN_STEM: &str = "en_stem";

/// Build the `en_stem` [`TextAnalyzer`] pipeline.
///
/// Equivalent to Tantivy's pre-configured `en_stem` tokenizer. Stop-word
/// removal is intentionally **not** included: it matches the built-in behavior
/// and the caller layer (`akar_fts::STOP_WORDS` / `remove_stop_words`) already
/// filters stop words separately.
pub fn en_stem_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::new(Language::English))
        .build()
}

/// A [`TokenizerManager`] pre-registered with Akar's `en_stem` tokenizer,
/// replacing the identically-named built-in so both reference this pipeline.
///
/// Also retains Tantivy's built-in managers (`default`, `raw`, `whitespace`,
/// `simple`). Installed on every [`super::index::TantivyIndex`].
pub fn manager() -> TokenizerManager {
    let manager = TokenizerManager::default();
    manager.register(EN_STEM, en_stem_analyzer());
    manager
}

/// Stem a single word with the `en_stem` pipeline.
///
/// Returns the trimmed, lowercased input when it produces no token (empty
/// input, or a word longer than the 40-character tokenizer limit).
pub fn stem(word: &str) -> String {
    let mut analyzer = en_stem_analyzer();
    let mut stream = analyzer.token_stream(word);
    match stream.next() {
        Some(token) => token.text.clone(),
        None => word.trim().to_lowercase(),
    }
}

/// Tokenize `text` into lowercased, stemmed word tokens using the `en_stem`
/// pipeline. Same tokens a full-text field indexed with `en_stem` produces.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut analyzer = en_stem_analyzer();
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while let Some(token) = stream.next() {
        tokens.push(token.text.clone());
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::TokenStream;

    #[test]
    fn test_stem_single_word() {
        assert_eq!(stem("running"), "run");
        assert_eq!(stem("quickly"), "quick");
        assert_eq!(stem("is"), "is");
    }

    #[test]
    fn test_tokenize_running_quickly() {
        assert_eq!(tokenize("Running quickly"), vec!["run", "quick"]);
    }

    #[test]
    fn test_tokenize_empty() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn test_manager_registers_en_stem() {
        let mut analyzer = manager().get(EN_STEM).expect("en_stem registered");
        let mut stream = analyzer.token_stream("Running quickly");
        let tokens: Vec<String> = std::iter::from_fn(|| stream.next().map(|t| t.text.clone())).collect();
        assert_eq!(tokens, vec!["run", "quick"]);
    }
}
