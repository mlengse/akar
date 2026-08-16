//! Query planner — converts bound statements into logical query plans.
//!
//! Builds a tree of logical operators from the bound AST:
//! - MATCH → ScanNode / ScanRel
//! - Multiple MATCH patterns combined via join tree (HashJoin / CrossProduct)
//! - WHERE → Filter (applied after joins)
//! - RETURN → Projection (topmost operator)

use crate::join_order::{build_join_tree, build_wcoj_intersect, flatten_join_plan};
use crate::logical_operator::*;
use akar_binder::bound_statement::*;
use akar_common::error::PlannerError;
use akar_parser::ast::Expression;
use std::collections::HashSet;

/// Whether an ORDER BY key expression is produced by the projection output.
///
/// A key is covered when it matches a projection item's alias or is identical
/// to one of the projected expressions, or when the projection returns the bare
/// node variable and the key accesses a property of that variable. Keys that are
/// not covered (e.g. `m.access_count` in `RETURN m.id, m.label ORDER BY
/// m.access_count`) must sort on the pre-projection columns (P53.37).
pub fn projection_covers_sort_key(projected: &[BoundExpression], key: &Expression) -> bool {
    for be in projected {
        if let Some(alias) = &be.alias {
            if sort_key_matches_name(key, alias) {
                return true;
            }
        }
        if &be.expression == key {
            return true;
        }
    }
    if let Expression::PropertyAccess(obj, _) = key {
        if let Expression::Variable(var) = &**obj {
            let bare = Expression::Variable(var.clone());
            if projected.iter().any(|be| be.expression == bare) {
                return true;
            }
        }
    }
    false
}

/// Whether a sort key refers to the column named by `name` (`x` or `node.prop`).
fn sort_key_matches_name(key: &Expression, name: &str) -> bool {
    match key {
        Expression::Variable(v) => v == name,
        Expression::PropertyAccess(obj, prop) => {
            if let Expression::Variable(var) = &**obj {
                format!("{var}.{prop}") == name
            } else {
                false
            }
        }
        _ => false,
    }
}

/// The query planner transforms bound statements into logical query plans.
pub struct QueryPlanner;

