//! Loading a folder of Markdown notes into a [`WikiGraph`].
//!
//! A *wiki root* is a directory of Markdown files, optionally nested, where each
//! file is a note. A note's **id** is its path relative to the root, without the
//! file extension, using `/` as the separator — `notes/foo.md` becomes
//! `notes/foo`, on every platform.
//!
//! [`parse_wiki_dir`] walks the root recursively, ignores dot-directories and
//! dot-files, accepts the `.md` extension in any letter case, and sorts notes by
//! id so the result is deterministic.

use crate::frontmatter::FrontMatter;
use crate::wikilink::{WikiLink, extract_wikilinks};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A single parsed Markdown note.
#[derive(Debug, Clone)]
pub struct WikiNote {
    /// Note id: the path relative to the wiki root, extension removed and
    /// `/`-separated.
    pub id: String,
    /// Path the note was read from.
    pub path: PathBuf,
    /// Parsed YAML frontmatter; empty when the note has none.
    pub frontmatter: FrontMatter,
    /// Note body: the text after the frontmatter block, with one leading
    /// newline removed.
    pub body: String,
    /// Links found in the body, in document order.
    pub links: Vec<WikiLink>,
}

/// An edge between a note and a link target it writes.
#[derive(Debug, Clone)]
pub struct WikiRelation {
    /// Id of the note that writes the link.
    pub from: String,
    /// The link target exactly as written, *not* resolved to a note id.
    pub to: String,
    /// The link label, when the link carried a non-empty one.
    pub label: Option<String>,
}

/// Every note of a wiki root, in deterministic id order.
#[derive(Debug, Clone, Default)]
pub struct WikiGraph {
    /// Notes ordered by id.
    pub notes: Vec<WikiNote>,
}

impl WikiGraph {
    /// The deduplicated link relations of the graph.
    ///
    /// One relation is emitted per distinct `(from, to)` pair, keeping the
    /// label of its first appearance. Order is deterministic: notes in graph
    /// order, then links in the order they first appear within a note.
    ///
    /// Targets are the raw link text, so relations stay unresolved on purpose —
    /// a caller that wants ids decides how a target maps to a note.
    pub fn relations(&self) -> Vec<WikiRelation> {
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        let mut relations = Vec::new();
        for note in &self.notes {
            for link in &note.links {
                if seen.insert((note.id.as_str(), link.target.as_str())) {
                    relations.push(WikiRelation {
                        from: note.id.clone(),
                        to: link.target.clone(),
                        label: link.label.clone(),
                    });
                }
            }
        }
        relations
    }

    /// Look up a note by its id.
    pub fn get(&self, id: &str) -> Option<&WikiNote> {
        self.notes.iter().find(|note| note.id == id)
    }
}

/// Everything that can go wrong while reading a Markdown wiki.
#[derive(Debug, thiserror::Error)]
pub enum MarkdownError {
    /// A file or directory could not be read.
    #[error("failed to read `{path}`: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The given wiki root exists but is not a directory.
    #[error("wiki root is not a directory: `{0}`")]
    NotADirectory(PathBuf),
    /// A note path is not located below the given wiki root, so it has no id.
    #[error("note `{path}` is not inside wiki root `{root}`")]
    NotUnderRoot {
        /// The note path that was rejected.
        path: PathBuf,
        /// The wiki root it should have been below.
        root: PathBuf,
    },
}

/// Read a single note from `path`, interpreting it relative to `root`.
///
/// The returned [`WikiNote::id`] is `path` relative to `root` without its
/// extension and with `/` separators. Link line numbers refer to the whole file,
/// not to the body.
///
/// # Errors
///
/// Returns [`MarkdownError::Io`] when the file cannot be read and
/// [`MarkdownError::NotUnderRoot`] when `path` is not below `root`.
pub fn parse_note(path: &Path, root: &Path) -> Result<WikiNote, MarkdownError> {
    let source = std::fs::read_to_string(path).map_err(|source| MarkdownError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let relative = path.strip_prefix(root).map_err(|_| MarkdownError::NotUnderRoot {
        path: path.to_path_buf(),
        root: root.to_path_buf(),
    })?;

    let split = crate::frontmatter::split(&source);
    let mut links = extract_wikilinks(split.body);
    for link in &mut links {
        link.line += split.line_offset;
    }

    Ok(WikiNote {
        id: note_id(relative),
        path: path.to_path_buf(),
        frontmatter: split.frontmatter,
        body: split.body.to_string(),
        links,
    })
}

/// Read every Markdown note below `root` into a [`WikiGraph`].
///
/// Dot-directories and dot-files are skipped, as is every file whose extension
/// is not `.md` (in any letter case). Notes are returned sorted by id.
///
/// # Errors
///
/// Returns [`MarkdownError::NotADirectory`] when `root` is not a directory and
/// [`MarkdownError::Io`] when the tree cannot be walked or a note cannot be read.
pub fn parse_wiki_dir(root: &Path) -> Result<WikiGraph, MarkdownError> {
    if !root.is_dir() {
        return Err(MarkdownError::NotADirectory(root.to_path_buf()));
    }
    let mut files = Vec::new();
    collect_markdown_files(root, root, &mut files)?;
    files.sort();

    let mut notes = Vec::with_capacity(files.len());
    for (_, path) in files {
        notes.push(parse_note(&path, root)?);
    }
    Ok(WikiGraph { notes })
}

/// Collect `(id, path)` pairs for every Markdown file below `dir`.
fn collect_markdown_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), MarkdownError> {
    let entries = std::fs::read_dir(dir).map_err(|source| MarkdownError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| MarkdownError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| MarkdownError::Io {
            path: entry.path(),
            source,
        })?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_markdown_files(root, &path, out)?;
        } else if file_type.is_file() && is_markdown(&path) {
            let relative = path.strip_prefix(root).map_err(|_| MarkdownError::NotUnderRoot {
                path: path.clone(),
                root: root.to_path_buf(),
            })?;
            out.push((note_id(relative), path));
        }
    }
    Ok(())
}

