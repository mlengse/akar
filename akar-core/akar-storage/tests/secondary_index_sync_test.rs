//! Randomized property tests for secondary-index synchronization (T028, US4).
//!
//! Every `DELETE`/`SET` on a table with a secondary index must leave the index
//! pointing only at live rows whose indexed-column value matches the key. The
//! write path owns this synchronization (`akar-processor` calls
//! `TableCatalog::remove_secondary_index_row` / `move_secondary_index_row`);
//! this test drives the same catalog API directly over 1,000 randomized
//! operations and asserts the index contents stay exactly equal to a
//! recomputation from the table.

use akar_catalog::IndexType;
use akar_common::enums::CompressionType;
use akar_common::memory::MemoryManager;
use akar_common::types::{LogicalTypeID, Value};
use akar_storage::StorageManager;
use akar_storage::table::{ColumnDefinition, TableCatalog};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tempfile::TempDir;

const TABLE: &str = "Memory";
const INDEX: &str = "si_status";
const STATUSES: [&str; 4] = ["active", "archived", "pinned", "cold"];

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

fn setup() -> (Arc<StorageManager>, TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let sm = Arc::new(StorageManager::new(
        dir.path().to_path_buf(),
        Arc::new(MemoryManager::new(64 * 1024 * 1024)),
    ));
    sm.create_node_table(TABLE.into(), columns());
    (sm, dir)
}

/// Deterministic, dependency-free PRNG so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn pick(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn insert(sm: &StorageManager, id: i64, status: &str) -> u64 {
    let (row_id, duplicate) = sm
        .insert_row_with_dedup(
            TABLE,
            vec![
                Value::Int64(id),
                Value::String(status.to_string()),
                Value::String(format!("payload-{id}")),
            ],
            None,
        )
        .expect("insert row");
    assert!(!duplicate, "id {id} must not be reported as a duplicate");
    row_id
}

fn set_status(sm: &StorageManager, row_id: u64, old: &str, new: &str) {
    let catalog = sm.table_catalog();
    {
        let mut table = catalog.get_node_table_by_name_mut(TABLE).expect("node table present");
        table
            .update_cell(row_id, 1, Value::String(new.to_string()))
            .expect("update status cell");
    }
    catalog.move_secondary_index_row(INDEX, row_id, Some(old), Some(new));
}

fn delete(sm: &StorageManager, row_id: u64) {
    let catalog = sm.table_catalog();
    {
        let mut table = catalog.get_node_table_by_name_mut(TABLE).expect("node table present");
        table.delete_row_with_txn(row_id, None).expect("soft delete row");
    }
    catalog.remove_secondary_index_row(TABLE, row_id);
}

/// Assert the index holds exactly the live rows, under exactly their current
/// status key — no stale offsets, no missing rows, no wrong-key entries.
fn assert_index_matches_table(sm: &StorageManager, live: &BTreeMap<i64, (u64, String)>, context: &str) {
    let catalog: Arc<TableCatalog> = sm.table_catalog();
    let index = catalog.get_secondary_index(INDEX).expect("secondary index present");
    assert!(index.is_available(), "index must be available after backfill");

    let mut bm = sm.buffer_manager().lock().expect("buffer manager lock");

    let mut indexed_total = 0usize;
    for status in STATUSES {
        let mut actual = index.lookup_all(status, &mut bm).expect("lookup_all");
        actual.sort_unstable();
        indexed_total += actual.len();

        let mut expected: Vec<u64> = live
            .values()
            .filter(|(_, current)| current == status)
            .map(|(row, _)| *row)
            .collect();
        expected.sort_unstable();

        assert_eq!(
            actual, expected,
            "{context}: index key '{status}' does not match live rows"
        );
    }
    assert_eq!(
        indexed_total,
        live.len(),
        "{context}: index must contain one entry per live row"
    );
}

