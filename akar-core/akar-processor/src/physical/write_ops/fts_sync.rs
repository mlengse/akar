//! Commit-time FTS propagation (P107.1).
//!
//! [`sync_indexes_on_commit`] is invoked by the connection layer's commit
//! pipe-line (Step 4.5 of `Connection::commit_write_txn`) with the txn's
//! written `(table_id, row_id)` pairs. For every registered FTS index over a
//! touched table it snapshots the current text values of those rows and applies
//! them to the on-disk Tantivy index via [`akar_fts::build::apply_doc_writes`]:
//! an update replaces the row's document, a delete (NULL / missing column)
//! removes it.
//!
//! This is the *single* incremental writer: the `PhysicalFtsScan` read path
//! only opens + reloads + searches. Failures here are non-fatal by contract —
//! they are surfaced as warnings by the caller so a stale index can never
//! roll back an already-durable commit.

use std::sync::Arc;

use akar_common::types::Value;
use akar_storage::table::TableCatalog;

/// Apply the written rows of a committed transaction to every FTS index over
/// a touched table.
///
/// `fts_indexes` is the `(name, table_name, column_name)` snapshot from the
/// catalog; `written_rows` is the txn's deduplicated `(table_id, row_id)`
/// write set. Returns the number of `(row, text)` writes applied, or an error
/// message. Indexes with no on-disk Tantivy data yet (index never built /
/// legacy schema) are skipped with a warning.
pub fn sync_indexes_on_commit(
    table_catalog: &Arc<TableCatalog>,
    fts_indexes: &[(String, String, String)],
    written_rows: &[(u64, u64)],
) -> Result<usize, String> {
    if fts_indexes.is_empty() || written_rows.is_empty() {
        return Ok(0);
    }

    // On-disk indexes only: an in-memory database has no `<db_path>/fts/`
    // directory, so there is nothing to propagate to.
    let Some(base) = table_catalog.db_path() else {
        return Ok(0);
    };
    if base.to_string_lossy() == ":memory:" {
        return Ok(0);
    }

    let written_tables: std::collections::HashSet<u64> = written_rows.iter().map(|(t, _)| *t).collect();

    let mut synced = 0usize;
    for (name, table_name, column_name) in fts_indexes {
        // Only indexes whose source table actually wrote rows need syncing.
        let Some(source_table) = table_catalog.get_node_table_by_name(table_name) else {
            continue;
        };
        let table_id = source_table.table_id;
        if !written_tables.contains(&table_id) {
            continue;
        }
        let Some(col_idx) = source_table.columns.iter().position(|c| c.name == *column_name) else {
            continue;
        };

        let index_dir = base.join("fts").join(name);
        if !index_dir.join("meta.json").exists() {
            tracing::warn!(
                "FTS: index '{name}' has no on-disk Tantivy data; skipping commit-time sync \
                 (rebuild with DROP + CREATE FTS INDEX if the schema changed)"
            );
            continue;
        }

        // Snapshot the current value of every written row. A single row may
        // have several undo records within one txn; `get_value` returns the
        // last committed value. The DashMap `Ref` is scoped out before the
        // Tantivy writer is opened (DashMap is not re-entrant — the FTS test
        // flake, P53.x).
        let writes: Vec<(i64, Option<String>)> = {
            let mut seen = std::collections::HashSet::with_capacity(written_rows.len());
            let mut out = Vec::with_capacity(written_rows.len());
            for (tid, row_id) in written_rows {
                if *tid != table_id || !seen.insert(*row_id) {
                    continue;
                }
                let Ok(row) = usize::try_from(*row_id) else {
                    continue;
                };
                let text = match source_table.get_value(row, col_idx) {
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => None, // NULL (soft-deleted / never set) → delete-term
                };
                out.push((*row_id as i64, text));
            }
            out
        };

        if writes.is_empty() {
            continue;
        }

        let index = akar_fts::index::TantivyIndex::open_on_disk(&index_dir)
            .map_err(|e| format!("FTS: open index '{name}': {e}"))?;
        akar_fts::build::apply_doc_writes(&index, column_name, &writes)
            .map_err(|e| format!("FTS: sync index '{name}': {e}"))?;
        synced += writes.len();
    }

    Ok(synced)
}
