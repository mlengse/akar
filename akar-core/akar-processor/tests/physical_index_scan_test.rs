//! Physical index scan liveness, re-verification, and build-fallback tests
//! (T029, US4 — FR-004, FR-005, FR-006, FR-007).
//!
//! `PhysicalIndexScan` must never surface a row the table no longer owns:
//! a soft-deleted slot (NULL primary key), a row whose indexed column changed
//! without the index being updated, or an index that is still `Building`.

use akar_catalog::IndexType;
use akar_common::enums::CompressionType;
use akar_common::memory::MemoryManager;
use akar_common::types::{LogicalTypeID, Value};
use akar_processor::physical::index_scan::PhysicalIndexScan;
use akar_processor::physical::types::PhysicalOperatorExec;
use akar_storage::StorageManager;
use akar_storage::table::ColumnDefinition;
use std::sync::Arc;
use tempfile::TempDir;

const TABLE: &str = "Memory";
const INDEX: &str = "si_status";

fn columns() -> Vec<ColumnDefinition> {
    vec![
        ColumnDefinition {
            name: "id".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: true,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: "status".into(),
            logical_type: LogicalTypeID::String,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: "payload".into(),
            logical_type: LogicalTypeID::String,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
    ]
}

/// Statuses by row index: 0 active, 1 archived, 2 active, 3 cold, 4 active,
/// 5 archived, 6 pinned, 7 active, 8 cold, 9 archived.
const STATUSES: [&str; 10] = [
    "active", "archived", "active", "cold", "active", "archived", "pinned", "active", "cold", "archived",
];

fn setup() -> (Arc<StorageManager>, TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let sm = Arc::new(StorageManager::new(
        dir.path().to_path_buf(),
        Arc::new(MemoryManager::new(64 * 1024 * 1024)),
    ));
    sm.create_node_table(TABLE.into(), columns());
    for (id, status) in STATUSES.iter().enumerate() {
        sm.insert_row_with_dedup(
            TABLE,
            vec![
                Value::Int64(id as i64),
                Value::String((*status).to_string()),
                Value::String(format!("payload-{id}")),
            ],
            None,
        )
        .expect("insert row");
    }
    sm.create_secondary_index(INDEX.into(), TABLE.into(), "status".into(), IndexType::Hash, false)
        .expect("create secondary index");
    (sm, dir)
}

/// Execute an index scan and return the sorted primary-key values it produced.
fn scan(sm: &Arc<StorageManager>, index_name: &str, values: Vec<Value>) -> Vec<i64> {
    let operator = PhysicalIndexScan {
        table_name: TABLE.into(),
        table_id: 0,
        alias: Some("m".into()),
        column_name: "status".into(),
        index_name: index_name.into(),
        values,
        table_catalog: sm.table_catalog(),
        storage_manager: Arc::clone(sm),
    };
    let chunks = operator.execute(Vec::new()).expect("index scan executes");
    let mut ids = Vec::new();
    for chunk in &chunks {
        for row in 0..chunk.size {
            ids.push(chunk.get_i64(0, row).expect("primary key is Int64"));
        }
    }
    ids.sort_unstable();
    ids
}

#[test]
fn test_physical_index_scan_returns_only_live_matching_rows() {
    let (sm, _dir) = setup();

    assert_eq!(scan(&sm, INDEX, vec![Value::String("active".into())]), vec![0, 2, 4, 7]);
    assert_eq!(
        scan(
            &sm,
            INDEX,
            vec![Value::String("active".into()), Value::String("pinned".into())]
        ),
        vec![0, 2, 4, 6, 7],
        "multi-value lookup must union both key sets"
    );
    assert!(
        scan(&sm, INDEX, vec![Value::String("missing".into())]).is_empty(),
        "unknown key must return an empty chunk"
    );
}

#[test]
fn test_physical_index_scan_verifies_live_rows() {
    let (sm, _dir) = setup();
    let catalog = sm.table_catalog();

    // (a) A soft-deleted row keeps its index entry until the write path removes
    // it. Liveness (PK != NULL) alone must hide it.
    {
        let mut table = catalog.get_node_table_by_name_mut(TABLE).expect("node table present");
        table.delete_row_with_txn(5, None).expect("soft delete row 5");
    }
    assert_eq!(
        scan(&sm, INDEX, vec![Value::String("archived".into())]),
        vec![1, 9],
        "soft-deleted row 5 must not be returned"
    );

    // (b) A raw column change that skipped index maintenance leaves the row
    // under its old key. Column re-verification must reject it.
    {
        let mut table = catalog.get_node_table_by_name_mut(TABLE).expect("node table present");
        table
            .update_cell(6, 1, Value::String("archived".into()))
            .expect("update row 6");
    }
    assert!(
        scan(&sm, INDEX, vec![Value::String("pinned".into())]).is_empty(),
        "row 6 no longer matches 'pinned' and must be filtered out"
    );

    // (c) Once the write path moves the index entry (what `PhysicalSet` does),
    // the new key resolves and the old one stays empty.
    catalog.move_secondary_index_row(INDEX, 6, Some("pinned"), Some("archived"));
    assert_eq!(
        scan(&sm, INDEX, vec![Value::String("archived".into())]),
        vec![1, 6, 9],
        "row 6 must be found under its new key after the index move"
    );
    assert!(scan(&sm, INDEX, vec![Value::String("pinned".into())]).is_empty());
}

#[test]
fn test_physical_index_scan_falls_back_while_building() {
    let (sm, _dir) = setup();
    let catalog = sm.table_catalog();

    // Declared but never backfilled → `Building`; no entries are registered.
    sm.table_catalog()
        .create_secondary_index(
            "si_building".into(),
            TABLE.into(),
            "status".into(),
            IndexType::Hash,
            "si_building".into(),
        )
        .expect("create building index");
    assert!(
        !sm.table_catalog()
            .get_secondary_index("si_building")
            .expect("index present")
            .is_available(),
        "index must still be Building"
    );

    for key in ["active", "archived", "pinned", "cold", "missing"] {
        let indexed = scan(&sm, INDEX, vec![Value::String(key.into())]);
        let fallback = scan(&sm, "si_building", vec![Value::String(key.into())]);
        assert_eq!(
            fallback, indexed,
            "Building-index fallback must match the available index for '{key}'"
        );
    }

    // The fallback also applies liveness: a deleted row stays hidden.
    {
        let mut table = catalog.get_node_table_by_name_mut(TABLE).expect("node table present");
        table.delete_row_with_txn(0, None).expect("soft delete row 0");
    }
    assert_eq!(
        scan(&sm, "si_building", vec![Value::String("active".into())]),
        vec![2, 4, 7],
        "full-scan fallback must exclude soft-deleted rows too"
    );
}
