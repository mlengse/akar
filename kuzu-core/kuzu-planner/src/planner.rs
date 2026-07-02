//! Query planner — converts bound statements into logical query plans.
//!
//! Builds a tree of logical operators from the bound AST:
//! - MATCH → ScanNode / ScanRel
//! - Multiple MATCH patterns combined via join tree (HashJoin / CrossProduct)
//! - WHERE → Filter (applied after joins)
//! - RETURN → Projection (topmost operator)

use crate::join_order::{build_join_tree, flatten_join_plan};
use crate::logical_operator::*;
use kuzu_binder::bound_statement::*;

/// The query planner transforms bound statements into logical query plans.
pub struct QueryPlanner;

impl QueryPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(&self, statement: BoundStatement) -> Result<Vec<LogicalOperator>, String> {
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
            _ => Ok(Vec::new()),
        }
    }

    /// Plan an EXPLAIN statement.
    ///
    /// Plans the inner statement first, then wraps the result in a
    /// LogicalExplain operator that will serialize the plan tree to text.
    fn plan_explain(&self, e: BoundExplain) -> Result<Vec<LogicalOperator>, String> {
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

    fn plan_copy_from(&self, c: BoundCopyFrom) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CopyFrom(LogicalCopyFrom {
            table_name: c.table_name,
            table_id: c.table_id,
            file_path: c.file_path,
            options: c.options,
            cardinality: 0,
        })])
    }

    // ==================== DDL Planning ====================

    fn plan_create_node_table(&self, t: BoundCreateNodeTable) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CreateNodeTable(LogicalCreateNodeTable {
            name: t.name,
            columns: t.columns,
            primary_key: t.primary_key,
            cardinality: 1,
        })])
    }

    fn plan_create_rel_table(&self, t: BoundCreateRelTable) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CreateRelTable(LogicalCreateRelTable {
            name: t.name,
            from: t.from,
            to: t.to,
            columns: t.columns,
            cardinality: 1,
        })])
    }

    fn plan_drop_table(&self, t: BoundDropTable) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::DropTable(LogicalDropTable {
            name: t.name,
            cardinality: 1,
        })])
    }

    fn plan_alter_table(&self, a: BoundAlterTable) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::AlterTable(LogicalAlterTable {
            table_name: a.table_name,
            action: a.action,
            cardinality: 1,
        })])
    }

    fn plan_create_index(&self, idx: BoundCreateIndex) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CreateIndex(LogicalCreateIndex {
            index_type: idx.index_type,
            index_name: idx.index_name,
            table_name: idx.table_name,
            column_name: idx.column_name,
            cardinality: 1,
        })])
    }

    fn plan_drop_index(&self, idx: BoundDropIndex) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::DropIndex(LogicalDropIndex {
            index_name: idx.index_name,
            table_name: idx.table_name,
            cardinality: 1,
        })])
    }

    fn plan_create_vector_index(&self, idx: BoundCreateVectorIndex) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CreateVectorIndex(LogicalCreateVectorIndex {
            index_name: idx.index_name,
            table_name: idx.table_name,
            column_name: idx.column_name,
            metric: idx.metric,
            dimensions: idx.dimensions,
            cardinality: 1,
        })])
    }

    fn plan_create_sequence(&self, s: BoundCreateSequence) -> Result<Vec<LogicalOperator>, String> {
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

    fn plan_drop_sequence(&self, s: BoundDropSequence) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::DropSequence(LogicalDropSequence {
            name: s.name,
            if_exists: s.if_exists,
            cardinality: 1,
        })])
    }

    fn plan_create_dml(&self, c: BoundCreateDml) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::CreateDml(LogicalCreateDml {
            table_name: c.table_name,
            table_id: c.table_id,
            properties: c.properties,
            cardinality: 1,
        })])
    }

    fn plan_export_database(&self, e: BoundExportDatabase) -> Result<Vec<LogicalOperator>, String> {
        Ok(vec![LogicalOperator::ExportDatabase(LogicalExportDatabase {
            file_path: e.file_path,
            file_type: e.file_type,
            schema_only: e.schema_only,
            options: e.options,
            cardinality: 1,
        })])
    }

    fn plan_import_database(&self, i: BoundImportDatabase) -> Result<Vec<LogicalOperator>, String> {
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
    fn plan_merge(&self, m: BoundMerge) -> Result<Vec<LogicalOperator>, String> {
        let on_match: Vec<LogicalSet> = m
            .on_match
            .iter()
            .map(|item| LogicalSet {
                table_name: item.table_name.clone(),
                table_id: item.table_id,
                column_name: item.column_name.clone(),
                column_idx: item.column_idx,
                value: item.value.clone(),
                cardinality: 0,
            })
            .collect();

        let on_create: Vec<LogicalSet> = m
            .on_create
            .iter()
            .map(|item| LogicalSet {
                table_name: item.table_name.clone(),
                table_id: item.table_id,
                column_name: item.column_name.clone(),
                column_idx: item.column_idx,
                value: item.value.clone(),
                cardinality: 0,
            })
            .collect();

        Ok(vec![LogicalOperator::Merge(LogicalMerge {
            table_name: m.table_name,
            table_id: m.table_id,
            properties: m.properties,
            on_match,
            on_create,
            cardinality: 0,
        })])
    }

    /// Plan a UNION or UNION ALL statement.
    ///
    /// Plans left and right sub-queries independently, then wraps each
    /// side's pipeline (potentially multiple operators) into a synthetic
    /// projection root so that `LogicalUnion` can store them as tree children.
    fn plan_union(&self, u: BoundUnion) -> Result<Vec<LogicalOperator>, String> {
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

    fn plan_query(&self, query: BoundQuery) -> Result<Vec<LogicalOperator>, String> {
        let mut scan_ops: Vec<LogicalOperator> = Vec::new();
        let mut filter_expr: Option<BoundExpression> = None;
        let mut projection: Option<LogicalProjection> = None;
        let mut delete_exprs: Vec<LogicalOperator> = Vec::new();
        // Flag to skip destination node pattern consumed by RecursiveExtend
        let mut skip_next_node = false;

        for clause in query.clauses {
            match clause {
                BoundClause::BoundMatch(m) => {
                    let mut patterns_iter = m.patterns.into_iter().peekable();
                    while let Some(pattern) = patterns_iter.next() {
                        // If the previous pattern's RecursiveExtend consumed this dest node, skip
                        if skip_next_node {
                            skip_next_node = false;
                            continue;
                        }

                        // Check if this pattern has a var-length edge → create RecursiveExtend
                        if let Some(ref edge) = pattern.edge {
                            let is_var_length = edge.lower_bound.is_some() || edge.upper_bound.is_some();
                            if is_var_length {
                                let lb = edge.lower_bound.unwrap_or(0);
                                let ub = edge.upper_bound.unwrap_or(1);
                                let direction = match edge.direction {
                                    kuzu_parser::ast::EdgeDirection::LeftToRight =>
                                        kuzu_common::enums::ExtendDirection::Fwd,
                                    kuzu_parser::ast::EdgeDirection::RightToLeft =>
                                        kuzu_common::enums::ExtendDirection::Bwd,
                                    kuzu_parser::ast::EdgeDirection::Both =>
                                        kuzu_common::enums::ExtendDirection::Both,
                                };

                                // Scan source node
                                let node_var = &pattern.node_variable;
                                if let Some(label) = pattern.node_label {
                                    scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                        table_name: label,
                                        table_id: pattern.node_table_id.unwrap_or(0),
                                        alias: node_var.clone(),
                                        columns: Vec::new(),
                                        cardinality: 0,
                                    }));
                                }

                                // Create RecursiveExtend (consumes destination node pattern)
                                let rel_table_ids = edge.rel_table_id.map_or(vec![], |id| vec![id]);
                                let rel_labels = edge.label.as_ref().map_or(vec![], |l| vec![l.clone()]);
                                let target_var = patterns_iter.peek()
                                    .and_then(|p| p.node_variable.clone())
                                    .unwrap_or_default();
                                scan_ops.push(LogicalOperator::RecursiveExtend(LogicalRecursiveExtend {
                                    source_var: node_var.clone().unwrap_or_default(),
                                    source_table_id: pattern.node_table_id.unwrap_or(0),
                                    edge_var: edge.variable.clone(),
                                    target_var,
                                    rel_table_ids,
                                    rel_labels,
                                    lower_bound: lb,
                                    upper_bound: ub,
                                    direction,
                                    semantic: kuzu_common::enums::PathSemantic::Walk,
                                    weight_property: None,
                                    cost_output_name: None,
                                    cardinality: 0,
                                }));
                                // Skip the destination node pattern (consumed by RecursiveExtend)
                                skip_next_node = true;
                                continue;
                            }
                        }

                        // Regular (non-var-length) pattern: Scan node
                        if let Some(label) = pattern.node_label {
                            scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                table_name: label,
                                table_id: pattern.node_table_id.unwrap_or(0),
                                alias: pattern.node_variable,
                                columns: Vec::new(),
                                cardinality: 0,
                            }));
                        }
                        // Regular edge: ScanRel
                        if let Some(edge) = pattern.edge
                            && let Some(rel_label) = edge.label {
                                scan_ops.push(LogicalOperator::ScanRel(LogicalScanRel {
                                    table_name: rel_label,
                                    table_id: edge.rel_table_id.unwrap_or(0),
                                    direction: edge.direction,
                                    cardinality: 0,
                                }));
                            }
                    }
                }
                BoundClause::BoundWhere(w) => {
                    filter_expr = Some(w.expression);
                }
                BoundClause::BoundReturn(r) => {
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
                            }));
                        }
                        if let Some(edge) = &pattern.edge
                            && let Some(rel_label) = &edge.label {
                                right_ops.push(LogicalOperator::ScanRel(LogicalScanRel {
                                    table_name: rel_label.clone(),
                                    table_id: edge.rel_table_id.unwrap_or(0),
                                    direction: edge.direction.clone(),
                                    cardinality: 0,
                                }));
                            }
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
                    // Reset pipeline state — subsequent clauses (DELETE, SET, etc.) build fresh
                    scan_ops = Vec::new();
                    filter_expr = None;
                    projection = None;
                }
                BoundClause::BoundDelete(d) => {
                    delete_exprs.push(LogicalOperator::Delete(LogicalDelete {
                        table_name: d.table_name.clone(),
                        table_id: d.table_id,
                        primary_key_column: d.primary_key_column.clone(),
                        cardinality: 0,
                    }));
                }
                BoundClause::BoundUnwind(u) => {
                    delete_exprs.push(LogicalOperator::Unwind(LogicalUnwind {
                        expression: u.expression.clone(),
                        variable: u.variable.clone(),
                        cardinality: 0,
                    }));
                }
                BoundClause::BoundSet(s) => {
                    for item in &s.items {
                        delete_exprs.push(LogicalOperator::Set(LogicalSet {
                            table_name: item.table_name.clone(),
                            table_id: item.table_id,
                            column_name: item.column_name.clone(),
                            column_idx: item.column_idx,
                            value: item.value.clone(),
                            cardinality: 0,
                        }));
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

        // Apply filter on top of scans/joins
        if let Some(expr) = filter_expr {
            result.push(LogicalOperator::Filter(LogicalFilter {
                expression: expr.expression,
                children: Vec::new(),
                cardinality: 0,
            }));
        }

        // Project as topmost
        if let Some(proj) = projection {
            result.push(LogicalOperator::Projection(proj));
        }

        // Append DELETE operators at the end
        result.extend(delete_ops);

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
    use kuzu_binder::Binder;
    use kuzu_catalog::{Catalog, CatalogColumn};
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::parse;
    use std::sync::Arc;

    fn setup_binder() -> Binder {
        let mut catalog = Catalog::new();
        catalog.create_node_table(
            "Person".into(),
            vec![
                CatalogColumn {
                    name: "name".into(),
                    logical_type: LogicalTypeID::String,
                    is_primary_key: true,
                    default_value: None,
                },
                CatalogColumn {
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
        // Should have at least a ScanNode
        assert!(plan.iter().any(|op| matches!(op, LogicalOperator::ScanNode(_))));
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
