//! Extraction of `[[wikilinks]]` from a note body.
//!
//! The scanner recognizes the four link forms used by Open Knowledge Format
//! wikis: `[[Target]]`, `[[Target|Label]]`, `[[Target#Anchor]]` and
//! `[[Target#Anchor|Label]]`. A link whose opening `[[` is escaped with a
//! backslash is not a link, and links inside fenced code blocks are ignored.
//!
//! Scanning is line oriented: an unclosed `[[` swallows the text up to the
//! first `]]` on the same line, and when a line has no `]]` at all no link is
//! recorded from it.

/// A single `[[...]]` link found in a note body.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiLink {
    /// The link target as written, before any `#` anchor or `|` label.
    pub target: String,
    /// The display label written after `|`, when it is non-empty.
    pub label: Option<String>,
    /// The section anchor written after `#`, when it is non-empty.
    pub anchor: Option<String>,
    /// 1-based line number of the link.
    ///
    /// [`extract_wikilinks`] numbers lines relative to the text it is given;
    /// [`crate::parse_note`] shifts the numbers so they refer to the whole file.
    pub line: usize,
}

/// Extract every `[[wikilink]]` from `body`, in document order.
///
/// Duplicates are reported as separate links; deduplication is the caller's
/// job (see [`crate::WikiGraph::relations`]).
///
/// Line numbers are 1-based and relative to the first line of `body`. Links
/// inside fenced code blocks — opened by three or more backticks or tildes,
/// with an optional info string — are skipped, including everything after an
/// unterminated fence. A closing fence must repeat the opening character at
/// least as many times and must not carry an info string.
pub fn extract_wikilinks(body: &str) -> Vec<WikiLink> {
    let mut links = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (index, (_, line, _)) in crate::line_spans(body).into_iter().enumerate() {
        if let Some((fence_char, fence_len)) = fence {
            if closes_fence(line, fence_char, fence_len) {
                fence = None;
            }
            continue;
        }
        if let Some(opening) = opens_fence(line) {
            fence = Some(opening);
            continue;
        }
        scan_line(line, index + 1, &mut links);
    }
    links
}

/// The fence character and run length when `line` opens a code fence.
fn opens_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let fence_char = trimmed.chars().next()?;
    if fence_char != '`' && fence_char != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|character| *character == fence_char).count();
    if run < 3 { None } else { Some((fence_char, run)) }
}

/// Whether `line` closes a fence opened with `fence_char` repeated
/// `fence_len` times.
fn closes_fence(line: &str, fence_char: char, fence_len: usize) -> bool {
    let trimmed = line.trim();
    let run = trimmed.chars().take_while(|character| *character == fence_char).count();
    // The first `run` characters are ASCII (` or ~), so the byte index is safe.
    run >= fence_len && trimmed[run..].trim().is_empty()
}

/// Collect the links of a single body line.
fn scan_line(line: &str, line_number: usize, links: &mut Vec<WikiLink>) {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] != b'[' || bytes[index + 1] != b'[' || is_escaped(bytes, index) {
            index += 1;
            continue;
        }
        let Some(relative_end) = line[index + 2..].find("]]") else {
            index += 2;
            continue;
        };
        let inner = &line[index + 2..index + 2 + relative_end];
        if let Some(link) = parse_inner(inner, line_number) {
            links.push(link);
        }
        index += 2 + relative_end + 2;
    }
}

/// Whether the `[` at `index` is escaped by an odd number of backslashes.
fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

/// Build a link from the text between `[[` and `]]`, or `None` when the target
/// is empty after trimming.
fn parse_inner(inner: &str, line: usize) -> Option<WikiLink> {
    let (target_part, label_part) = match inner.split_once('|') {
        Some((target, label)) => (target, Some(label)),
        None => (inner, None),
    };
    let (target_raw, anchor_raw) = match target_part.split_once('#') {
        Some((target, anchor)) => (target, Some(anchor)),
        None => (target_part, None),
    };

    let target = target_raw.trim();
    if target.is_empty() {
        return None;
    }

    Some(WikiLink {
        target: target.to_string(),
        label: non_empty(label_part.map(str::trim)),
        anchor: non_empty(anchor_raw.map(str::trim)),
        line,
    })
}

