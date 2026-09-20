//! The `read_markdown_wiki` table function.
//!
//! # Invocation
//!
//! ```sql
//! CALL read_markdown_wiki('/path/to/wiki');
//! ```
//!
//! The path argument is the wiki root folder, read with [`crate::parse_wiki_dir`].
//!
//! Akar's Cypher grammar has no `YIELD` clause (see `StandaloneCall` in
//! `akar-parser/src/ast.rs`), so the output columns are not named by the caller:
//! they are addressed purely by the field names declared on the produced chunk.
//!
//! # Output contract
//!
//! Exactly two columns, in this order:
//!
//! | column | type | nullable | contents |
//! |--------|------|----------|----------|
//! | `node` | STRING | no | a note id |
//! | `rel`  | STRING | yes | the raw `[[wikilink]]` target on a link row, `NULL` on the note's own row |
//!
//! Rows are emitted note by note, in graph order (ids ascending): first the
//! note's own row `(id, NULL)`, then one `(id, target)` row per link in the
//! note body, in document order. Targets are emitted exactly as written, with no
//! resolution to note ids. Repeated links therefore produce repeated rows; use
//! [`crate::WikiGraph::relations`] for a deduplicated edge list.

use akar_common::types::{PhysicalTypeID, Value};
use akar_common::vector::DataChunk;
use akar_extension::{Extension, ExtensionContext};
use akar_function::registry::TableFunction;
use arrow::array::{ArrayRef, StringArray};
use std::path::Path;
use std::sync::Arc;

/// Registers the `read_markdown_wiki` table function.
///
/// The extension is stateless: it only wires the table function into the
/// function registry, and the function itself re-reads the wiki root on each
/// call.
pub struct MarkdownExtension;

impl Default for MarkdownExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownExtension {
    /// Create the Markdown extension.
    pub fn new() -> Self {
        Self
    }
}

impl Extension for MarkdownExtension {
    fn name(&self) -> &'static str {
        "MARKDOWN"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        context.register_table_function(
            "read_markdown_wiki",
            TableFunction::CustomTable {
                name: "read_markdown_wiki".into(),
                execute: Arc::new(read_markdown_wiki),
            },
        );
        tracing::debug!("MARKDOWN extension registered table function read_markdown_wiki");
        Ok(())
    }
}

