//! Markdown wiki / Open Knowledge Format reader for Akar.
//!
//! This crate reads a folder of Markdown notes — the layout used by
//! "open knowledge format" wikis such as Obsidian vaults — where each note may
//! carry YAML frontmatter and links written as `[[wikilinks]]`, and exposes the
//! result as Rust types plus a Cypher table function.
//!
//! # Limits
//!
//! The crate has no third-party parser dependencies: the frontmatter reader
//! implements a small, documented YAML subset (see [`frontmatter`] and the
//! crate README) and the link scanner is line oriented. Unsupported constructs
//! are skipped rather than rejected, so a slightly malformed note never fails a
//! whole wiki.
//!
//! # Example
//!
//! ```no_run
//! let graph = akar_markdown::parse_wiki_dir(std::path::Path::new("/path/to/vault"))?;
//! for note in &graph.notes {
//!     println!("{}: {} link(s)", note.id, note.links.len());
//! }
//! for relation in graph.relations() {
//!     println!("{} -> {}", relation.from, relation.to);
//! }
//! # Ok::<(), akar_markdown::MarkdownError>(())
//! ```
//!
//! From Cypher, the same data is available through the `read_markdown_wiki`
//! table function registered by [`MarkdownExtension`]:
//!
//! ```sql
//! CALL read_markdown_wiki('/path/to/vault');
//! ```

mod extension;
mod frontmatter;
mod wiki;
mod wikilink;

pub use extension::MarkdownExtension;
pub use frontmatter::{FrontMatter, FrontMatterValue};
pub use wiki::{MarkdownError, WikiGraph, WikiNote, WikiRelation, parse_note, parse_wiki_dir};
pub use wikilink::{WikiLink, extract_wikilinks};

/// Split `text` into lines as `(start, content, next)`.
///
/// `content` excludes the line terminator, and a trailing `\r` is removed so
/// `\r\n` files behave like `\n` files. `start` is the byte offset of the line
/// in `text` and `next` is the byte offset of the following line, which is
/// `text.len()` for the last line.
///
/// Both the frontmatter reader and the wikilink scanner use this single
/// primitive, so they tolerate line endings identically.
fn line_spans(text: &str) -> Vec<(usize, &str, usize)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        match text[offset..].find('\n') {
            Some(relative) => {
                let next = offset + relative + 1;
                let line = &text[offset..offset + relative];
                spans.push((offset, line.strip_suffix('\r').unwrap_or(line), next));
                offset = next;
            }
            None => {
                let line = &text[offset..];
                spans.push((offset, line.strip_suffix('\r').unwrap_or(line), text.len()));
                break;
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::line_spans;
    use akar_extension::Extension;

    #[test]
    fn test_public_api_is_reachable() {
        let extract: fn(&str) -> Vec<crate::WikiLink> = crate::extract_wikilinks;
        assert_eq!(extract("[[a]]").len(), 1);

        let graph = crate::WikiGraph::default();
        assert!(graph.notes.is_empty());
        assert!(graph.relations().is_empty());
        assert!(graph.get("missing").is_none());

        let frontmatter = crate::FrontMatter::default();
        assert!(frontmatter.is_empty());
        assert_eq!(frontmatter.len(), 0);
        assert_eq!(frontmatter.get("absent"), None);
        assert_eq!(frontmatter.title(), None);
        assert!(frontmatter.tags().is_empty());
        assert!(frontmatter.aliases().is_empty());
        assert_eq!(frontmatter.iter().count(), 0);

        assert_eq!(crate::MarkdownExtension::new().name(), "MARKDOWN");
    }

    #[test]
    fn test_line_spans_handles_crlf_and_last_line() {
        let spans = line_spans("a\r\nb\n\nc");
        let lines: Vec<&str> = spans.iter().map(|(_, line, _)| *line).collect();
        assert_eq!(lines, vec!["a", "b", "", "c"]);
        assert_eq!(spans[0].0, 0);
        assert_eq!(spans[0].2, 3);
        assert_eq!(spans[3].2, 7);
    }

    #[test]
    fn test_line_spans_empty_input() {
        assert!(line_spans("").is_empty());
    }
}
