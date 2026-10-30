//! Integration tests for row-scoped vector index maintenance (WP-A1, T017–T021, SC-002, FR-002–FR-004).

use akar_common::enums::CompressionType;
use akar_common::types::{LogicalTypeID, Value};
use akar_storage::table::{ColumnDefinition, TableCatalog};
use akar_vector::hnsw::DistanceMetric;

fn setup_table_and_index() -> (TableCatalog, u64, u64) {
    let catalog = TableCatalog::new();
    let columns = vec![
        ColumnDefinition {
            name: "id".into(),
            logical_type: LogicalTypeID::Int64,
            is_primary_key: true,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: "text".into(),
            logical_type: LogicalTypeID::String,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
        ColumnDefinition {
            name: "embedding".into(),
            logical_type: LogicalTypeID::Float,
            is_primary_key: false,
            compression: CompressionType::Uncompressed,
        },
    ];

    let table = catalog.create_node_table("Memory".into(), columns);
    let table_id = table.table_id;

    let vi = catalog.create_vector_index(
        "mem_vec_idx".into(),
        "Memory".into(),
        "embedding".into(),
        DistanceMetric::Cosine,
        3,
    );
    let index_id = vi.index_id;

    (catalog, table_id, index_id)
}

#[test]
fn single_row_insert_examines_exactly_one_row() {
    // T017 / SC-002 / FR-002: into a 1,000-row indexed table, a 1-row insert yields rows_examined == 1
    let (catalog, table_id, index_id) = setup_table_and_index();

    // Populate initial 1,000 rows
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        for i in 0..1000 {
            let row = vec![
                Value::Int64(i as i64),
                Value::String(format!("text_{i}")),
                Value::List(vec![Value::Float(i as f32), Value::Float(0.5), Value::Float(0.25)]),
            ];
            table.insert_row(row).unwrap();
        }
    }
    catalog.refresh_vector_index(index_id);

    {
        let vi = catalog.get_vector_index(index_id).unwrap();
        assert_eq!(vi.hnsw().len(), 1000);
    }

    // Now insert 1 new row (row_id = 1000)
    let new_row_id = {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        let row = vec![
            Value::Int64(1000),
            Value::String("new_memory".into()),
            Value::List(vec![Value::Float(1000.0), Value::Float(0.5), Value::Float(0.25)]),
        ];
        table.insert_row(row).unwrap()
    };
    assert_eq!(new_row_id, 1000);

    // Row-scoped maintenance for just the 1 new row
    let outcome = catalog.refresh_vector_index_rows(&[(table_id, new_row_id)]);
    assert_eq!(outcome.rows_examined, 1, "rows_examined must equal scope size");
    assert_eq!(outcome.rows_inserted, 1, "exactly one row inserted");
    assert_eq!(outcome.rows_replaced, 0, "no rows replaced");
    assert_eq!(outcome.rows_tombstoned, 0, "no rows tombstoned");

    let vi = catalog.get_vector_index(index_id).unwrap();
    assert_eq!(vi.hnsw().len(), 1001);
}

#[test]
fn non_indexed_column_write_examines_zero_rows() {
    // T018 / FR-003: write touching no indexed column yields empty scope and rows_examined == 0
    let (catalog, _table_id, _index_id) = setup_table_and_index();

    // Empty scope
    let outcome = catalog.refresh_vector_index_rows(&[]);
    assert_eq!(outcome.rows_examined, 0);
    assert_eq!(outcome.rows_inserted, 0);
    assert_eq!(outcome.rows_replaced, 0);
    assert_eq!(outcome.rows_tombstoned, 0);

    // Table with no vector index
    let other_columns = vec![ColumnDefinition {
        name: "id".into(),
        logical_type: LogicalTypeID::Int64,
        is_primary_key: true,
        compression: CompressionType::Uncompressed,
    }];
    let other_table = catalog.create_node_table("Other".into(), other_columns);
    let other_outcome = catalog.refresh_vector_index_rows(&[(other_table.table_id, 0)]);
    assert_eq!(other_outcome.rows_examined, 0);
}