/// Whether `path` carries the `.md` extension, ignoring letter case.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// The note id for a path relative to the wiki root.
fn note_id(relative: &Path) -> String {
    relative
        .with_extension("")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_note(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create note directory");
        }
        fs::write(&path, contents).expect("write note");
    }

    #[test]
    fn test_parse_note_reads_frontmatter_body_and_links() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(
            dir.path(),
            "notes/foo.md",
            "---\ntitle: Foo\ntags: [a, b]\n---\n\n# Foo\n\nSee [[bar]] and [[baz#sec|Baz]].\n",
        );

        let note = parse_note(&dir.path().join("notes/foo.md"), dir.path()).expect("parse note");
        assert_eq!(note.id, "notes/foo");
        assert_eq!(note.frontmatter.title(), Some("Foo"));
        assert_eq!(note.frontmatter.tags(), vec!["a", "b"]);
        assert_eq!(note.body, "# Foo\n\nSee [[bar]] and [[baz#sec|Baz]].\n");
        assert_eq!(note.links.len(), 2);
        // Body line 3 is file line 8: 4 frontmatter lines plus the blank line.
        assert_eq!(note.links[0].line, 8);
        assert_eq!(note.links[1].line, 8);
        assert_eq!(note.links[1].anchor.as_deref(), Some("sec"));
    }

    #[test]
    fn test_parse_note_without_frontmatter_keeps_line_numbers() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "plain.md", "line one\n[[a]]\n");

        let note = parse_note(&dir.path().join("plain.md"), dir.path()).expect("parse note");
        assert!(note.frontmatter.is_empty());
        assert_eq!(note.links[0].line, 2);
    }

    #[test]
    fn test_parse_wiki_dir_ids_ordering_and_filtering() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "b.md", "[[c]]\n");
        write_note(dir.path(), "a.md", "# A\n");
        write_note(dir.path(), "case.MD", "# Case\n");
        write_note(dir.path(), "sub/c.md", "# C\n");
        write_note(dir.path(), "sub/nested/d.md", "# D\n");
        write_note(dir.path(), "notes.txt", "ignored\n");
        write_note(dir.path(), ".hidden.md", "ignored\n");
        write_note(dir.path(), ".git/inside.md", "ignored\n");

        let graph = parse_wiki_dir(dir.path()).expect("parse wiki");
        let ids: Vec<&str> = graph.notes.iter().map(|note| note.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "case", "sub/c", "sub/nested/d"]);
    }

    #[test]
    fn test_parse_wiki_dir_relations_are_deduplicated() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "a.md", "[[b|Bee]] then [[c]] then [[b]]\n");
        write_note(dir.path(), "b.md", "[[a]]\n");

        let graph = parse_wiki_dir(dir.path()).expect("parse wiki");
        let relations = graph.relations();
        assert_eq!(relations.len(), 3);
        assert_eq!((relations[0].from.as_str(), relations[0].to.as_str()), ("a", "b"));
        assert_eq!(relations[0].label.as_deref(), Some("Bee"));
        assert_eq!((relations[1].from.as_str(), relations[1].to.as_str()), ("a", "c"));
        assert_eq!(relations[1].label, None);
        assert_eq!((relations[2].from.as_str(), relations[2].to.as_str()), ("b", "a"));
    }

    #[test]
    fn test_parse_wiki_dir_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let graph = parse_wiki_dir(dir.path()).expect("parse wiki");
        assert!(graph.notes.is_empty());
        assert!(graph.relations().is_empty());
        assert!(graph.get("nothing").is_none());
    }

    #[test]
    fn test_graph_get_finds_notes_by_id() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "sub/note.md", "body\n");

        let graph = parse_wiki_dir(dir.path()).expect("parse wiki");
        let note = graph.get("sub/note").expect("note by id");
        assert_eq!(note.path, dir.path().join("sub").join("note.md"));
        assert!(graph.get("sub/note.md").is_none());
    }

    #[test]
    fn test_parse_wiki_dir_rejects_non_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "note.md", "body\n");

        let error = parse_wiki_dir(&dir.path().join("note.md")).expect_err("file is not a directory");
        assert!(
            matches!(error, MarkdownError::NotADirectory(_)),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_parse_note_outside_root_is_rejected() {
        let root = tempfile::tempdir().expect("temp dir");
        let other = tempfile::tempdir().expect("temp dir");
        write_note(other.path(), "loose.md", "body\n");

        let error = parse_note(&other.path().join("loose.md"), root.path()).expect_err("note is outside the root");
        assert!(
            matches!(error, MarkdownError::NotUnderRoot { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_parse_note_reports_missing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = parse_note(&dir.path().join("missing.md"), dir.path()).expect_err("missing file");
        assert!(matches!(error, MarkdownError::Io { .. }), "unexpected error: {error}");
    }
}
