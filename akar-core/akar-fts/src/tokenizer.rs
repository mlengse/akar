//! Tantivy tokenizer integration.
//!
//! Registers the tokenizers used for full-text fields and exposes
//! Tantivy-backed [`stem`]/[`tokenize`] primitives for the FTS extension; the
//! full list is [`SUPPORTED_TOKENIZERS`].
//!
//! - `en_stem` (the default) mirrors Tantivy's built-in `en_stem` tokenizer
//!   (split on whitespace/punctuation → remove tokens longer than 40 bytes →
//!   lowercase → Snowball-Porter2 stemming).
//! - `cjk` (P109.2) handles Chinese / Japanese / Korean, scripts written without
//!   spaces, via Tantivy's [`NgramTokenizer`] over character 1- and 2-grams.
//!
//! We build the pipelines explicitly so index-time and query-time processing
//! always agree, and so per-language variants (P109) can be added without
//! touching the callers.

use tantivy::tokenizer::{
    Language, LowerCaser, NgramTokenizer, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer, TokenizerManager,
};

/// Name of the default English-stemming tokenizer registered on every
/// [`super::index::TantivyIndex`]. Grammar references this name via
/// `CREATE FTS INDEX ... WITH TOKENIZER('en_stem')` (P109).
pub const EN_STEM: &str = "en_stem";

/// Name of the CJK (Chinese / Japanese / Korean) character n-gram tokenizer
/// (P109.2).
///
/// These scripts are written without spaces, so word-oriented tokenizers
/// cannot split them; Tantivy's [`NgramTokenizer`] instead indexes every
/// character and every adjacent character pair, which is the standard technique
/// for CJK full-text search. Selectable via
/// `CREATE FTS INDEX ... WITH TOKENIZER('cjk')`.
pub const CJK: &str = "cjk";

/// Tokenizers selectable via `CREATE FTS INDEX ... WITH TOKENIZER('<name>')`
/// (P109.1/P109.2). `en_stem` (Akar's English-stemming pipeline, the default),
/// `cjk` (character n-gram for Han / Kana / Hangul) plus the Tantivy built-ins
/// that [`manager`] also exposes: `default` (simple + lowercase), `raw`
/// (verbatim tokens — exact-match searching) and `whitespace` (split on
/// whitespace only). Everything [`manager`] registers is resolved from the
/// same registry at index- and query-time, so the persisted schema always
/// finds its tokenizer after a reopen.
pub const SUPPORTED_TOKENIZERS: &[&str] = &[EN_STEM, CJK, "default", "raw", "whitespace"];

/// Whether `name` resolves to a registered tokenizer.
pub fn is_supported(name: &str) -> bool {
    SUPPORTED_TOKENIZERS.contains(&name)
}

/// Resolve an optional `WITH TOKENIZER(...)` value to a concrete tokenizer name,
/// defaulting to [`EN_STEM`] when the clause was omitted (P109.1).
///
/// Returns an error naming the supported set when `name` is not registered.
pub fn resolve(name: Option<&str>) -> Result<String, String> {
    let name = name.unwrap_or(EN_STEM).to_string();
    if is_supported(&name) {
        Ok(name)
    } else {
        Err(format!(
            "Unknown FTS tokenizer '{name}' — supported: {}",
            SUPPORTED_TOKENIZERS.join(", ")
        ))
    }
}

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

/// Build the `cjk` [`TextAnalyzer`] pipeline (P109.2).
///
/// Tantivy [`NgramTokenizer`] over **character 1- and 2-grams**: every Han /
/// Kana / Hangul character is indexed both on its own and as part of its
/// rightward bigram, so queries match on single characters (e.g. `我`) and
/// character pairs (e.g. `机器`) alike. `LowerCaser` normalizes embedded Latin
/// (CJK scripts have no case). Token positions are all 0 — a documented
/// [`NgramTokenizer`] property — so phrase queries do not apply to CJK fields.
pub fn cjk_analyzer() -> TextAnalyzer {
    // min_gram=1, max_gram=2 is a valid NgramTokenizer configuration by
    // construction (min_gram > 0 and min_gram <= max_gram), so the error branch
    // of NgramTokenizer::new is unreachable for these constants.
    let ngram = NgramTokenizer::all_ngrams(1, 2).expect("NgramTokenizer(1, 2) is always a valid configuration");
    TextAnalyzer::builder(ngram).filter(LowerCaser).build()
}