#[test]
fn vector_column_update_replaces_row_without_duplicate() {
    // T019: vector-column update yields exactly one replacement, no duplicate node
    let (catalog, table_id, index_id) = setup_table_and_index();

    // Insert 10 rows and index them
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        for i in 0..10 {
            let row = vec![
                Value::Int64(i as i64),
                Value::String(format!("text_{i}")),
                Value::List(vec![Value::Float(i as f32), Value::Float(0.5), Value::Float(0.25)]),
            ];
            table.insert_row(row).unwrap();
        }
    }
    catalog.refresh_vector_index(index_id);

    // Update row 5 with a completely new vector
    let new_vec = vec![Value::Float(99.0), Value::Float(88.0), Value::Float(77.0)];
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        table.update_cell(5, 2, Value::List(new_vec)).unwrap();
    }

    let outcome = catalog.refresh_vector_index_rows(&[(table_id, 5)]);
    assert_eq!(outcome.rows_examined, 1);
    assert_eq!(outcome.rows_replaced, 1);
    assert_eq!(outcome.rows_inserted, 0);
    assert_eq!(outcome.rows_tombstoned, 0);
    assert!(outcome.rows_inserted + outcome.rows_replaced + outcome.rows_tombstoned <= outcome.rows_examined);

    let vi = catalog.get_vector_index(index_id).unwrap();
    assert_eq!(vi.hnsw().len(), 10, "count must remain 10 without duplicates");
    let stored = vi.hnsw().get_vector(5).expect("row 5 vector present");
    assert_eq!(stored, &[99.0, 88.0, 77.0]);
}

#[test]
fn delete_case_tombstones_and_excludes_from_search() {
    // T020: delete case tombstones row and similarity search never returns it
    let (catalog, table_id, index_id) = setup_table_and_index();

    let target_vec = [10.0f32, 20.0f32, 30.0f32];
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        for i in 0..5 {
            let vec_vals = if i == 2 {
                vec![
                    Value::Float(target_vec[0]),
                    Value::Float(target_vec[1]),
                    Value::Float(target_vec[2]),
                ]
            } else {
                vec![Value::Float(i as f32), Value::Float(0.1), Value::Float(0.2)]
            };
            table
                .insert_row(vec![
                    Value::Int64(i as i64),
                    Value::String(format!("doc_{i}")),
                    Value::List(vec_vals),
                ])
                .unwrap();
        }
    }
    catalog.refresh_vector_index(index_id);

    // Confirm row 2 is found by search before deletion
    {
        let vi = catalog.get_vector_index(index_id).unwrap();
        let hits = vi.hnsw().search(&target_vec, 3);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].1, 2usize);
    }

    // Set row 2's vector to Null (representing delete/nulling)
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        table.update_cell(2, 2, Value::Null).unwrap();
    }

    let outcome = catalog.refresh_vector_index_rows(&[(table_id, 2)]);
    assert_eq!(outcome.rows_examined, 1);
    assert_eq!(outcome.rows_tombstoned, 1);
    assert_eq!(outcome.rows_inserted, 0);
    assert_eq!(outcome.rows_replaced, 0);

    // Verify row 2 is never returned in similarity search
    {
        let vi = catalog.get_vector_index(index_id).unwrap();
        assert!(vi.hnsw().is_deleted(2));
        let hits = vi.hnsw().search(&target_vec, 5);
        for (_dist, hit_id) in hits {
            assert_ne!(hit_id, 2, "deleted row must never be returned in search");
        }
    }
}

#[test]
fn batch_insert_reconciles_all_rows_in_one_pass() {
    // T021 / FR-004: 2,000-row batch insert indexes every vector in one refresh
    let (catalog, table_id, index_id) = setup_table_and_index();

    let mut row_pairs = Vec::with_capacity(2000);
    {
        let mut table = catalog.get_node_table_mut(table_id).unwrap();
        for i in 0..2000 {
            let row = vec![
                Value::Int64(i as i64),
                Value::String(format!("batch_{i}")),
                Value::List(vec![Value::Float(i as f32), Value::Float(1.0), Value::Float(2.0)]),
            ];
            let row_id = table.insert_row(row).unwrap();
            row_pairs.push((table_id, row_id));
        }
    }

    let outcome = catalog.refresh_vector_index_rows(&row_pairs);
    assert_eq!(outcome.rows_examined, 2000);
    assert_eq!(outcome.rows_inserted, 2000);
    assert_eq!(outcome.rows_replaced, 0);
    assert_eq!(outcome.rows_tombstoned, 0);

    let vi = catalog.get_vector_index(index_id).unwrap();
    assert_eq!(vi.hnsw().len(), 2000);
}
