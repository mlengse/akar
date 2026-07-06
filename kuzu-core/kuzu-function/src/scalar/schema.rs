use crate::registry::*;
use kuzu_common::types::Value;

// ==================== Schema Functions ====================

/// Evaluate a schema function: OFFSET, ID, START_NODE, END_NODE, LABEL.
pub(crate) fn evaluate_schema(op: SchemaOp, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err(format!("Schema function {:?} requires an argument", op));
    }

    match op {
        SchemaOp::Offset => {
            // OFFSET(v) → returns the internal offset (row number) of a node/rel ID
            match &args[0] {
                Value::InternalID(id) => Ok(Value::Int64(id.offset as i64)),
                Value::Struct(entries) => {
                    // Try to extract offset from a struct with "_id" field
                    for (k, v) in entries {
                        if k == "_id" {
                            if let Value::InternalID(inner) = v {
                                return Ok(Value::Int64(inner.offset as i64));
                            }
                        }
                    }
                    Err("OFFSET: argument struct has no _id field".into())
                }
                other => Err(format!(
                    "OFFSET requires a node/rel value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::Id => {
            // ID(v) → returns the InternalID (offset + table_id)
            match &args[0] {
                Value::InternalID(id) => Ok(Value::InternalID(*id)),
                Value::Struct(entries) => {
                    // Try to extract id from a struct with "_id" field
                    for (k, v) in entries {
                        if k == "_id" {
                            return Ok(v.clone());
                        }
                    }
                    Err("ID: argument struct has no _id field".into())
                }
                other => Err(format!("ID requires a node/rel value, got {:?}", other.logical_type())),
            }
        }
        SchemaOp::StartNode => {
            // START_NODE(r) → returns the source node of a relationship
            match &args[0] {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if k == "_src" {
                            return Ok(v.clone());
                        }
                    }
                    Err("START_NODE: rel struct has no _src field".into())
                }
                other => Err(format!(
                    "START_NODE requires a relationship value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::EndNode => {
            // END_NODE(r) → returns the target node of a relationship
            match &args[0] {
                Value::Struct(entries) => {
                    for (k, v) in entries {
                        if k == "_dst" {
                            return Ok(v.clone());
                        }
                    }
                    Err("END_NODE: rel struct has no _dst field".into())
                }
                other => Err(format!(
                    "END_NODE requires a relationship value, got {:?}",
                    other.logical_type()
                )),
            }
        }
        SchemaOp::Label => {
            // LABEL(v) → returns the table/label name as a string
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.clone())),
                Value::Struct(entries) => {
                    // Try _label field first
                    for (k, v) in entries {
                        if k == "_label" {
                            return Ok(v.clone());
                        }
                    }
                    // Fallback: try _id and look up by table_id
                    for (k, v) in entries {
                        if k == "_id" {
                            if let Value::InternalID(id) = v {
                                return Ok(Value::String(format!("Table({})", id.table_id)));
                            }
                        }
                    }
                    Err("LABEL: argument struct has no _label field".into())
                }
                Value::InternalID(id) => Ok(Value::String(format!("Table({})", id.table_id))),
                other => Err(format!(
                    "LABEL requires a node/rel/string value, got {:?}",
                    other.logical_type()
                )),
            }
        }
    }
}