impl QueryPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(&self, statement: BoundStatement) -> Result<Vec<LogicalOperator>, PlannerError> {
        match statement {
            BoundStatement::BoundQuery(query) => self.plan_query(query),
            BoundStatement::BoundCopyFrom(c) => self.plan_copy_from(c),
            BoundStatement::BoundUnion(u) => self.plan_union(u),
            BoundStatement::BoundMerge(m) => self.plan_merge(m),
            BoundStatement::BoundExplain(e) => self.plan_explain(e),
            BoundStatement::BoundCreateNodeTable(t) => self.plan_create_node_table(t),
            BoundStatement::BoundCreateRelTable(t) => self.plan_create_rel_table(t),
            BoundStatement::BoundDropTable(t) => self.plan_drop_table(t),
            BoundStatement::BoundAlterTable(a) => self.plan_alter_table(a),
            BoundStatement::BoundCreateIndex(idx) => self.plan_create_index(idx),
            BoundStatement::BoundDropIndex(idx) => self.plan_drop_index(idx),
            BoundStatement::BoundCreateVectorIndex(idx) => self.plan_create_vector_index(idx),
            BoundStatement::BoundCreateSequence(s) => self.plan_create_sequence(s),
            BoundStatement::BoundDropSequence(s) => self.plan_drop_sequence(s),
            BoundStatement::BoundCreateDml(c) => self.plan_create_dml(c),
            BoundStatement::BoundExportDatabase(e) => self.plan_export_database(e),
            BoundStatement::BoundImportDatabase(i) => self.plan_import_database(i),
            BoundStatement::BoundCreateFtsIndex(c) => {
                Ok(vec![LogicalOperator::CreateFtsIndex(LogicalCreateFtsIndex {
                    index_name: c.index_name,
                    table_name: c.table_name,
                    column_name: c.column_name,
                    if_not_exists: c.if_not_exists,
                    docs_table: c.docs_table,
                    terms_table: c.terms_table,
                    posting_table: c.posting_table,
                    cardinality: 1,
                })])
            }
            BoundStatement::BoundStandaloneCall(c) => self.plan_standalone_call(c),
            _ => Ok(Vec::new()),
        }
    }

    /// Plan an EXPLAIN statement.
    ///
    /// Plans the inner statement first, then wraps the result in a
    /// LogicalExplain operator that will serialize the plan tree to text.
    fn plan_explain(&self, e: BoundExplain) -> Result<Vec<LogicalOperator>, PlannerError> {
        let inner_plan = self.plan(*e.inner)?;
        // Take the last operator of the inner plan as the tree root to explain
        let inner_op = if inner_plan.is_empty() {
            return Err("Cannot EXPLAIN an empty plan".into());
        } else if inner_plan.len() == 1 {
            inner_plan.into_iter().next().unwrap()
        } else {
            // Wrap multi-operator pipeline in a projection root
            LogicalOperator::Projection(LogicalProjection {
                expressions: Vec::new(),
                children: inner_plan,
                cardinality: 0,
            })
        };

        Ok(vec![LogicalOperator::Explain(LogicalExplain {
            inner: Box::new(inner_op),
            explain_type: e.explain_type,
            cardinality: 1,
        })])
    }

    fn plan_copy_from(&self, c: BoundCopyFrom) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CopyFrom(LogicalCopyFrom {
            table_name: c.table_name,
            table_id: c.table_id,
            file_path: c.file_path,
            options: c.options,
            cardinality: 0,
        })])
    }

    fn plan_standalone_call(&self, c: BoundStandaloneCall) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::StandaloneCall(LogicalStandaloneCall {
            function_name: c.function_name,
            args: c.args,
            cardinality: 1,
        })])
    }

    // ==================== DDL Planning ====================

    fn plan_create_node_table(&self, t: BoundCreateNodeTable) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CreateNodeTable(LogicalCreateNodeTable {
            name: t.name,
            columns: t.columns,
            primary_key: t.primary_key,
            cardinality: 1,
        })])
    }

    fn plan_create_rel_table(&self, t: BoundCreateRelTable) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CreateRelTable(LogicalCreateRelTable {
            name: t.name,
            from: t.from,
            to: t.to,
            columns: t.columns,
            cardinality: 1,
        })])
    }

    fn plan_drop_table(&self, t: BoundDropTable) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::DropTable(LogicalDropTable {
            name: t.name,
            cardinality: 1,
        })])
    }

    fn plan_alter_table(&self, a: BoundAlterTable) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::AlterTable(LogicalAlterTable {
            table_name: a.table_name,
            action: a.action,
            cardinality: 1,
        })])
    }

    fn plan_create_index(&self, idx: BoundCreateIndex) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CreateIndex(LogicalCreateIndex {
            index_type: idx.index_type,
            index_name: idx.index_name,
            table_name: idx.table_name,
            column_name: idx.column_name,
            cardinality: 1,
        })])
    }

    fn plan_drop_index(&self, idx: BoundDropIndex) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::DropIndex(LogicalDropIndex {
            index_name: idx.index_name,
            table_name: idx.table_name,
            cardinality: 1,
        })])
    }

    fn plan_create_vector_index(&self, idx: BoundCreateVectorIndex) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CreateVectorIndex(LogicalCreateVectorIndex {
            index_name: idx.index_name,
            table_name: idx.table_name,
            column_name: idx.column_name,
            metric: idx.metric,
            dimensions: idx.dimensions,
            cardinality: 1,
        })])
    }

    fn plan_create_sequence(&self, s: BoundCreateSequence) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::CreateSequence(LogicalCreateSequence {
            name: s.name,
            if_not_exists: s.if_not_exists,
            or_replace: s.or_replace,
            start_with: s.start_with,
            increment: s.increment,
            min_value: s.min_value,
            max_value: s.max_value,
            cycle: s.cycle,
            cardinality: 1,
        })])
    }

    fn plan_drop_sequence(&self, s: BoundDropSequence) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::DropSequence(LogicalDropSequence {
            name: s.name,
            if_exists: s.if_exists,
            cardinality: 1,
        })])
    }

    fn plan_create_dml(&self, c: BoundCreateDml) -> Result<Vec<LogicalOperator>, PlannerError> {
        let first_node = c.patterns.iter().find_map(|p| p.node.clone());
        let (table_name, table_id, properties) = match first_node {
            Some(n) => (n.table_name, n.table_id, n.properties),
            None => (String::new(), 0, Vec::new()),
        };
        Ok(vec![LogicalOperator::CreateDml(LogicalCreateDml {
            table_name,
            table_id,
            properties,
            cardinality: 1,
        })])
    }

    fn plan_export_database(&self, e: BoundExportDatabase) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::ExportDatabase(LogicalExportDatabase {
            file_path: e.file_path,
            file_type: e.file_type,
            schema_only: e.schema_only,
            options: e.options,
            cardinality: 1,
        })])
    }

    fn plan_import_database(&self, i: BoundImportDatabase) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(vec![LogicalOperator::ImportDatabase(LogicalImportDatabase {
            file_path: i.file_path,
            query: i.query,
            index_query: i.index_query,
            cardinality: 1,
        })])
    }

    /// Plan a MERGE statement.
    ///
    /// Converts the bound merge into a `LogicalMerge` operator with
    /// ON MATCH SET and ON CREATE SET as `LogicalSet` sub-operators.
    fn plan_merge(&self, m: BoundMerge) -> Result<Vec<LogicalOperator>, PlannerError> {
        Ok(self.build_merge_operators(&m))
    }

    /// Build the update operator(s) for a MERGE clause. Edge MERGE
    /// (`MERGE (a)-[r:R]->(b)`, P53.20) emits a `LogicalMergeRel`; otherwise a
    /// node `LogicalMerge` is produced.
    fn build_merge_operators(&self, m: &BoundMerge) -> Vec<LogicalOperator> {
        let on_match: Vec<LogicalSet> = m
            .on_match
            .iter()
            .map(|item| LogicalSet {
                table_name: item.table_name.clone(),
                table_id: item.table_id,
                is_node: item.is_node,
                items: vec![SetItem {
                    column_name: item.column_name.clone(),
                    column_idx: item.column_idx,
                    value: item.value.clone(),
                }],
                cardinality: 0,
            })
            .collect();

        let on_create: Vec<LogicalSet> = m
            .on_create
            .iter()
            .map(|item| LogicalSet {
                table_name: item.table_name.clone(),
                table_id: item.table_id,
                is_node: item.is_node,
                items: vec![SetItem {
                    column_name: item.column_name.clone(),
                    column_idx: item.column_idx,
                    value: item.value.clone(),
                }],
                cardinality: 0,
            })
            .collect();

        if let Some(edge) = m.patterns.iter().find_map(|p| p.edge.clone()) {
            return vec![LogicalOperator::MergeRel(LogicalMergeRel {
                rel_table_name: edge.table_name,
                rel_table_id: edge.table_id,
                edge_var: edge.variable.unwrap_or_default(),
                src_node_var: edge.src_var,
                dst_node_var: edge.dst_var,
                properties: edge.properties,
                on_match,
                on_create,
                cardinality: 0,
            })];
        }

        vec![LogicalOperator::Merge(LogicalMerge {
            table_name: m.table_name.clone(),
            table_id: m.table_id,
            properties: m.properties.clone(),
            on_match,
            on_create,
            cardinality: 0,
        })]
    }

    /// Plan a UNION or UNION ALL statement.
    ///
    /// Plans left and right sub-queries independently, then wraps each
    /// side's pipeline (potentially multiple operators) into a synthetic
    /// projection root so that `LogicalUnion` can store them as tree children.
    fn plan_union(&self, u: BoundUnion) -> Result<Vec<LogicalOperator>, PlannerError> {
        let left_plan = self.plan_query(*u.left)?;
        let right_plan = self.plan_query(*u.right)?;

        let left_op = if left_plan.len() == 1 {
            left_plan.into_iter().next().unwrap()
        } else {
            // Wrap multi-operator pipeline in a projection root
            LogicalOperator::Projection(LogicalProjection {
                expressions: Vec::new(),
                children: left_plan,
                cardinality: 0,
            })
        };

        let right_op = if right_plan.len() == 1 {
            right_plan.into_iter().next().unwrap()
        } else {
            LogicalOperator::Projection(LogicalProjection {
                expressions: Vec::new(),
                children: right_plan,
                cardinality: 0,
            })
        };

        Ok(vec![LogicalOperator::Union(LogicalUnion {
            left: Box::new(left_op),
            right: Box::new(right_op),
            all: u.all,
            cardinality: 0,
        })])
    }

    pub fn plan_query(&self, query: BoundQuery) -> Result<Vec<LogicalOperator>, PlannerError> {
        let mut scan_ops: Vec<LogicalOperator> = Vec::new();
        let mut filter_expr: Option<BoundExpression> = None;
        let mut projection: Option<LogicalProjection> = None;
        let mut distinct = false;
        let mut delete_exprs: Vec<LogicalOperator> = Vec::new();
        let mut extend_ops: Vec<LogicalOperator> = Vec::new();
        // ORDER BY / LIMIT / SKIP from RETURN clause
        let mut order_by: Option<Vec<BoundOrderByItem>> = None;
        let mut limit: Option<u64> = None;
        let mut skip: Option<u64> = None;
        // Flag to skip destination node pattern consumed by RecursiveExtend or Extend
        let mut skip_next_node = false;
        // Node variables already bound to a scan in the current pipeline.
        // Prevents duplicate scans for a shared variable across comma patterns
        // (P48.1): `MATCH (a)-[:r1]->(b), (b)-[:r3]->(c)` must not scan `b` twice.
        let mut available_vars: HashSet<String> = HashSet::new();

        for clause in query.clauses {
            match clause {
                BoundClause::BoundMatch(mut m) => {
                    let mut fts_to_assign = m.fts_query.as_ref().map(|fq| LogicalFtsScan {
                        index_name: fq.index_name.clone(),
                        query_string: fq.query_string.clone(),
                        docs_table: fq.docs_table.clone(),
                        terms_table: fq.terms_table.clone(),
                        posting_table: fq.posting_table.clone(),
                        table_name: fq.table_name.clone(),
                        column_name: fq.column_name.clone(),
                        cardinality: 0,
                    });
                    let patterns: Vec<BoundPattern> = std::mem::take(&mut m.patterns);

                    // WCOJ pass: `MATCH (a)-[:r1]->(b), (a)-[:r2]->(c)` becomes a single
                    // Intersect that probes the shared node once across all build sides.
                    // Triangle queries additionally get closure-edge Extend+Filter ops.
                    if let Some((wcoj_op, wcoj_trailing)) = build_wcoj_intersect(&patterns) {
                        scan_ops.push(wcoj_op);
                        extend_ops.extend(wcoj_trailing);
                    } else {
                        let mut patterns_iter = patterns.into_iter().peekable();
                        let _skip_next_node = false;
                        while let Some(pattern) = patterns_iter.next() {
                            // If the previous pattern consumed this dest node, skip the node scan
                            let skip_current_node_scan = skip_next_node;
                            skip_next_node = false;

                            // Check if this pattern has a var-length edge → create RecursiveExtend
                            if let Some(ref edge) = pattern.edge {
                                let is_var_length = edge.lower_bound.is_some() || edge.upper_bound.is_some();
                                if is_var_length {
                                    let lb = edge.lower_bound.unwrap_or(0);
                                    let ub = edge.upper_bound.unwrap_or(1);
                                    let direction = match edge.direction {
                                        akar_parser::ast::EdgeDirection::LeftToRight => {
                                            akar_common::enums::ExtendDirection::Fwd
                                        }
                                        akar_parser::ast::EdgeDirection::RightToLeft => {
                                            akar_common::enums::ExtendDirection::Bwd
                                        }
                                        akar_parser::ast::EdgeDirection::Both => {
                                            akar_common::enums::ExtendDirection::Both
                                        }
                                    };

                                    // Scan source node
                                    let node_var = &pattern.node_variable;
                                    let var_len_src_bound =
                                        node_var.as_ref().is_some_and(|v| available_vars.contains(v));
                                    if !skip_current_node_scan && !var_len_src_bound {
                                        if let Some(label) = pattern.node_label {
                                            scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                                table_name: label,
                                                table_id: pattern.node_table_id.unwrap_or(0),
                                                alias: node_var.clone(),
                                                columns: Vec::new(),
                                                cardinality: 0,
                                                fts_query: fts_to_assign.take(),
                                                predicate: None,
                                            }));
                                            if let Some(v) = node_var {
                                                available_vars.insert(v.clone());
                                            }
                                        }
                                    }

                                    // Create RecursiveExtend (consumes destination node pattern)
                                    let rel_table_ids = edge.rel_table_id.map_or(vec![], |id| vec![id]);
                                    let rel_labels = edge.label.as_ref().map_or(vec![], |l| vec![l.clone()]);
                                    let target_var = patterns_iter
                                        .peek()
                                        .and_then(|p| p.node_variable.clone())
                                        .unwrap_or_default();
                                    scan_ops.push(LogicalOperator::RecursiveExtend(LogicalRecursiveExtend {
                                        source_var: node_var.clone().unwrap_or_default(),
                                        source_table_id: pattern.node_table_id.unwrap_or(0),
                                        edge_var: edge.variable.clone(),
                                        target_var: target_var.clone(),
                                        rel_table_ids,
                                        rel_labels,
                                        lower_bound: lb,
                                        upper_bound: ub,
                                        direction,
                                        semantic: akar_common::enums::PathSemantic::Walk,
                                        weight_property: None,
                                        cost_output_name: None,
                                        cardinality: 0,
                                    }));
                                    // The destination node is produced by the RecursiveExtend (P48.1).
                                    if !target_var.is_empty() {
                                        available_vars.insert(target_var);
                                    }
                                    // Skip the destination node pattern scan
                                    skip_next_node = true;
                                    continue;
                                }

                                // Regular (non-var-length) edge → create Extend
                                // Scan the source node (clone what we need before pattern is moved)
                                let src_node_var = pattern.node_variable.clone();
                                let src_already_bound =
                                    src_node_var.as_ref().is_some_and(|v| available_vars.contains(v));
                                if !skip_current_node_scan && !src_already_bound {
                                    if let Some(label) = &pattern.node_label {
                                        scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                            table_name: label.clone(),
                                            table_id: pattern.node_table_id.unwrap_or(0),
                                            alias: src_node_var.clone(),
                                            columns: Vec::new(),
                                            cardinality: 0,
                                            fts_query: fts_to_assign.take(),
                                            predicate: None,
                                        }));
                                        if let Some(v) = &src_node_var {
                                            available_vars.insert(v.clone());
                                        }
                                    }
                                }

                                // Create Extend which replaces ScanRel + destination ScanNode
                                if let Some(rel_label) = &edge.label {
                                    let dest_pattern = patterns_iter.peek();
                                    let dst_var =
                                        dest_pattern.and_then(|p| p.node_variable.clone()).unwrap_or_default();
                                    let dst_table_name =
                                        dest_pattern.and_then(|p| p.node_label.clone()).unwrap_or_default();
                                    let dst_table_id = dest_pattern.and_then(|p| p.node_table_id).unwrap_or(0);

                                    extend_ops.push(LogicalOperator::Extend(LogicalExtend {
                                        rel_table_name: rel_label.clone(),
                                        rel_table_id: edge.rel_table_id.unwrap_or(0),
                                        rel_var: edge.variable.clone().unwrap_or_default(),
                                        bound_node_var: src_node_var.unwrap_or_default(),
                                        direction: edge.direction.clone(),
                                        dst_node_var: dst_var.clone(),
                                        dst_table_name,
                                        dst_table_id,
                                        cardinality: 0,
                                    }));
                                    // The destination node is produced by this Extend — it becomes
                                    // available to later patterns without a fresh scan (P48.1).
                                    if !dst_var.is_empty() {
                                        available_vars.insert(dst_var);
                                    }

                                    // Skip the destination node pattern scan
                                    skip_next_node = true;
                                    continue;
                                }
                            }

                            // Regular (non-var-length) pattern without edge: Scan node only
                            if !skip_current_node_scan {
                                if let Some(label) = pattern.node_label {
                                    let var = pattern.node_variable.clone();
                                    if !var.as_ref().is_some_and(|v| available_vars.contains(v)) {
                                        scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                            table_name: label,
                                            table_id: pattern.node_table_id.unwrap_or(0),
                                            alias: var.clone(),
                                            columns: Vec::new(),
                                            cardinality: 0,
                                            fts_query: fts_to_assign.take(),
                                            predicate: None,
                                        }));
                                        if let Some(v) = var {
                                            available_vars.insert(v);
                                        }
                                    }
                                }
                            }
                        }
                    } // end else (regular pattern loop)
                }

                BoundClause::BoundWhere(w) => {
                    if !delete_exprs.is_empty() {
                        // WHERE appearing after a WITH/UNWIND/update clause filters the
                        // in-flight pipeline, not the scan. Push it as a Filter on top of
                        // the clauses accumulated so far (P53.14).
                        delete_exprs.push(LogicalOperator::Filter(LogicalFilter {
                            expression: w.expression.expression,
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                    } else {
                        // Combine with any prior WHERE clause (e.g. an implicit one
                        // generated by the binder from inline node-properties) —
                        // otherwise the earlier predicate is silently dropped
                        // (P48.17 BUG-A).
                        filter_expr = Some(match filter_expr.take() {
                            Some(prev) => BoundExpression {
                                expression: Expression::BinaryOp(
                                    akar_parser::ast::BinaryOp::And,
                                    Box::new(prev.expression),
                                    Box::new(w.expression.expression),
                                ),
                                resolved_type: akar_common::types::LogicalTypeID::Bool,
                                is_constant: prev.is_constant && w.expression.is_constant,
                                alias: None,
                            },
                            None => w.expression,
                        });
                    }
                }
                BoundClause::BoundReturn(r) => {
                    distinct = r.distinct;
                    order_by = r.order_by;
                    limit = r.limit;
                    skip = r.skip;
                    projection = Some(LogicalProjection {
                        expressions: r.expressions,
                        children: Vec::new(),
                        cardinality: 0,
                    });
                }
                BoundClause::BoundWith(r) => {
                    delete_exprs.push(LogicalOperator::Projection(LogicalProjection {
                        expressions: r.expressions,
                        children: Vec::new(),
                        cardinality: 0,
                    }));
                }
                BoundClause::BoundOptionalMatch(om) => {
                    // P53.25: Detect the bound-edge-probe shape — an optional
                    // pattern that is a single edge between two node variables
                    // already bound by the required side:
                    //   `OPTIONAL MATCH (a)-[existing:Connected]-(b)`
                    // where `a`/`b` come from the mandatory MATCH and carry no
                    // label/property constraints here. Such patterns are executed
                    // by probing the relationship adjacency per input row
                    // (OptionalExtend) rather than scanning + outer-joining, so
                    // the compound `a._id`/`b._id` endpoints survive.
                    let edge_probe = om.patterns.len() == 2
                        && om.patterns.iter().all(|p| {
                            p.node_label.is_none()
                                && p.properties.is_empty()
                                && p.node_variable.as_ref().is_some_and(|v| available_vars.contains(v))
                        })
                        && om.patterns[0].edge.as_ref().is_some_and(|e| {
                            e.variable.is_some()
                                && e.label.as_ref().is_some_and(|l| !l.is_empty())
                                && e.properties.is_empty()
                                && e.lower_bound.is_none()
                                && e.upper_bound.is_none()
                        })
                        && om.patterns[1].edge.is_none();

                    // Build the current required-side pipeline (left child)
                    let mut left_pipeline: Vec<LogicalOperator> = Vec::new();
                    if !scan_ops.is_empty() {
                        if scan_ops.len() == 1 {
                            left_pipeline.push(scan_ops.into_iter().next().unwrap());
                        } else {
                            let join_plan = build_join_tree(scan_ops, filter_expr.as_ref());
                            let flattened = flatten_join_plan(&join_plan);
                            left_pipeline.extend(flattened);
                        }
                    }
                    if let Some(expr) = filter_expr.take() {
                        left_pipeline.push(LogicalOperator::Filter(LogicalFilter {
                            expression: expr.expression,
                            children: Vec::new(),
                            cardinality: 0,
                        }));
                    }
                    if let Some(proj) = projection.take() {
                        left_pipeline.push(LogicalOperator::Projection(proj));
                    }
                    if edge_probe {
                        let edge = om.patterns[0].edge.as_ref().unwrap();
                        let src_var = om.patterns[0].node_variable.clone().unwrap();
                        let dst_var = om.patterns[1].node_variable.clone().unwrap();
                        delete_exprs.push(LogicalOperator::OptionalExtend(LogicalOptionalExtend {
                            children: left_pipeline,
                            rel_table_name: edge.label.clone().unwrap(),
                            rel_table_id: edge.rel_table_id.unwrap_or(0),
                            rel_var: edge.variable.clone().unwrap(),
                            src_node_var: src_var,
                            dst_node_var: dst_var,
                            direction: edge.direction.clone(),
                            cardinality: 0,
                        }));
                    } else {
                        let left_op = if left_pipeline.len() == 1 {
                            left_pipeline.into_iter().next().unwrap()
                        } else if left_pipeline.is_empty() {
                            // Empty left side — use a dummy scan
                            LogicalOperator::ScanNode(LogicalScanNode {
                                table_name: String::new(),
                                table_id: 0,
                                alias: None,
                                columns: Vec::new(),
                                cardinality: 0,
                                fts_query: None,
                                predicate: None,
                            })
                        } else {
                            LogicalOperator::Projection(LogicalProjection {
                                expressions: Vec::new(),
                                children: left_pipeline,
                                cardinality: 0,
                            })
                        };

                        // Build the optional-side pipeline (right child)
                        let mut right_ops: Vec<LogicalOperator> = Vec::new();
                        for pattern in &om.patterns {
                            if let Some(label) = &pattern.node_label {
                                right_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                    table_name: label.clone(),
                                    table_id: pattern.node_table_id.unwrap_or(0),
                                    alias: pattern.node_variable.clone(),
                                    columns: Vec::new(),
                                    cardinality: 0,
                                    fts_query: None,
                                    predicate: None,
                                }));
                            }
                            if let Some(edge) = &pattern.edge
                                && let Some(rel_label) = &edge.label
                            {
                                right_ops.push(LogicalOperator::ScanRel(LogicalScanRel {
                                    table_name: rel_label.clone(),
                                    table_id: edge.rel_table_id.unwrap_or(0),
                                    direction: edge.direction.clone(),
                                    cardinality: 0,
                                }));
                            }
                        }
                        // Apply inline node/edge property predicates to the optional
                        // side, mirroring the implicit WHERE the binder generates
                        // for MATCH. Without this, `OPTIONAL MATCH (m:T {id: 999})`
                        // scans every T row (predicate silently dropped) and the
                        // left-outer merge degenerates into a cross product.
                        let mut inline_exprs: Vec<Expression> = Vec::new();
                        for pattern in &om.patterns {
                            if let Some(node_var) = &pattern.node_variable {
                                for (key, val_expr) in &pattern.properties {
                                    inline_exprs.push(Expression::BinaryOp(
                                        akar_parser::ast::BinaryOp::Equal,
                                        Box::new(Expression::PropertyAccess(
                                            Box::new(Expression::Variable(node_var.clone())),
                                            key.clone(),
                                        )),
                                        Box::new(val_expr.clone()),
                                    ));
                                }
                            }
                            if let Some(edge) = &pattern.edge
                                && let Some(edge_var) = &edge.variable
                            {
                                for (key, val_expr) in &edge.properties {
                                    inline_exprs.push(Expression::BinaryOp(
                                        akar_parser::ast::BinaryOp::Equal,
                                        Box::new(Expression::PropertyAccess(
                                            Box::new(Expression::Variable(edge_var.clone())),
                                            key.clone(),
                                        )),
                                        Box::new(val_expr.clone()),
                                    ));
                                }
                            }
                        }
                        if !inline_exprs.is_empty() {
                            let combined = inline_exprs.into_iter().reduce(|acc, e| {
                                Expression::BinaryOp(akar_parser::ast::BinaryOp::And, Box::new(acc), Box::new(e))
                            });
                            right_ops.push(LogicalOperator::Filter(LogicalFilter {
                                expression: combined.unwrap(),
                                children: Vec::new(),
                                cardinality: 0,
                            }));
                        }
                        let right_op = if right_ops.len() == 1 {
                            right_ops.into_iter().next().unwrap()
                        } else if right_ops.is_empty() {
                            LogicalOperator::ScanNode(LogicalScanNode {
                                table_name: String::new(),
                                table_id: 0,
                                alias: None,
                                columns: Vec::new(),
                                cardinality: 0,
                                fts_query: None,
                                predicate: None,
                            })
                        } else {
                            LogicalOperator::Projection(LogicalProjection {
                                expressions: Vec::new(),
                                children: right_ops,
                                cardinality: 0,
                            })
                        };

                        // Create the OptionalMatch tree node.
                        // The left side is the entire pipeline built so far (scans + filter + projection).
                        // The right side is the optional pattern scans.
                        // Push to delete_exprs so it gets appended at the end of the pipeline.
                        delete_exprs.push(LogicalOperator::OptionalMatch(LogicalOptionalMatch {
                            left: Box::new(left_op),
                            right: Box::new(right_op),
                            cardinality: 0,
                        }));
                    }
                    // Reset pipeline state — subsequent clauses (DELETE, SET, etc.) build fresh
                    scan_ops = Vec::new();
                    filter_expr = None;
                    projection = None;
                    distinct = false;
                }
                BoundClause::BoundDelete(d) => {
                    for item in &d.items {
                        delete_exprs.push(LogicalOperator::Delete(LogicalDelete {
                            table_name: item.table_name.clone(),
                            table_id: item.table_id,
                            primary_key_column: item.primary_key_column.clone(),
                            is_node: item.is_node,
                            detach: d.detach,
                            cardinality: 0,
                        }));
                    }
                }
                BoundClause::BoundUnwind(u) => {
                    // Treat UNWIND as a scan operator: it produces the row
                    // variable that later MATCH clauses join against. Routing it
                    // through the scan list (instead of delete_exprs) lets the
                    // join tree combine UNWIND rows with node scans, and keeps
                    // implicit WHERE predicates from MATCH inline properties in
                    // the scan filter instead of the top-level pipeline (P53.25).
                    scan_ops.push(LogicalOperator::Unwind(LogicalUnwind {
                        expression: u.expression.clone(),
                        variable: u.variable.clone(),
                        cardinality: 0,
                    }));
                    if !u.variable.is_empty() {
                        available_vars.insert(u.variable.clone());
                    }
                }
                BoundClause::BoundSet(s) => {
                    // Merge every item of a single SET clause into one operator
                    // (grouped by target table). All items then evaluate against
                    // the same pre-update snapshot (P53.17) — chaining one
                    // operator per item would feed items 2+ the previous item's
                    // count chunk, losing the scan rows (`SET a=123.0, b=b+1`
                    // left `b` at its old value).
                    let mut groups: Vec<(String, u64, bool, Vec<SetItem>)> = Vec::new();
                    for item in &s.items {
                        let key = (item.table_name.clone(), item.table_id, item.is_node);
                        match groups
                            .iter_mut()
                            .find(|(n, id, n2, _)| *n == key.0 && *id == key.1 && *n2 == key.2)
                        {
                            Some((_, _, _, items)) => items.push(SetItem {
                                column_name: item.column_name.clone(),
                                column_idx: item.column_idx,
                                value: item.value.clone(),
                            }),
                            None => groups.push((
                                key.0,
                                key.1,
                                key.2,
                                vec![SetItem {
                                    column_name: item.column_name.clone(),
                                    column_idx: item.column_idx,
                                    value: item.value.clone(),
                                }],
                            )),
                        }
                    }
                    for (table_name, table_id, is_node, items) in groups {
                        delete_exprs.push(LogicalOperator::Set(LogicalSet {
                            table_name,
                            table_id,
                            is_node,
                            items,
                            cardinality: 0,
                        }));
                    }
                }
                BoundClause::BoundCreate(c) => {
                    let mut patterns_iter = c.patterns.into_iter().peekable();
                    while let Some(pattern) = patterns_iter.next() {
                        let node_var = pattern.node_variable.clone().unwrap_or_default();

                        if c.new_variables.iter().any(|v| v.name == node_var) {
                            delete_exprs.push(LogicalOperator::CreateNode(LogicalCreateNode {
                                table_name: pattern.node_label.clone().unwrap_or_default(),
                                table_id: pattern.node_table_id.unwrap_or(0),
                                out_var_name: node_var.clone(),
                                properties: pattern.properties.clone(),
                                cardinality: 0,
                            }));
                        }

                        if let Some(edge) = pattern.edge {
                            let dest_var = patterns_iter
                                .peek()
                                .and_then(|p| p.node_variable.clone())
                                .unwrap_or_default();
                            let (src_node_name, dst_node_name) = match edge.direction {
                                akar_parser::ast::EdgeDirection::RightToLeft => (dest_var, node_var.clone()),
                                _ => (node_var.clone(), dest_var),
                            };

                            delete_exprs.push(LogicalOperator::CreateRel(LogicalCreateRel {
                                table_name: edge.label.clone().unwrap_or_default(),
                                table_id: edge.rel_table_id.unwrap_or(0),
                                src_node_name,
                                dst_node_name,
                                properties: edge.properties.clone(),
                                cardinality: 0,
                            }));
                        }
                    }
                }
                BoundClause::BoundForeach(f) => {
                    // Plan FOREACH sub-statements
                    let mut sub_plans = Vec::new();
                    for sub_stmt in &f.sub_statements {
                        let plan = self.plan(sub_stmt.clone())?;
                        sub_plans.push(plan);
                    }
                    delete_exprs.push(LogicalOperator::Foreach(LogicalForeach {
                        variable: f.variable.clone(),
                        expression: f.expression.clone(),
                        sub_plans,
                        cardinality: 0,
                    }));
                }
                BoundClause::BoundMerge(m) => {
                    delete_exprs.extend(self.build_merge_operators(&m));
                }
            }
        }

        // Collect delete/set clauses (added after the main pipeline)
        let delete_ops: Vec<LogicalOperator> = std::mem::take(&mut delete_exprs);

        // Build operator pipeline bottom-up
        let mut result: Vec<LogicalOperator> = Vec::new();

        if scan_ops.is_empty() {
            result.extend(delete_ops);
            if let Some(proj) = projection {
                result.push(LogicalOperator::Projection(proj));
            }
            return Ok(result);
        }

        if scan_ops.len() == 1 {
            // Single scan — no join needed
            result.push(scan_ops.into_iter().next().unwrap());
        } else {
            // Multiple scans — build join tree with greedy ordering
            let join_plan = build_join_tree(scan_ops, filter_expr.as_ref());
            let flattened = flatten_join_plan(&join_plan);
            result.extend(flattened);
        }

        // Append extend operators (replace ScanRel in pipeline)
        result.extend(std::mem::take(&mut extend_ops));

        // Apply filter on top of scans/joins/extends
        if let Some(expr) = filter_expr {
            result.push(LogicalOperator::Filter(LogicalFilter {
                expression: expr.expression,
                children: Vec::new(),
                cardinality: 0,
            }));
        }

        // Append update/delete/set/create/unwind/with-projection operators.
        // These run before the RETURN projection so that updates are visible to
        // the returned expressions (P53.14): `MATCH ... SET ... RETURN ...`.
        result.extend(delete_ops);

        // Project as topmost
        if let Some(proj) = projection {
            let group_by = if distinct {
                Some(
                    proj.expressions
                        .iter()
                        .map(|be| be.expression.clone())
                        .collect::<Vec<Expression>>(),
                )
            } else {
                None
            };

            // When ORDER BY references a column the projection does not output
            // (e.g. `RETURN m.id, m.label ORDER BY m.access_count`), the sort key
            // cannot be evaluated against the projected (pruned) chunk. Push the
            // sort below the projection so it runs against the full pre-projection
            // columns (P53.37). It stays on top when every key is covered by the
            // projection output (alias or identical expression), or when DISTINCT
            // deduplicates above (sorting before dedup would lose the order).
            let order_by_below_projection = match &order_by {
                Some(items) => {
                    group_by.is_none()
                        && !items
                            .iter()
                            .all(|item| projection_covers_sort_key(&proj.expressions, &item.expression.expression))
                }
                None => false,
            };

            if order_by_below_projection {
                if let Some(items) = order_by.take() {
                    let sort_keys: Vec<(Expression, bool)> = items
                        .iter()
                        .map(|item| (item.expression.expression.clone(), item.ascending))
                        .collect();
                    result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                        sort_keys,
                        children: Vec::new(),
                        cardinality: 0,
                    }));
                }
            }

            result.push(LogicalOperator::Projection(proj));
            // DISTINCT is implemented as a hash aggregate with group-by keys and no aggregate functions
            if let Some(gb) = group_by {
                result.push(LogicalOperator::Aggregate(LogicalAggregate {
                    group_by: gb,
                    aggregates: Vec::new(),
                    children: Vec::new(),
                    cardinality: 0,
                }));
            }
        }

        // Insert ORDER BY operator if present
        if let Some(items) = order_by {
            let sort_keys: Vec<(Expression, bool)> = items
                .iter()
                .map(|item| (item.expression.expression.clone(), item.ascending))
                .collect();
            result.push(LogicalOperator::OrderBy(LogicalOrderBy {
                sort_keys,
                children: Vec::new(),
                cardinality: 0,
            }));
        }

        // Insert LIMIT/SKIP operator if present
        if limit.is_some() || skip.is_some() {
            result.push(LogicalOperator::Limit(LogicalLimit {
                limit: limit.unwrap_or(u64::MAX),
                offset: skip.unwrap_or(0),
                children: Vec::new(),
                cardinality: 0,
            }));
        }

        Ok(result)
    }
}