#[test]
fn test_secondary_index_delete_and_update() {
    let (sm, _dir) = setup();

    // Backfill from a non-empty table so the index starts `Available`.
    let mut live: BTreeMap<i64, (u64, String)> = BTreeMap::new();
    for id in 0..10i64 {
        let status = STATUSES[(id as usize) % STATUSES.len()];
        let row = insert(&sm, id, status);
        live.insert(id, (row, status.to_string()));
    }
    sm.create_secondary_index(INDEX.into(), TABLE.into(), "status".into(), IndexType::Hash, false)
        .expect("create secondary index");
    assert_index_matches_table(&sm, &live, "after backfill");

    // Delete: the index must stop returning the soft-deleted row.
    let (deleted_row, deleted_status) = live.remove(&3).expect("row 3 is live");
    delete(&sm, deleted_row);
    assert_index_matches_table(&sm, &live, "after delete");

    let hits = {
        let catalog = sm.table_catalog();
        let index = catalog.get_secondary_index(INDEX).expect("index present");
        let mut bm = sm.buffer_manager().lock().expect("bm lock");
        index.lookup_all(&deleted_status, &mut bm).expect("lookup")
    };
    assert!(
        !hits.contains(&deleted_row),
        "deleted row {deleted_row} must not survive in the secondary index"
    );

    // Update: the row must move from its old key to its new key.
    let (updated_row, old_status) = live.get(&4).cloned().expect("row 4 is live");
    set_status(&sm, updated_row, &old_status, "pinned");
    live.insert(4, (updated_row, "pinned".to_string()));
    assert_index_matches_table(&sm, &live, "after update");
}

#[test]
fn test_secondary_index_randomized_delete_update_lookup() {
    let (sm, _dir) = setup();
    sm.create_secondary_index(INDEX.into(), TABLE.into(), "status".into(), IndexType::Hash, false)
        .expect("create secondary index");

    let mut rng = Rng(0x5EED_0210);
    let mut live: BTreeMap<i64, (u64, String)> = BTreeMap::new();
    let mut next_id: i64 = 0;

    for step in 0..1_000 {
        let roll = rng.next() % 100;

        if live.is_empty() || (live.len() < 40 && roll < 40) {
            let status = STATUSES[rng.pick(STATUSES.len())];
            let row = insert(&sm, next_id, status);
            live.insert(next_id, (row, status.to_string()));
            next_id += 1;
        } else if roll < 65 {
            let keys: Vec<i64> = live.keys().copied().collect();
            let id = keys[rng.pick(keys.len())];
            let (row, old_status) = live.get(&id).cloned().expect("picked row is live");
            let new_status = STATUSES[rng.pick(STATUSES.len())];
            set_status(&sm, row, &old_status, new_status);
            live.insert(id, (row, new_status.to_string()));
        } else if roll < 85 {
            let keys: Vec<i64> = live.keys().copied().collect();
            let id = keys[rng.pick(keys.len())];
            let (row, _) = live.remove(&id).expect("picked row is live");
            delete(&sm, row);
        } else {
            // Point lookup must never surface a deleted or non-matching row.
            let status = STATUSES[rng.pick(STATUSES.len())];
            let row = sm
                .lookup_secondary_index(TABLE, "status", status)
                .expect("lookup secondary index");
            if let Some(row) = row {
                let (id, (expected_row, expected_status)) = live
                    .iter()
                    .find(|(_, (stored_row, _))| *stored_row == row)
                    .map(|(id, value)| (*id, value.clone()))
                    .expect("index returned a row that is not live");
                assert_eq!(row, expected_row);
                assert_eq!(
                    expected_status, status,
                    "row {id} was returned for key '{status}' but holds '{expected_status}'"
                );
            }
        }

        assert_index_matches_table(&sm, &live, &format!("step {step}"));
    }

    // 1,000 operations later the index must still agree with the table exactly.
    let catalog = sm.table_catalog();
    let table = catalog.get_node_table_by_name(TABLE).expect("node table present");
    let mut expected_live = 0usize;
    for row in 0..table.num_rows as usize {
        if table
            .get_value(row, 0)
            .is_some_and(|value| !matches!(*value, Value::Null))
        {
            expected_live += 1;
        }
    }
    drop(table);
    assert_eq!(
        expected_live,
        live.len(),
        "bookkeeping drifted from the table's live row count"
    );

    // And the re-derived key → offsets map covers every live row exactly once.
    let mut seen: HashMap<u64, String> = HashMap::new();
    for (id, (row, status)) in &live {
        assert!(
            seen.insert(*row, status.clone()).is_none(),
            "row for id {id} indexed twice"
        );
    }
}
