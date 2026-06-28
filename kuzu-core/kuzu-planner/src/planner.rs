//! Query planner — converts bound statements into logical query plans.
//!
//! Builds a tree of logical operators from the bound AST:
//! - MATCH → ScanNode / ScanRel
//! - WHERE → Filter (child of scans)
//! - RETURN → Projection (topmost operator)

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
            _ => Ok(Vec::new()),
        }
    }

    fn plan_query(&self, query: BoundQuery) -> Result<Vec<LogicalOperator>, String> {
        let mut scan_ops: Vec<LogicalOperator> = Vec::new();
        let mut filter_expr: Option<BoundExpression> = None;
        let mut projection: Option<LogicalProjection> = None;

        for clause in query.clauses {
            match clause {
                BoundClause::BoundMatch(m) => {
                    for pattern in m.patterns {
                        // Scan node table
                        if let Some(label) = pattern.node_label {
                            scan_ops.push(LogicalOperator::ScanNode(LogicalScanNode {
                                table_name: label,
                                table_id: pattern.node_table_id.unwrap_or(0),
                                alias: pattern.node_variable,
                                columns: Vec::new(),
                            }));
                        }
                        // Scan rel table
                        if let Some(edge) = pattern.edge {
                            if let Some(rel_label) = edge.label {
                                scan_ops.push(LogicalOperator::ScanRel(LogicalScanRel {
                                    table_name: rel_label,
                                    table_id: edge.rel_table_id.unwrap_or(0),
                                    direction: edge.direction,
                                }));
                            }
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
                    });
                }
            }
        }

        // Build operator tree bottom-up
        // If multiple scans, combine with CrossProduct (simplified)
        let mut result: Vec<LogicalOperator> = Vec::new();

        // Add scans
        for scan in scan_ops {
            result.push(scan);
        }

        // Apply filter on top of scans
        if let Some(expr) = filter_expr {
            result.push(LogicalOperator::Filter(LogicalFilter {
                expression: expr.expression,
                children: Vec::new(),
            }));
        }

        // Project as topmost
        if let Some(proj) = projection {
            result.push(LogicalOperator::Projection(proj));
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
    use kuzu_binder::Binder;
    use kuzu_catalog::{Catalog, CatalogColumn};
    use kuzu_common::types::LogicalTypeID;
    use kuzu_parser::parse;

    fn setup_binder() -> Binder {
        let mut catalog = Catalog::new();
        catalog.create_node_table(
            "Person".into(),
            vec![
                CatalogColumn { name: "name".into(), logical_type: LogicalTypeID::String, is_primary_key: true, default_value: None },
                CatalogColumn { name: "age".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false, default_value: None },
            ],
        );
        catalog.create_rel_table(
            "Knows".into(), 0, 0,
            vec![CatalogColumn { name: "since".into(), logical_type: LogicalTypeID::Int64, is_primary_key: false, default_value: None }],
        );
        Binder::new(catalog)
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
        let scan_count = plan.iter().filter(|op| matches!(op, LogicalOperator::ScanNode(_))).count();
        let proj_count = plan.iter().filter(|op| matches!(op, LogicalOperator::Projection(_))).count();
        assert_eq!(scan_count, 1);
        assert_eq!(proj_count, 1);
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

        let scan_count = plan.iter().filter(|op| matches!(op, LogicalOperator::ScanNode(_))).count();
        let filter_count = plan.iter().filter(|op| matches!(op, LogicalOperator::Filter(_))).count();
        let proj_count = plan.iter().filter(|op| matches!(op, LogicalOperator::Projection(_))).count();
        assert_eq!(scan_count, 1);
        assert_eq!(filter_count, 1);
        assert_eq!(proj_count, 1);
    }

    #[test]
    fn test_plan_ddl_empty() {
        let binder = Binder::new(Catalog::new());
        let sql = "CREATE NODE TABLE City(name STRING, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        let planner = QueryPlanner::new();
        let plan = planner.plan(bound).unwrap();
        assert!(plan.is_empty()); // DDL produces no logical plan
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
        let positions: Vec<&str> = plan.iter().map(|op| match op {
            LogicalOperator::ScanNode(_) => "scan",
            LogicalOperator::Filter(_) => "filter",
            LogicalOperator::Projection(_) => "proj",
            _ => "other",
        }).collect();

        let scan_pos = positions.iter().position(|&p| p == "scan").unwrap();
        let filter_pos = positions.iter().position(|&p| p == "filter").unwrap();
        let proj_pos = positions.iter().position(|&p| p == "proj").unwrap();

        assert!(scan_pos < filter_pos);
        assert!(filter_pos < proj_pos);
    }
}