/// A [`TokenizerManager`] pre-registered with Akar's `en_stem` and `cjk`
/// tokenizers, replacing the identically-named `en_stem` built-in so both
/// reference these pipelines.
///
/// Also retains Tantivy's built-in managers (`default`, `raw`, `whitespace`,
/// `simple`). Installed on every [`super::index::TantivyIndex`].
pub fn manager() -> TokenizerManager {
    let manager = TokenizerManager::default();
    manager.register(EN_STEM, en_stem_analyzer());
    manager.register(CJK, cjk_analyzer());
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

    /// P109.1 — `manager()` must expose every name `resolve()` accepts, so a
    /// persisted schema can always find its tokenizer again after a reopen.
    #[test]
    fn test_manager_registers_all_supported() {
        for name in SUPPORTED_TOKENIZERS {
            let mut analyzer = manager()
                .get(name)
                .unwrap_or_else(|| panic!("missing tokenizer '{name}'"));
            let mut stream = analyzer.token_stream("Running quickly");
            let _ = std::iter::from_fn(|| stream.next().map(|t| t.text.clone())).collect::<Vec<_>>();
        }
    }

    #[test]
    fn test_manager_registers_en_stem() {
        let mut analyzer = manager().get(EN_STEM).expect("en_stem registered");
        let mut stream = analyzer.token_stream("Running quickly");
        let tokens: Vec<String> = std::iter::from_fn(|| stream.next().map(|t| t.text.clone())).collect();
        assert_eq!(tokens, vec!["run", "quick"]);
    }

    #[test]
    fn test_resolve_defaults_to_en_stem() {
        assert_eq!(resolve(None).unwrap(), EN_STEM);
        assert_eq!(resolve(Some("raw")).unwrap(), "raw");
        assert_eq!(resolve(Some(CJK)).unwrap(), CJK);
        assert!(resolve(Some("klingon")).is_err(), "unknown tokenizer must be rejected");
    }

    /// P109.2 — the `cjk` pipeline tokenizes a Han script into character 1- and
    /// 2-grams: every char alone plus each adjacent pair, lowercase-normalized.
    #[test]
    fn test_cjk_analyzer_chinese() {
        let mut analyzer = manager().get(CJK).expect("cjk registered");
        let mut stream = analyzer.token_stream("数据库");
        let tokens: Vec<String> = std::iter::from_fn(|| stream.next().map(|t| t.text.clone())).collect();
        assert_eq!(tokens, vec!["数", "数据", "据", "据库", "库"]);
    }

    /// P109.2 — the same character n-gram tokenization applies to Japanese
    /// (katakana/hiragana) and Korean (Hangul). No spaces to split on in any of
    /// these scripts, so the searchable unit is the character / character pair.
    #[test]
    fn test_cjk_analyzer_japanese_korean() {
        let expect = |text: &str| {
            let mut analyzer = manager().get(CJK).expect("cjk registered");
            let mut stream = analyzer.token_stream(text);
            std::iter::from_fn(|| stream.next().map(|t| t.text.clone())).collect::<Vec<String>>()
        };
        assert_eq!(expect("カタナ"), vec!["カ", "カタ", "タ", "タナ", "ナ"]);
        assert_eq!(
            expect("안녕하세요"),
            vec!["안", "안녕", "녕", "녕하", "하", "하세", "세", "세요", "요"]
        );
        // CJK has no case, but embedded Latin is lowercased like the other
        // pipelines.
        assert_eq!(expect("本an"), vec!["本", "本a", "a", "an", "n"]);
    }
}
