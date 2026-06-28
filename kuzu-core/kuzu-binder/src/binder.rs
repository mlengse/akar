//! Binder implementation — resolves symbols and validates semantics.

use crate::bound_statement::*;
use kuzu_catalog::{Catalog, CatalogEntry, NodeTableEntry, RelTableEntry};
use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::{Statement, *};

/// The binder transforms a parsed AST into a bound statement
/// by resolving symbols against the catalog and validating types.
#[allow(dead_code)]
pub struct Binder {
    catalog: Catalog,
}

impl Binder {
    pub fn new(catalog: Catalog) -> Self {
        Self { catalog }
    }

    pub fn bind(&self, statement: Statement) -> Result<BoundStatement, String> {
        match statement {
            Statement::Query(query) => self.bind_query(query),
            Statement::CreateNodeTable(t) => self.bind_create_node_table(t),
            Statement::CreateRelTable(t) => self.bind_create_rel_table(t),
            Statement::DropTable(t) => self.bind_drop_table(t),
        }
    }

    fn bind_query(&self, query: Query) -> Result<BoundStatement, String> {
        let mut clauses = Vec::new();
        for clause in query.clauses {
            let bound = match clause {
                Clause::Match(m) => {
                    BoundClause::BoundMatch(self.bind_match(m)?)
                }
                Clause::Return(r) => {
                    BoundClause::BoundReturn(self.bind_return(r)?)
                }
                Clause::Where(w) => {
                    BoundClause::BoundWhere(self.bind_where(w)?)
                }
                Clause::Create(c) => {
                    BoundClause::BoundCreate(self.bind_create(c)?)
                }
            };
            clauses.push(bound);
        }
        Ok(BoundStatement::BoundQuery(BoundQuery { clauses }))
    }

    fn bind_match(&self, m: MatchClause) -> Result<BoundMatchClause, String> {
        let mut patterns = Vec::new();
        for pattern in m.patterns {
            let bound = BoundPattern {
                node_variable: pattern.node.as_ref().and_then(|n| n.variable.clone()),
                node_label: pattern.node.as_ref().and_then(|n| n.labels.first().cloned()),
                edge: pattern.edge.map(|e| BoundEdgePattern {
                    variable: e.variable,
                    label: e.labels.first().cloned(),
                    direction: e.direction,
                }),
            };
            patterns.push(bound);
        }
        Ok(BoundMatchClause { patterns })
    }

    fn bind_return(&self, r: ReturnClause) -> Result<BoundReturnClause, String> {
        let expressions = r
            .expressions
            .into_iter()
            .map(|item| {
                // TODO: resolve actual type from catalog
                (item.expression, item.alias, LogicalTypeID::Any)
            })
            .collect();
        Ok(BoundReturnClause { expressions })
    }

    fn bind_where(&self, w: WhereClause) -> Result<BoundWhereClause, String> {
        Ok(BoundWhereClause {
            expression: w.expression,
        })
    }

    fn bind_create(&self, c: CreateClause) -> Result<BoundCreateClause, String> {
        let patterns = c
            .patterns
            .into_iter()
            .map(|p| BoundPattern {
                node_variable: p.node.as_ref().and_then(|n| n.variable.clone()),
                node_label: p.node.as_ref().and_then(|n| n.labels.first().cloned()),
                edge: p.edge.map(|e| BoundEdgePattern {
                    variable: e.variable,
                    label: e.labels.first().cloned(),
                    direction: e.direction,
                }),
            })
            .collect();
        Ok(BoundCreateClause { patterns })
    }

    fn bind_create_node_table(
        &self,
        t: CreateNodeTable,
    ) -> Result<BoundStatement, String> {
        let entry = CatalogEntry::NodeTable(NodeTableEntry {
            table_id: 0,
            name: t.name.clone(),
            columns: t
                .columns
                .into_iter()
                .map(|c| kuzu_catalog::CatalogColumn {
                    name: c.name,
                    logical_type: LogicalTypeID::Any,
                    is_primary_key: false,
                    default_value: None,
                })
                .collect(),
            primary_key_column: 0,
        });
        Ok(BoundStatement::BoundCreateNodeTable(BoundCreateNodeTable {
            name: t.name,
            catalog_entry: entry,
        }))
    }

    fn bind_create_rel_table(
        &self,
        t: CreateRelTable,
    ) -> Result<BoundStatement, String> {
        let entry = CatalogEntry::RelTable(RelTableEntry {
            table_id: 0,
            name: t.name.clone(),
            src_table_id: 0,
            dst_table_id: 0,
            columns: t
                .columns
                .into_iter()
                .map(|c| kuzu_catalog::CatalogColumn {
                    name: c.name,
                    logical_type: LogicalTypeID::Any,
                    is_primary_key: false,
                    default_value: None,
                })
                .collect(),
        });
        Ok(BoundStatement::BoundCreateRelTable(BoundCreateRelTable {
            name: t.name,
            catalog_entry: entry,
        }))
    }

    fn bind_drop_table(&self, t: DropTable) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundDropTable(BoundDropTable {
            name: t.name,
        }))
    }
}