/// The trimmed text, or `None` when it is missing or empty.
fn non_empty(text: Option<&str>) -> Option<String> {
    match text {
        Some(value) if !value.is_empty() => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(links: &[WikiLink]) -> Vec<&str> {
        links.iter().map(|link| link.target.as_str()).collect()
    }

    #[test]
    fn test_link_forms() {
        let links = extract_wikilinks("[[a]] [[a|b]] [[a#s]] [[a#s|b]]");
        assert_eq!(links.len(), 4);
        assert_eq!(
            links[0],
            WikiLink {
                target: "a".into(),
                label: None,
                anchor: None,
                line: 1,
            }
        );
        assert_eq!(links[1].label.as_deref(), Some("b"));
        assert_eq!(links[1].anchor, None);
        assert_eq!(links[2].anchor.as_deref(), Some("s"));
        assert_eq!(links[2].label, None);
        assert_eq!(links[3].anchor.as_deref(), Some("s"));
        assert_eq!(links[3].label.as_deref(), Some("b"));
    }

    #[test]
    fn test_whitespace_trimming() {
        let links = extract_wikilinks("[[  target  #  anchor  |  label  ]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "target");
        assert_eq!(links[0].anchor.as_deref(), Some("anchor"));
        assert_eq!(links[0].label.as_deref(), Some("label"));
    }

    #[test]
    fn test_empty_target_skipped() {
        let links = extract_wikilinks("[[   ]] [[|label]] [[#anchor]] [[ok]]");
        assert_eq!(targets(&links), vec!["ok"]);
    }

    #[test]
    fn test_empty_label_and_anchor_dropped() {
        let links = extract_wikilinks("[[a|]] [[a#]]");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].label, None);
        assert_eq!(links[1].anchor, None);
    }

    #[test]
    fn test_duplicates_kept_with_line_numbers() {
        let links = extract_wikilinks("[[a]] and text\n[[a]] again");
        assert_eq!(targets(&links), vec!["a", "a"]);
        assert_eq!(links[0].line, 1);
        assert_eq!(links[1].line, 2);
    }

    #[test]
    fn test_escaped_link_ignored() {
        let links = extract_wikilinks(r"\[[a]] then [[b]]");
        assert_eq!(targets(&links), vec!["b"]);
    }

    #[test]
    fn test_escaped_backslash_still_yields_link() {
        let links = extract_wikilinks(r"\\[[a]]");
        assert_eq!(targets(&links), vec!["a"]);
    }

    #[test]
    fn test_fenced_code_block_skipped() {
        let body = "before [[a]]\n```rust\nlet x = [[b]];\n```\n~~~\n[[c]]\n~~~\nafter [[d]]\n";
        let links = extract_wikilinks(body);
        assert_eq!(targets(&links), vec!["a", "d"]);
        assert_eq!(links[0].line, 1);
        assert_eq!(links[1].line, 8);
    }

    #[test]
    fn test_unterminated_fence_skips_rest() {
        let links = extract_wikilinks("[[a]]\n```\n[[b]]\n");
        assert_eq!(targets(&links), vec!["a"]);
    }

    #[test]
    fn test_closing_fence_rejects_info_string() {
        let body = "```\n[[a]]\n```rust\n[[b]]\n```\n[[c]]\n";
        let links = extract_wikilinks(body);
        assert_eq!(targets(&links), vec!["c"]);
        assert_eq!(links[0].line, 6);
    }

    #[test]
    fn test_crlf_line_endings() {
        let links = extract_wikilinks("first\r\n[[a]]\r\n");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "a");
        assert_eq!(links[0].line, 2);
    }

    #[test]
    fn test_unclosed_link_on_line_records_nothing() {
        assert!(extract_wikilinks("[[a\n[[b\n").is_empty());
    }
}
