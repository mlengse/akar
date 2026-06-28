//! Query planner — converts bound statements into a logical plan.

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
            _ => Ok(Vec::new()), // DDL statements don't produce a logical plan
        }
    }

    fn plan_query(&self, query: BoundQuery) -> Result<Vec<LogicalOperator>, String> {
        let mut operators = Vec::new();

        for clause in query.clauses {
            match clause {
                BoundClause::BoundMatch(m) => {
                    for pattern in m.patterns {
                        if let Some(label) = pattern.node_label {
                            operators.push(LogicalOperator::ScanNode(LogicalScanNode {
                                table_name: label.clone(),
                                table_id: 0,
                                alias: pattern.node_variable,
                                columns: Vec::new(),
                            }));
                        }
                    }
                }
                BoundClause::BoundWhere(_) => {
                    // TODO: implement filter planning
                }
                BoundClause::BoundReturn(r) => {
                    operators.push(LogicalOperator::Projection(LogicalProjection {
                        expressions: r.expressions,
                        children: Vec::new(),
                    }));
                }
                _ => {}
            }
        }

        Ok(operators)
    }
}

impl Default for QueryPlanner {
    fn default() -> Self {
        Self::new()
    }
}