impl Default for QueryPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akar_binder::Binder;
    use akar_catalog::{Catalog, CatalogColumn};
    use akar_common::types::LogicalTypeID;
    use akar_parser::parse;
    use std::sync::Arc;

    fn setup_binder() -> Binder {
        let mut catalog = Catalog::new();
        catalog.create_node_table(
            "Person".into(),
            vec![
                CatalogColumn {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                    default_value: None,
                },
                CatalogColumn {
                    compression: akar_common::enums::CompressionType::Uncompressed,
                    name: "age".into(),
                    logical_type: LogicalTypeID::Int64,
                    is_primary_key: false,
                    default_value: None,
                },
            ],
        );
        catalog.create_rel_table(
            "Knows".into(),
            0,
            0,
            vec![CatalogColumn {
                compression: akar_common::enums::CompressionType::Uncompressed,
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
        );
        Binder::new(Arc::new(std::sync::Mutex::new(catalog)))
    }

    #[test]
    fn test_plan_match_return() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.name";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();
        assert!(!plan.is_empty());

        // Should have ScanNode + Projection
        let scan_count = plan
            .iter()
            .filter(|op| matches!(op, LogicalOperator::ScanNode(_)))
            .count();
        let proj_count = plan
            .iter()
            .filter(|op| matches!(op, LogicalOperator::Projection(_)))
            .count();
        assert_eq!(scan_count, 1);
        assert_eq!(proj_count, 1);
    }

    #[test]
    fn test_plan_return_only_projection() {
        let binder = setup_binder();
        let sql = "RETURN 1";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();

        assert_eq!(plan.len(), 1);
        assert!(matches!(plan[0], LogicalOperator::Projection(_)));
    }

    #[test]
    fn test_plan_unwind_then_projection_without_scan() {
        let binder = setup_binder();
        let sql = "UNWIND [1, 2, 3] AS x RETURN x";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();

        assert_eq!(plan.len(), 2);
        assert!(matches!(plan[0], LogicalOperator::Unwind(_)));
        assert!(matches!(plan[1], LogicalOperator::Projection(_)));
    }

    #[test]
    fn test_plan_match_where_return() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();
        assert!(!plan.is_empty());

        let scan_count = plan
            .iter()
            .filter(|op| matches!(op, LogicalOperator::ScanNode(_)))
            .count();
        let filter_count = plan
            .iter()
            .filter(|op| matches!(op, LogicalOperator::Filter(_)))
            .count();
        let proj_count = plan
            .iter()
            .filter(|op| matches!(op, LogicalOperator::Projection(_)))
            .count();
        assert_eq!(scan_count, 1);
        assert_eq!(filter_count, 1);
        assert_eq!(proj_count, 1);
    }

    #[test]
    fn test_plan_ddl_empty() {
        let binder = Binder::new(Arc::new(std::sync::Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();
        assert!(!plan.is_empty()); // DDL now produces a logical plan
        match &plan[0] {
            LogicalOperator::CreateNodeTable(ct) => {
                assert_eq!(ct.name, "City");
            }
            _ => panic!("Expected CreateNodeTable"),
        }
    }

    #[test]
    fn test_plan_scan_node_fields() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();

        match &plan[0] {
            LogicalOperator::ScanNode(s) => {
                assert_eq!(s.table_name, "Person");
                assert_eq!(s.alias, Some("a".into()));
            }
            _ => panic!("Expected ScanNode"),
        }
    }

    #[test]
    fn test_plan_rel_pattern() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a, b";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();
        // Should have an Extend operator replacing the relationship scan + join
        assert!(plan.iter().any(|op| matches!(op, LogicalOperator::Extend(_))));
        // Should also have ScanNode for the source node
        assert!(plan.iter().any(|op| matches!(op, LogicalOperator::ScanNode(_))));
        // Should NOT have ScanRel (replaced by Extend)
        assert!(!plan.iter().any(|op| matches!(op, LogicalOperator::ScanRel(_))));
    }

    #[test]
    fn test_plan_order() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();

        // Order should be: scans → filter → projection
        let positions: Vec<&str> = plan
            .iter()
            .map(|op| match op {
                LogicalOperator::ScanNode(_) => "scan",
                LogicalOperator::Filter(_) => "filter",
                LogicalOperator::Projection(_) => "proj",
                _ => "other",
            })
            .collect();

        let scan_pos = positions.iter().position(|&p| p == "scan").unwrap();
        let filter_pos = positions.iter().position(|&p| p == "filter").unwrap();
        let proj_pos = positions.iter().position(|&p| p == "proj").unwrap();

        assert!(scan_pos < filter_pos);
        assert!(filter_pos < proj_pos);
    }
}
