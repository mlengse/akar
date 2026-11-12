//! Parsing of the YAML frontmatter block at the top of a Markdown note.
//!
//! # Supported subset
//!
//! This module deliberately implements a **small, dependency-free subset of
//! YAML** instead of depending on a YAML crate (`serde_yaml` is deprecated and
//! unmaintained, and this crate is meant to stay lightweight). A frontmatter
//! block is recognized when a note starts with a `---` line and a later line is
//! exactly `---`; a leading UTF-8 BOM and `\r\n` line endings are tolerated.
//!
//! Inside the block these forms are understood:
//!
//! - `key: value` — a scalar. The value may be plain, `'single quoted'`, or
//!   `"double quoted"`. Inside double quotes `\"` and `\\` are unescaped; every
//!   other backslash sequence (for example `\n`) is preserved verbatim rather
//!   than interpreted.
//! - `key:` followed by `- item` lines — a list.
//! - `key: [a, b, "c"]` — an inline (flow) list. Quoted elements may contain
//!   commas, and elements are trimmed.
//! - `#` starts a comment both on a whole line and on a plain (unquoted) value.
//!
//! Anything else is **skipped without error**: nested maps (both indented
//! `a: 1` blocks and inline `{a: 1}` flow maps), multi-line block scalars
//! (`|`, `>`), anchors and aliases (`&anchor`, `*alias`), and lines that carry
//! no `key:` separator. A key whose value is empty and is not followed by list
//! items is recorded as [`FrontMatterValue::Scalar`] with an empty string.
//!
//! # Failure modes
//!
//! Malformed frontmatter never fails a note:
//!
//! - An opening `---` with no closing `---` makes the whole file the body, with
//!   empty frontmatter.
//! - A value that starts with a quote but has no closing quote is kept as a
//!   plain scalar (with trailing comments stripped).
//! - A value that starts with `[` but has no closing `]` is kept as a plain
//!   scalar as well.
//!
//! Entries live in a [`BTreeMap`], so [`FrontMatter::iter`] order is
//! deterministic (sorted by key) and independent of the order the keys appear
//! in the file. A key repeated in the block keeps its last value.

use std::collections::BTreeMap;

/// UTF-8 byte order mark, which may precede the opening `---` line.
const BOM: char = '\u{feff}';

/// A value stored under a frontmatter key, supporting full OKF v0.2 nested structures.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontMatterValue {
    /// A single scalar string value, with surrounding quotes removed.
    Scalar(String),
    /// A list of frontmatter values, in document order.
    List(Vec<FrontMatterValue>),
    /// A nested mapping of key-value frontmatter entries.
    Map(BTreeMap<String, FrontMatterValue>),
    /// A boolean value (`true` or `false`).
    Bool(bool),
    /// A numeric value.
    Number(f64),
    /// Null / empty value.
    Null,
}

impl FrontMatterValue {
    /// Returns the string slice if this is a Scalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FrontMatterValue::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the list slice if this is a List.
    pub fn as_list(&self) -> Option<&[FrontMatterValue]> {
        match self {
            FrontMatterValue::List(l) => Some(l.as_slice()),
            _ => None,
        }
    }