/// Fill `chunk` with the note and link rows of the wiki root passed as argument.
///
/// The chunk is filled once: a chunk that already carries rows is left
/// untouched, mirroring the other scan-style table functions.
fn read_markdown_wiki(args: &[Value], chunk: &mut DataChunk) -> Result<(), String> {
    if chunk.size > 0 {
        return Ok(());
    }
    let path = match args.first() {
        Some(Value::String(path)) => path,
        Some(_) => return Err("read_markdown_wiki argument must be a string path".into()),
        None => return Err("read_markdown_wiki requires 1 argument (wiki folder path)".into()),
    };

    let graph = crate::parse_wiki_dir(Path::new(path)).map_err(|error| error.to_string())?;
    tracing::debug!("read_markdown_wiki: parsed {} notes from {}", graph.notes.len(), path);

    let mut nodes: Vec<String> = Vec::new();
    let mut relations: Vec<Option<String>> = Vec::new();
    for note in &graph.notes {
        nodes.push(note.id.clone());
        relations.push(None);
        for link in &note.links {
            nodes.push(note.id.clone());
            relations.push(Some(link.target.clone()));
        }
    }

    let size = nodes.len();
    let node_column: ArrayRef = Arc::new(StringArray::from_iter_values(nodes.iter().map(String::as_str)));
    let rel_column: ArrayRef = Arc::new(StringArray::from_iter(relations.iter().map(|target| target.as_deref())));

    chunk.fields = vec![node_column, rel_column];
    chunk.field_types = vec![PhysicalTypeID::String, PhysicalTypeID::String];
    chunk.field_names = vec!["node".into(), "rel".into()];
    chunk.size = size;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_common::file_system::VirtualFileSystemRegistry;
    use akar_function::registry::FunctionRegistry;
    use arrow::array::Array;
    use std::fs;
    use std::sync::Mutex;

    fn test_context() -> ExtensionContext {
        ExtensionContext::new(
            Arc::new(Mutex::new(FunctionRegistry::new())),
            Arc::new(Mutex::new(akar_catalog::Catalog::new())),
            Arc::new(VirtualFileSystemRegistry::new()),
        )
    }

    fn registered_executor() -> Arc<dyn Fn(&[Value], &mut DataChunk) -> Result<(), String> + Send + Sync> {
        let context = test_context();
        MarkdownExtension::new().load(&context).expect("load extension");
        let registry = context.function_registry().lock().expect("lock registry");
        match registry.get_table("read_markdown_wiki") {
            Some(TableFunction::CustomTable { execute, .. }) => Arc::clone(execute),
            other => panic!("read_markdown_wiki must be a CustomTable, got {other:?}"),
        }
    }

    fn write_note(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create note directory");
        }
        fs::write(&path, contents).expect("write note");
    }

    #[test]
    fn test_extension_name() {
        assert_eq!(MarkdownExtension::new().name(), "MARKDOWN");
        let from_default: MarkdownExtension = Default::default();
        assert_eq!(from_default.name(), "MARKDOWN");
    }

    #[test]
    fn test_load_registers_table_function() {
        let context = test_context();
        MarkdownExtension::new().load(&context).expect("load extension");

        let registry = context.function_registry().lock().expect("lock registry");
        assert!(
            matches!(
                registry.get_table("read_markdown_wiki"),
                Some(TableFunction::CustomTable { .. })
            ),
            "read_markdown_wiki should be registered as a table function"
        );
    }

    #[test]
    fn test_table_function_rows() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "a.md", "[[b]] and [[c|See]]\n");
        write_note(dir.path(), "b.md", "no links here\n");

        let execute = registered_executor();
        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        execute(&[Value::String(dir.path().to_string_lossy().into_owned())], &mut chunk).expect("execute");

        assert_eq!(chunk.size, 4);
        assert_eq!(chunk.field_names, vec!["node".to_string(), "rel".to_string()]);
        assert_eq!(chunk.field_types, vec![PhysicalTypeID::String, PhysicalTypeID::String]);

        let nodes = chunk.fields[0]
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("node column is a StringArray");
        let rels = chunk.fields[1]
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("rel column is a StringArray");

        assert_eq!(nodes.value(0), "a");
        assert!(rels.is_null(0), "the note's own row carries a NULL rel");
        assert_eq!(nodes.value(1), "a");
        assert_eq!(rels.value(1), "b");
        assert_eq!(nodes.value(2), "a");
        assert_eq!(rels.value(2), "c");
        assert_eq!(nodes.value(3), "b");
        assert!(rels.is_null(3), "the note's own row carries a NULL rel");
    }

    #[test]
    fn test_table_function_is_noop_on_non_empty_chunk() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "a.md", "[[b]]\n");

        let execute = registered_executor();
        let args = [Value::String(dir.path().to_string_lossy().into_owned())];
        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        execute(&args, &mut chunk).expect("first call");
        let fields = chunk.fields.len();
        let size = chunk.size;

        execute(&args, &mut chunk).expect("second call");
        assert_eq!(chunk.size, size);
        assert_eq!(chunk.fields.len(), fields);
        assert_eq!(chunk.field_names, vec!["node".to_string(), "rel".to_string()]);
    }

    #[test]
    fn test_table_function_argument_errors() {
        let execute = registered_executor();

        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        let error = execute(&[], &mut chunk).expect_err("missing argument must fail");
        assert!(error.contains("requires 1 argument"), "unexpected error: {error}");

        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        let error = execute(&[Value::Int64(7)], &mut chunk).expect_err("non-string argument must fail");
        assert!(error.contains("must be a string path"), "unexpected error: {error}");
    }

    #[test]
    fn test_table_function_reports_bad_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_note(dir.path(), "note.md", "body\n");

        let execute = registered_executor();
        let mut chunk = DataChunk::new(Vec::new(), Vec::new());
        let file = dir.path().join("note.md");
        let error = execute(&[Value::String(file.to_string_lossy().into_owned())], &mut chunk)
            .expect_err("a file is not a wiki root");
        assert!(error.contains("not a directory"), "unexpected error: {error}");
    }
}
