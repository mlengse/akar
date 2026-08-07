use super::ExecutionContext;
use crate::expression_evaluator::ExpressionEvaluator;
use crate::physical_operator::*;
use akar_common::error::ProcessorError;
use akar_common::types::Value;
use akar_common::vector::DataChunk;
use akar_parser::ast::Expression;
use akar_planner::logical_operator::{LogicalOperator, LogicalScanNode};
use std::sync::{Arc, Mutex};

fn extract_zone_map_predicate(expr: &Expression, columns: &[String]) -> Option<(usize, String, Value)> {
    if let Expression::BinaryOp(op, left, right) = expr {
        let op_str = match op {
            akar_parser::ast::BinaryOp::Equal => "=",
            akar_parser::ast::BinaryOp::GreaterThan => ">",
            akar_parser::ast::BinaryOp::LessThan => "<",
            akar_parser::ast::BinaryOp::GreaterThanOrEqual => ">=",
            akar_parser::ast::BinaryOp::LessThanOrEqual => "<=",
            akar_parser::ast::BinaryOp::NotEqual => "!=",
            _ => return None,
        };
        if let Expression::Variable(var_name) = &**left {
            if let Expression::Constant(c) = &**right {
                let col_name = var_name.split('.').next_back().unwrap_or(var_name);
                if let Some(col_idx) = columns.iter().position(|c| c == col_name) {
                    let val = match c {
                        akar_parser::ast::Constant::Integer(i) => Value::Int64(*i),
                        akar_parser::ast::Constant::Float(f) => Value::Double(*f),
                        akar_parser::ast::Constant::String(s) => Value::String(s.clone()),
                        akar_parser::ast::Constant::Bool(b) => Value::Bool(*b),
                        akar_parser::ast::Constant::Null => Value::Null,
                    };
                    return Some((col_idx, op_str.to_string(), val));
                }
            }
        }
    }
    None
}

pub fn map_and_execute_scan_node(
    s: &LogicalScanNode,
    next_op: Option<&LogicalOperator>,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    let mut pred_owned = None;
    if let Some(LogicalOperator::Filter(f)) = next_op {
        pred_owned = extract_zone_map_predicate(&f.expression, &s.columns);
    }

    let pred_ref = pred_owned
        .as_ref()
        .map(|(idx, op_str, val)| (*idx, op_str.as_str(), val));

    // Try Arrow fast path first (avoids Vec<Vec<Value>> intermediate)
    let (arrow_data, columns, arrow_num_rows) = ctx.resolve_scan_arrow_data(&s.table_name);
    let mut scan = if let Some(arrays) = arrow_data {
        PhysicalScan::new(s.table_name.clone(), s.table_id, arrow_num_rows.max(1)).with_arrow_data(arrays, columns)
    } else {
        // Fallback to legacy Vec<Vec<Value>> data path (MVCC-aware)
        let (data, fallback_columns, fallback_num_rows) = ctx.resolve_scan_data(&s.table_name, pred_ref);
        let mut s = PhysicalScan::new(s.table_name.clone(), s.table_id, fallback_num_rows.max(1));
        if let Some(d) = data {
            s = s.with_data(d, fallback_columns);
        }
        s
    };
    if let Some(ref fq) = s.fts_query {
        scan = scan.with_fts_query(PhysicalFtsScan {
            index_name: fq.index_name.clone(),
            query_string: fq.query_string.clone(),
            docs_table: fq.docs_table.clone(),
            terms_table: fq.terms_table.clone(),
            posting_table: fq.posting_table.clone(),
            table_catalog: ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "Table catalog required for FTS scan".to_string())?,
        });
    }
    if let Some(ref pred) = s.predicate {
        scan = scan.with_predicate(pred.clone());
        scan = scan.with_evaluator(Arc::new(Mutex::new(ExpressionEvaluator::new(
            ctx.function_registry.clone().unwrap(),
        ))));
    }
    let mut result = scan.execute(current_input)?;
    let prefix = s.alias.as_ref().unwrap_or(&s.table_name);

    for chunk in &mut result {
        chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
    }
    Ok(result)
}

pub fn map_and_execute_scan(
    op: &LogicalOperator,
    current_input: Vec<DataChunk>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<DataChunk>, ProcessorError> {
    match op {
        LogicalOperator::ScanRel(s) => {
            let (data, columns, _num_rows) = ctx.resolve_scan_data(&s.table_name, None);
            let scan = PhysicalScanRel {
                table_name: s.table_name.clone(),
                table_id: s.table_id,
                direction: s.direction.clone(),
                table_data: data,
                table_columns: columns,
            };
            let mut result = scan.execute(current_input)?;
            let prefix = &s.table_name;
            for chunk in &mut result {
                chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
            }
            Ok(result)
        }
        LogicalOperator::VectorSimilarityScan(vs) => {
            let scan = PhysicalVectorSimilarityScan {
                index_name: vs.index_name.clone(),
                index_id: vs.index_id,
                query_vector: vs.query_vector.clone(),
                top_k: vs.top_k,
                table_name: vs.table_name.clone(),
                table_catalog: ctx.table_catalog.clone(),
            };
            let result = scan.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::ArtIndexRangeScan(ars) => {
            let scan = PhysicalArtIndexRangeScan {
                table_name: ars.table_name.clone(),
                table_id: ars.table_id,
                lower_bound: ars.lower_bound.clone(),
                upper_bound: ars.upper_bound.clone(),
                lower_inclusive: ars.lower_inclusive,
                upper_inclusive: ars.upper_inclusive,
                table_catalog: ctx.table_catalog.clone(),
            };
            let mut result = scan.execute(current_input)?;
            let prefix = ars.alias.as_ref().unwrap_or(&ars.table_name);
            for chunk in &mut result {
                chunk.field_names = chunk.field_names.iter().map(|n| format!("{}.{}", prefix, n)).collect();
            }
            Ok(result)
        }
        LogicalOperator::IndexLookup(il) => {
            let table_catalog = ctx
                .table_catalog
                .clone()
                .ok_or_else(|| "No table catalog available for INDEX LOOKUP".to_string())?;
            let lookup_op = PhysicalIndexLookup {
                table_name: il.table_name.clone(),
                table_id: il.table_id,
                key_value: il.key_value.clone(),
                table_catalog,
            };
            let result = lookup_op.execute(current_input)?;
            Ok(result)
        }
        LogicalOperator::ExpressionsScan(_es) => Ok(vec![DataChunk::new(vec![], vec![])]),
        LogicalOperator::PathPropertyProbe(p) => {
            let properties = p
                .properties
                .iter()
                .map(|(t, is_node, props)| crate::physical::scan_filter::PathPropertySpec {
                    table_name: t.clone(),
                    is_node: *is_node,
                    property_names: props.clone(),
                })
                .collect();

            let probe = crate::physical::scan_filter::PhysicalPathPropertyProbe {
                node_ids_col_idx: p.node_ids_col_idx,
                edge_ids_col_idx: p.edge_ids_col_idx,
                properties,
                table_catalog: ctx
                    .table_catalog
                    .clone()
                    .ok_or_else(|| "table catalog required for PathPropertyProbe".to_string())?,
            };

            let input = if !p.children.is_empty() {
                ctx.execute_children(&p.children)?
            } else {
                current_input
            };

            let result = probe.execute(input)?;
            Ok(result)
        }
        _ => Err(format!("Not a scan operator: {:?}", op).into()),
    }
}