    /// Returns the map reference if this is a Map.
    pub fn as_map(&self) -> Option<&BTreeMap<String, FrontMatterValue>> {
        match self {
            FrontMatterValue::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Convert to serde_json::Value.
    pub fn to_json(&self) -> serde_json::Value {
        self.clone().into()
    }

    /// Construct from serde_json::Value.
    pub fn from_json(val: serde_json::Value) -> Self {
        val.into()
    }

    /// Convert to Akar's native query engine Value.
    pub fn to_akar_value(&self) -> akar_common::types::Value {
        match self {
            FrontMatterValue::Null => akar_common::types::Value::Null,
            FrontMatterValue::Bool(b) => akar_common::types::Value::Bool(*b),
            FrontMatterValue::Number(n) => akar_common::types::Value::Double(*n),
            FrontMatterValue::Scalar(s) => akar_common::types::Value::String(s.clone()),
            FrontMatterValue::List(items) => {
                akar_common::types::Value::List(items.iter().map(|it| it.to_akar_value()).collect())
            }
            FrontMatterValue::Map(_) => akar_common::types::Value::Json(self.clone().into()),
        }
    }
}

impl From<serde_json::Value> for FrontMatterValue {
    fn from(json: serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => FrontMatterValue::Null,
            serde_json::Value::Bool(b) => FrontMatterValue::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    FrontMatterValue::Number(f)
                } else {
                    FrontMatterValue::Scalar(n.to_string())
                }
            }
            serde_json::Value::String(s) => FrontMatterValue::Scalar(s),
            serde_json::Value::Array(arr) => {
                FrontMatterValue::List(arr.into_iter().map(FrontMatterValue::from).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k, FrontMatterValue::from(v));
                }
                FrontMatterValue::Map(map)
            }
        }
    }
}

impl From<FrontMatterValue> for serde_json::Value {
    fn from(fm: FrontMatterValue) -> Self {
        match fm {
            FrontMatterValue::Null => serde_json::Value::Null,
            FrontMatterValue::Bool(b) => serde_json::Value::Bool(b),
            FrontMatterValue::Number(n) => serde_json::Number::from_f64(n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            FrontMatterValue::Scalar(s) => serde_json::Value::String(s),
            FrontMatterValue::List(list) => {
                serde_json::Value::Array(list.into_iter().map(serde_json::Value::from).collect())
            }
            FrontMatterValue::Map(map) => {
                let mut obj = serde_json::Map::new();
                for (k, v) in map {
                    obj.insert(k, serde_json::Value::from(v));
                }
                serde_json::Value::Object(obj)
            }
        }
    }
}

/// The parsed YAML frontmatter of a single note.
///
/// Entries are keyed by name in a [`BTreeMap`], so iteration is ordered by key
/// and therefore deterministic across runs and platforms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrontMatter {
    entries: BTreeMap<String, FrontMatterValue>,
}

