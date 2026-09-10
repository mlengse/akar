//! Schema mapping: Akar `LogicalTypeID` → Tantivy field types.
//!
//! | Akar LogicalTypeID | Tantivy type | Flags                         |
//! |--------------------|--------------|-------------------------------|
//! | `String`           | `TEXT`       | INDEXED+STORED, tokenizer `en_stem` |
//! | `Int64`            | `I64`        | FAST                          |
//! | `Float64`          | `F64`        | FAST                          |
//! | `Bool`             | `BOOL`       | INDEXED+STORED                |
//!
//! Complex / relational types (`Node`, `Rel`, `List`, `Map`, `Struct`, …) are
//! **skipped** — they cannot be meaningfully indexed for full-text search.

use akar_common::types::LogicalTypeID;
use akar_storage::table::ColumnDefinition;
use tantivy::schema::{FAST, INDEXED, IndexRecordOption, STORED, Schema, SchemaBuilder, TEXT, TextFieldIndexing};

use crate::tokenizer::EN_STEM;

/// Build a Tantivy [`Schema`] from Akar source-table column definitions.
///
/// Each column is mapped according to the table above. Columns whose
/// `LogicalTypeID` has no Tantivy equivalent are silently skipped.
pub fn build_tantivy_schema(columns: &[ColumnDefinition]) -> Schema {
    let mut builder = SchemaBuilder::new();
    for col in columns {
        let _ = add_field(&mut builder, col); // field added; caller tracks name→field separately
    }
    builder.build()
}

/// Map a single [`ColumnDefinition`] into a Tantivy field on `builder`.
///
/// Returns `Some(Field)` when the type is indexable, `None` otherwise.
fn add_field(builder: &mut SchemaBuilder, col: &ColumnDefinition) -> Option<tantivy::schema::Field> {
    let name = &col.name;
    match col.logical_type {
        // ── Text: full-text indexed with `en_stem` + stored for retrieval ──
        // The tokenizer name is resolved from the index's TokenizerManager at
        // index/query time (registered by `TantivyIndex` constructors).
        LogicalTypeID::String => Some(
            builder.add_text_field(
                name,
                TEXT.set_indexing_options(
                    TextFieldIndexing::default()
                        .set_tokenizer(EN_STEM)
                        .set_index_option(IndexRecordOption::WithFreqsAndPositions),
                )
                .set_stored(),
            ),
        ),

        // ── Numerics: FAST for sorting / faceting ──
        LogicalTypeID::Int64 | LogicalTypeID::Serial => Some(builder.add_i64_field(name, FAST)),
        LogicalTypeID::Float => Some(builder.add_f64_field(name, FAST)),
        LogicalTypeID::Double => Some(builder.add_f64_field(name, FAST)),

        // ── Bool: indexed + stored ──
        LogicalTypeID::Bool => Some(builder.add_bool_field(name, INDEXED | STORED)),

        // ── Date / Timestamp → tantivy Date (stored) ──
        LogicalTypeID::Date
        | LogicalTypeID::Timestamp
        | LogicalTypeID::TimestampSec
        | LogicalTypeID::TimestampMs
        | LogicalTypeID::TimestampNs
        | LogicalTypeID::TimestampTz => Some(builder.add_date_field(name, STORED)),

        // ── Integers that map to i64 ──
        LogicalTypeID::Int32 | LogicalTypeID::Int16 | LogicalTypeID::Int8 => Some(builder.add_i64_field(name, FAST)),

        // ── Unsigned integers → u64 FAST ──
        LogicalTypeID::UInt64 => Some(builder.add_u64_field(name, FAST)),
        LogicalTypeID::UInt32 | LogicalTypeID::UInt16 | LogicalTypeID::UInt8 => Some(builder.add_u64_field(name, FAST)),

        // ── Unsupported / relational types: skip ──
        LogicalTypeID::Any
        | LogicalTypeID::Node
        | LogicalTypeID::Rel
        | LogicalTypeID::RecursiveRel
        | LogicalTypeID::Int128
        | LogicalTypeID::UInt128
        | LogicalTypeID::Decimal
        | LogicalTypeID::Interval
        | LogicalTypeID::Time
        | LogicalTypeID::InternalID
        | LogicalTypeID::Json
        | LogicalTypeID::Blob
        | LogicalTypeID::Uuid
        | LogicalTypeID::List
        | LogicalTypeID::Array
        | LogicalTypeID::Map
        | LogicalTypeID::Struct
        | LogicalTypeID::Union => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, logical_type: LogicalTypeID) -> ColumnDefinition {
        ColumnDefinition {
            name: name.to_string(),
            logical_type,
            is_primary_key: false,
            compression: akar_common::enums::CompressionType::Uncompressed,
        }
    }

    #[test]
    fn test_schema_document_table() {
        let columns = vec![
            col("id", LogicalTypeID::Int64),
            col("title", LogicalTypeID::String),
            col("content", LogicalTypeID::String),
            col("score", LogicalTypeID::Float),
        ];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        assert_eq!(fields.len(), 4);
    }

    #[test]
    fn test_string_maps_to_text_indexed_stored() {
        let columns = vec![col("body", LogicalTypeID::String)];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        assert_eq!(fields.len(), 1);
        let (_field, entry) = &fields[0];
        assert_eq!(entry.name(), "body");
        assert!(entry.is_indexed(), "TEXT field must be indexed");
        assert!(entry.is_stored(), "TEXT field must be stored");
    }

    #[test]
    fn test_int64_maps_to_i64_fast() {
        let columns = vec![col("count", LogicalTypeID::Int64)];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        let (_field, entry) = &fields[0];
        assert!(entry.is_fast(), "Int64 must be FAST");
        assert!(!entry.is_indexed(), "Int64 should not be indexed (only FAST)");
    }

    #[test]
    fn test_float_maps_to_f64_fast() {
        let columns = vec![col("price", LogicalTypeID::Float)];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        let (_field, entry) = &fields[0];
        assert!(entry.is_fast());
    }

    #[test]
    fn test_bool_maps_to_bool_indexed_stored() {
        let columns = vec![col("active", LogicalTypeID::Bool)];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        let (_field, entry) = &fields[0];
        assert!(entry.is_indexed(), "Bool must be indexed");
        assert!(entry.is_stored(), "Bool must be stored");
    }

    #[test]
    fn test_relational_types_skipped() {
        let columns = vec![
            col("node_col", LogicalTypeID::Node),
            col("rel_col", LogicalTypeID::Rel),
            col("list_col", LogicalTypeID::List),
        ];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        assert_eq!(fields.len(), 0);
    }

    #[test]
    fn test_mixed_columns() {
        let columns = vec![
            col("id", LogicalTypeID::Int64),
            col("name", LogicalTypeID::String),
            col("active", LogicalTypeID::Bool),
            col("node_ref", LogicalTypeID::Node), // skipped
            col("score", LogicalTypeID::Double),
        ];
        let schema = build_tantivy_schema(&columns);
        let fields: Vec<_> = schema.fields().collect();
        assert_eq!(fields.len(), 4); // node_ref skipped
    }
}