impl FrontMatter {
    /// Parse YAML frontmatter from document text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let s = split(text);
        Ok(s.frontmatter)
    }

    /// Look up an entry by its exact key.
    pub fn get(&self, key: &str) -> Option<&FrontMatterValue> {
        self.entries.get(key)
    }

    /// Look up a nested entry following path segments, e.g. `["meta", "author"]`.
    pub fn get_nested(&self, path: &[&str]) -> Option<&FrontMatterValue> {
        if path.is_empty() {
            return None;
        }
        let mut current = self.entries.get(path[0])?;
        for &segment in &path[1..] {
            match current {
                FrontMatterValue::Map(map) => {
                    current = map.get(segment)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }

    /// The `title` entry when it is a non-empty scalar.
    pub fn title(&self) -> Option<&str> {
        match self.entries.get("title") {
            Some(FrontMatterValue::Scalar(value)) if !value.is_empty() => Some(value.as_str()),
            _ => None,
        }
    }

    /// The `tags` entry as a list, whether it was written as a scalar or a list.
    ///
    /// An empty or whitespace-only scalar yields an empty list.
    pub fn tags(&self) -> Vec<String> {
        self.string_list("tags")
    }

    /// The `aliases` entry as a list, falling back to the singular `alias` key.
    pub fn aliases(&self) -> Vec<String> {
        let aliases = self.string_list("aliases");
        if aliases.is_empty() {
            self.string_list("alias")
        } else {
            aliases
        }
    }

    /// Whether the note carries no frontmatter entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of frontmatter entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterate over every entry in deterministic (key-sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &FrontMatterValue)> {
        self.entries.iter().map(|(key, value)| (key.as_str(), value))
    }

    /// Convert entire frontmatter to JSON.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in &self.entries {
            map.insert(k.clone(), serde_json::Value::from(v.clone()));
        }
        serde_json::Value::Object(map)
    }

    /// Convert frontmatter to Akar Value::Json.
    pub fn to_akar_value(&self) -> akar_common::types::Value {
        akar_common::types::Value::Json(self.to_json())
    }

    /// Read `key` as a list of strings, accepting the scalar form too.
    fn string_list(&self, key: &str) -> Vec<String> {
        match self.entries.get(key) {
            Some(FrontMatterValue::Scalar(value)) if !value.trim().is_empty() => vec![value.clone()],
            Some(FrontMatterValue::List(items)) => items
                .iter()
                .filter_map(|item| match item {
                    FrontMatterValue::Scalar(s) => Some(s.clone()),
                    FrontMatterValue::Bool(b) => Some(b.to_string()),
                    FrontMatterValue::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// A note source split into its frontmatter block and its body.
pub(crate) struct Split<'a> {
    /// The parsed frontmatter; empty when the note has no frontmatter block.
    pub(crate) frontmatter: FrontMatter,
    /// The body: everything after the frontmatter block, with one leading
    /// newline removed.
    pub(crate) body: &'a str,
    /// Number of file lines that precede the body. A body-relative 1-based line
    /// number `n` is therefore file line `n + line_offset`.
    pub(crate) line_offset: usize,
}

/// Split `text` into its (possibly empty) frontmatter block and its body.
pub(crate) fn split(text: &str) -> Split<'_> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let spans = crate::line_spans(text);
    let Some((_, first_line, _)) = spans.first() else {
        return Split {
            frontmatter: FrontMatter::default(),
            body: text,
            line_offset: 0,
        };
    };
    if first_line.trim() != "---" {
        return Split {
            frontmatter: FrontMatter::default(),
            body: text,
            line_offset: 0,
        };
    }

    // The block ends at the next `---` line; without one there is no
    // frontmatter and the whole file is the body.
    let Some(closing_index) = spans
        .iter()
        .skip(1)
        .position(|(_, line, _)| line.trim() == "---")
        .map(|i| i + 1)
    else {
        return Split {
            frontmatter: FrontMatter::default(),
            body: text,
            line_offset: 0,
        };
    };

    let block: Vec<&str> = spans[1..closing_index].iter().map(|(_, line, _)| *line).collect();
    let mut body = &text[spans[closing_index].2..];
    let mut line_offset = closing_index + 1;
    if let Some(rest) = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')) {
        body = rest;
        line_offset += 1;
    }

    Split {
        frontmatter: parse_block(&block),
        body,
        line_offset,
    }
}

/// Parse the lines between the opening and closing `---` markers.
fn parse_block(block: &[&str]) -> FrontMatter {
    let joined = block.join("\n");
    if let Ok(serde_json::Value::Object(map)) = crate::yaml::parse(&joined) {
        let mut frontmatter = FrontMatter::default();
        for (k, v) in map {
            let val = match v {
                serde_json::Value::Null => FrontMatterValue::Scalar(String::new()),
                other => FrontMatterValue::from(other),
            };
            frontmatter.entries.insert(k, val);
        }
        return frontmatter;
    }

    let mut frontmatter = FrontMatter::default();
    let mut index = 0;
    while index < block.len() {
        let line = block[index].trim();
        index += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = key_colon(line) else {
            continue;
        };
        let key = line[..colon].trim();
        if key.is_empty() {
            continue;
        }

        let value = strip_trailing_comment(line[colon + 1..].trim());
        if value.is_empty() {
            // An empty value is either a block list or an empty scalar.
            let mut items = Vec::new();
            let mut scan = index;
            loop {
                // Blank lines between list items are allowed.
                while scan < block.len() && block[scan].trim().is_empty() {
                    scan += 1;
                }
                let Some(item) = block.get(scan).and_then(|raw| list_item(raw)) else {
                    break;
                };
                if !item.is_empty() {
                    items.push(FrontMatterValue::Scalar(parse_scalar(item)));
                }
                scan += 1;
            }
            if !items.is_empty() {
                frontmatter
                    .entries
                    .insert(key.to_string(), FrontMatterValue::List(items));
                index = scan;
                continue;
            }
            // No list items: skip an indented nested block, or record an empty
            // scalar when the key really has no value.
            if let Some(next) = skip_indented_block(block, index) {
                index = next;
                continue;
            }
            frontmatter
                .entries
                .insert(key.to_string(), FrontMatterValue::Scalar(String::new()));
            continue;
        }

        match parse_value(value) {
            Some(parsed) => {
                frontmatter.entries.insert(key.to_string(), parsed);
            }
            // Unsupported constructs (flow maps, block scalars, anchors) are
            // skipped together with any indented continuation lines.
            None => {
                if let Some(next) = skip_indented_block(block, index) {
                    index = next;
                }
            }
        }
    }
    frontmatter
}

/// Index of the `:` that separates a key from its value, or `None` when the
/// line is not a `key: value` pair.
///
/// As in YAML, a `:` only separates when it is followed by whitespace or by the
/// end of the line, so plain values such as URLs (`url: https://example.com`)
/// keep their colons.
fn key_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b':' {
            continue;
        }
        match bytes.get(index + 1) {
            None => return Some(index),
            Some(next) if next.is_ascii_whitespace() => return Some(index),
            Some(_) => continue,
        }
    }
    None
}

/// The value of a `- item` line, or the empty string for a bare `-` line.
///
/// Returns `None` when the line is not a list item, which also rejects the
/// closing `---` marker and separators such as `---`.
fn list_item(raw: &str) -> Option<&str> {
    let rest = raw.trim().strip_prefix('-')?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

/// Skip a blank-lines-plus-indented block, returning the index after it.
///
/// Returns `None` when no indented line follows, in which case the caller must
/// keep parsing at the current index.
fn skip_indented_block(block: &[&str], from: usize) -> Option<usize> {
    let mut index = from;
    let mut saw_indented = false;
    loop {
        while index < block.len() && block[index].trim().is_empty() {
            index += 1;
        }
        match block.get(index) {
            Some(raw) if raw.starts_with(' ') || raw.starts_with('\t') => {
                saw_indented = true;
                index += 1;
            }
            _ => break,
        }
    }
    if saw_indented { Some(index) } else { None }
}

/// Parse a value that is not empty, returning `None` for the unsupported
/// constructs listed in the module documentation.
fn parse_value(value: &str) -> Option<FrontMatterValue> {
    if let Some(inner) = delimited_by(value, '[', ']') {
        return Some(FrontMatterValue::List(
            parse_inline_list(inner)
                .into_iter()
                .map(FrontMatterValue::Scalar)
                .collect(),
        ));
    }
    if delimited_by(value, '{', '}').is_some() {
        return None;
    }
    if value.starts_with('|') || value.starts_with('>') || value.starts_with('&') || value.starts_with('*') {
        return None;
    }
    Some(FrontMatterValue::Scalar(parse_scalar(value)))
}

/// The inner text of `value` when it starts with `open` and ends with `close`.
fn delimited_by(value: &str, open: char, close: char) -> Option<&str> {
    if value.len() < 2 || !value.starts_with(open) || !value.ends_with(close) {
        return None;
    }
    Some(&value[open.len_utf8()..value.len() - close.len_utf8()])
}

/// Split the body of an inline list on top-level commas and parse each element.
fn parse_inline_list(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in inner.chars() {
        match quote {
            Some(active) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' && active == '"' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
                current.push(character);
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                    current.push(character);
                } else if character == ',' {
                    items.push(std::mem::take(&mut current));
                } else {
                    current.push(character);
                }
            }
        }
    }
    items.push(current);

    items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(parse_scalar)
        .collect()
}

/// Parse a single scalar: unquote it, or strip a trailing comment when plain.
fn parse_scalar(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = unquote(raw, '"') {
        return unescape_double_quoted(inner);
    }
    if let Some(inner) = unquote(raw, '\'') {
        return inner.replace("''", "'");
    }
    strip_trailing_comment(raw).to_string()
}

/// The text between the outer quotes, or `None` when `raw` is not a quoted
/// value closed by a matching quote.
fn unquote(raw: &str, quote: char) -> Option<&str> {
    let mut characters = raw.char_indices();
    if characters.next().map(|(_, c)| c) != Some(quote) {
        return None;
    }
    let mut escaped = false;
    for (index, character) in characters {
        if escaped {
            escaped = false;
        } else if character == '\\' && quote == '"' {
            escaped = true;
        } else if character == quote {
            return Some(&raw[quote.len_utf8()..index]);
        }
    }
    None
}

/// Resolve `\"` and `\\` inside a double-quoted scalar, keeping every other
/// backslash sequence as written.
fn unescape_double_quoted(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Drop a trailing `#` comment from a plain (unquoted) value.
///
/// Inside quotes a `#` is literal, and it only starts a comment when it is the
/// first character of the value or follows whitespace.
fn strip_trailing_comment(value: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut after_whitespace = true;
    for (index, character) in value.char_indices() {
        match quote {
            Some(active) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' && active == '"' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character == '#' && after_whitespace {
                    return value[..index].trim_end();
                }
            }
        }
        after_whitespace = quote.is_none() && character.is_whitespace();
    }
    value.trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontmatter_of(text: &str) -> FrontMatter {
        split(text).frontmatter
    }

    #[test]
    fn test_plain_and_quoted_scalars() {
        let frontmatter =
            frontmatter_of("---\ntitle: Hello World\nauthor: 'Ada'\nnote: \"a \\\"quoted\\\" word\"\n---\n\nbody\n");
        assert_eq!(
            frontmatter.get("title"),
            Some(&FrontMatterValue::Scalar("Hello World".into()))
        );
        assert_eq!(frontmatter.get("author"), Some(&FrontMatterValue::Scalar("Ada".into())));
        assert_eq!(
            frontmatter.get("note"),
            Some(&FrontMatterValue::Scalar("a \"quoted\" word".into()))
        );
        assert_eq!(frontmatter.title(), Some("Hello World"));
        assert_eq!(frontmatter.len(), 3);
    }

    #[test]
    fn test_double_quote_escapes() {
        let frontmatter = frontmatter_of("---\npath: \"a\\\\b\"\nliteral: \"a\\nb\"\n---\n");
        // Standard YAML double-quote unescaping: `\\` -> `\`, `\n` -> newline.
        assert_eq!(frontmatter.get("path"), Some(&FrontMatterValue::Scalar("a\\b".into())));
        assert_eq!(
            frontmatter.get("literal"),
            Some(&FrontMatterValue::Scalar("a\nb".into()))
        );
    }

    #[test]
    fn test_block_list() {
        let frontmatter = frontmatter_of("---\ntags:\n  - rust\n  - graph\nother: x\n---\n");
        assert_eq!(
            frontmatter.get("tags"),
            Some(&FrontMatterValue::List(vec![
                FrontMatterValue::Scalar("rust".into()),
                FrontMatterValue::Scalar("graph".into())
            ]))
        );
        assert_eq!(frontmatter.get("other"), Some(&FrontMatterValue::Scalar("x".into())));
        assert_eq!(frontmatter.tags(), vec!["rust", "graph"]);
    }

    #[test]
    fn test_inline_list() {
        let frontmatter = frontmatter_of("---\ntags: [a, \"b, c\", 'd']\naliases: []\n---\n");
        assert_eq!(frontmatter.tags(), vec!["a", "b, c", "d"]);
        assert!(frontmatter.aliases().is_empty());
    }

    #[test]
    fn test_scalar_and_alias_forms() {
        let frontmatter = frontmatter_of("---\ntags: solo\nalias: alt\n---\n");
        assert_eq!(frontmatter.tags(), vec!["solo"]);
        assert_eq!(frontmatter.aliases(), vec!["alt"]);
    }

    #[test]
    fn test_comments() {
        let frontmatter =
            frontmatter_of("---\n# a whole-line comment\ntitle: Note # trailing\nquoted: \"keep # this\"\n---\n");
        assert_eq!(frontmatter.title(), Some("Note"));
        assert_eq!(
            frontmatter.get("quoted"),
            Some(&FrontMatterValue::Scalar("keep # this".into()))
        );
        assert_eq!(frontmatter.len(), 2);
    }

    #[test]
    fn test_missing_frontmatter() {
        let split = split("# Just a heading\n[[link]]\n");
        assert!(split.frontmatter.is_empty());
        assert_eq!(split.body, "# Just a heading\n[[link]]\n");
        assert_eq!(split.line_offset, 0);
    }

    #[test]
    fn test_unterminated_frontmatter() {
        let split = split("---\ntitle: x\nnot closed\n");
        assert!(split.frontmatter.is_empty());
        assert_eq!(split.body, "---\ntitle: x\nnot closed\n");
        assert_eq!(split.line_offset, 0);
    }

    #[test]
    fn test_empty_frontmatter_block() {
        let split = split("---\n---\nbody\n");
        assert!(split.frontmatter.is_empty());
        assert_eq!(split.body, "body\n");
        assert_eq!(split.line_offset, 2);
    }

    #[test]
    fn test_crlf_line_endings() {
        let split = split("---\r\ntitle: Win\r\n---\r\n\r\nbody line\r\n");
        assert_eq!(split.frontmatter.title(), Some("Win"));
        assert_eq!(split.body, "body line\r\n");
        assert_eq!(split.line_offset, 4);
    }

    #[test]
    fn test_bom_tolerated() {
        let split = split("\u{feff}---\ntitle: BOM\n---\n\nbody\n");
        assert_eq!(split.frontmatter.title(), Some("BOM"));
        assert_eq!(split.body, "body\n");
        assert_eq!(split.line_offset, 4);
    }

    #[test]
    fn test_unsupported_constructs_skipped() {
        let frontmatter = frontmatter_of(
            "---\nanchored: &anchor value\nempty:\n---\n",
        );
        assert!(frontmatter.get("anchored").is_none());
        assert_eq!(frontmatter.get("empty"), Some(&FrontMatterValue::Scalar(String::new())));
        assert_eq!(frontmatter.len(), 1);
    }

    #[test]
    fn test_nested_maps_and_okf_frontmatter() {
        let frontmatter = frontmatter_of(
            "---\nnested:\n  a: 1\n  b: 2\nflow: [1, 2]\n---\n",
        );
        assert!(frontmatter.get("nested").is_some());
        assert_eq!(
            frontmatter.get_nested(&["nested", "a"]),
            Some(&FrontMatterValue::Number(1.0))
        );
        assert_eq!(
            frontmatter.get_nested(&["nested", "b"]),
            Some(&FrontMatterValue::Number(2.0))
        );
    }

    #[test]
    fn test_iter_is_key_sorted() {
        let frontmatter = frontmatter_of("---\nzebra: 1\nalpha: 2\nmiddle: 3\n---\n");
        let keys: Vec<&str> = frontmatter.iter().map(|(key, _)| key).collect();
        assert_eq!(keys, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn test_key_with_url_value() {
        let frontmatter = frontmatter_of("---\nsource: https://example.com/a:b\n---\n");
        assert_eq!(
            frontmatter.get("source"),
            Some(&FrontMatterValue::Scalar("https://example.com/a:b".into()))
        );
    }
}
