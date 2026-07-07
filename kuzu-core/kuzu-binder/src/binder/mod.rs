//! Binder implementation ΓÇö resolves symbols and validates semantics.

#![allow(clippy::collapsible_if, clippy::never_loop)]

use crate::bound_statement::*;
use kuzu_catalog::{Catalog, CatalogColumn, CatalogResult, IndexType};
use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::{Clause, Expression, Statement, *};
use std::sync::{Arc, Mutex};

/// Resolve SET clause items against the catalog to find column info.
fn resolve_set_items(catalog: &Catalog, items: &[SetItem]) -> Result<Vec<BoundSetItem>, String> {
    let mut result = Vec::new();
    for item in items {
        // Expect property expression like `n.property_name = value`
        match &item.property {
            Expression::PropertyAccess(obj, prop_name) => {
                // Find the variable name by looking at the object
                let _var_name = match obj.as_ref() {
                    Expression::Variable(v) => v.clone(),
                    other => return Err(format!("Unsupported SET target: {:?}", other)),
                };
                // Look up the column in the table schema
                // We need to find which table this variable belongs to.
                // Since MERGE is a single-pattern operation, we just use the label.
                let found = catalog.all_entries().find_map(|entry| {
                    entry.columns().iter().find(|c| c.name == *prop_name).map(|_| {
                        let is_node = entry.is_node_table();
                        (entry.name().to_string(), entry.table_id(), is_node)
                    })
                });
                match found {
                    Some((table_name, table_id, is_node)) => {
                        let col_idx = catalog
                            .get_entry_by_name(&table_name)
                            .and_then(|e| e.columns().iter().position(|c| c.name == *prop_name))
                            .unwrap_or(0);
                        result.push(BoundSetItem {
                            property: item.property.clone(),
                            value: item.value.clone(),
                            column_name: prop_name.clone(),
                            column_idx: col_idx,
                            table_name: table_name.to_string(),
                            table_id,
                            is_node,
                        });
                    }
                    None => {
                        return Err(format!("Property '{}' not found in any table", prop_name));
                    }
                }
            }
            _ => return Err(format!("Expected property assignment in SET, got: {:?}", item.property)),
        }
    }
    Ok(result)
}

/// The binder transforms a parsed AST into a bound statement
/// by resolving symbols against the catalog and validating types.
pub struct Binder {
    catalog: Arc<Mutex<Catalog>>,
}

impl Binder {
    pub fn new(catalog: Arc<Mutex<Catalog>>) -> Self {
        Self { catalog }
    }

    pub fn bind(&self, statement: Statement) -> Result<BoundStatement, String> {
        match statement {
            Statement::Query(query) => self.bind_query(query),
            Statement::CreateNodeTable(t) => self.bind_create_node_table(t),
            Statement::CreateRelTable(t) => self.bind_create_rel_table(t),
            Statement::DropTable(t) => self.bind_drop_table(t),
            Statement::CopyFrom(c) => self.bind_copy_from(c),
            Statement::CopyTo(c) => self.bind_copy_to(c),
            Statement::AlterTable(a) => self.bind_alter_table(a),
            Statement::CreateVectorIndex(v) => self.bind_create_vector_index(v),
            Statement::CreateIndex(v) => self.bind_create_index(v),
            Statement::DropIndex(v) => self.bind_drop_index(v),
            Statement::Union(u) => self.bind_union(u),
            Statement::Merge(m) => self.bind_merge(m),
            Statement::Call(c) => self.bind_call(c),
            Statement::CreateDml(c) => self.bind_create_dml(c, &[]),
            Statement::Explain(e) => self.bind_explain(e),
            Statement::CreateSequence(s) => self.bind_create_sequence(s),
            Statement::DropSequence(s) => self.bind_drop_sequence(s),
            Statement::CreateMacro(m) => self.bind_create_macro(m),
            Statement::ExportDatabase(e) => self.bind_export_database(e),
            Statement::ImportDatabase(i) => self.bind_import_database(i),
            Statement::Analyze(a) => self.bind_analyze(a),
            Statement::CreateFtsIndex(f) => self.bind_create_fts_index(f),
            Statement::Transaction(t) => self.bind_transaction(t),
            Statement::Extension(e) => self.bind_extension(e),
            Statement::AttachDatabase(a) => self.bind_attach_database(a),
            Statement::DetachDatabase(d) => self.bind_detach_database(d),
            Statement::UseDatabase(u) => self.bind_use_database(u),
            Statement::LoadFrom(l) => self.bind_load_from(l),
        }
    }

    /// Map a string type name to LogicalTypeID.
    pub fn parse_type(type_name: &str) -> Result<LogicalTypeID, String> {
        match type_name.to_uppercase().as_str() {
            "BOOL" | "BOOLEAN" => Ok(LogicalTypeID::Bool),
            "INT64" => Ok(LogicalTypeID::Int64),
            "INT32" => Ok(LogicalTypeID::Int32),
            "INT16" => Ok(LogicalTypeID::Int16),
            "INT8" => Ok(LogicalTypeID::Int8),
            "UINT64" => Ok(LogicalTypeID::UInt64),
            "UINT32" => Ok(LogicalTypeID::UInt32),
            "UINT16" => Ok(LogicalTypeID::UInt16),
            "UINT8" => Ok(LogicalTypeID::UInt8),
            "DOUBLE" => Ok(LogicalTypeID::Double),
            "FLOAT" => Ok(LogicalTypeID::Float),
            "STRING" => Ok(LogicalTypeID::String),
            "BLOB" => Ok(LogicalTypeID::Blob),
            "DATE" => Ok(LogicalTypeID::Date),
            "TIMESTAMP" | "TIMESTAMP_MS" => Ok(LogicalTypeID::Timestamp),
            "TIMESTAMP_SEC" => Ok(LogicalTypeID::TimestampSec),
            "TIMESTAMP_NS" => Ok(LogicalTypeID::TimestampNs),
            "TIMESTAMP_TZ" => Ok(LogicalTypeID::TimestampTz),
            "INTERVAL" => Ok(LogicalTypeID::Interval),
            "SERIAL" => Ok(LogicalTypeID::Serial),
            _ => Err(format!("Unknown type: {type_name}")),
        }
    }

    // ==================== Query Binding ====================

    fn bind_query(&self, query: Query) -> Result<BoundStatement, String> {
        let mut clauses = Vec::new();
        let mut variables: Vec<BoundVariable> = Vec::new();

        for clause in query.clauses {
            let (bound_clause, new_vars) = match clause {
                Clause::Match(m) => {
                    let (bound, vars) = self.bind_match(&m, &variables)?;
                    (BoundClause::BoundMatch(bound), vars)
                }
                Clause::Return(r) => {
                    let bound = self.bind_return(&r, &variables)?;
                    (BoundClause::BoundReturn(bound), Vec::new())
                }
                Clause::With(r) => {
                    let bound = self.bind_return(&r, &variables)?;
                    (BoundClause::BoundWith(bound), Vec::new())
                }
                Clause::Where(w) => {
                    let bound = self.bind_where(&w, &variables)?;
                    (BoundClause::BoundWhere(bound), Vec::new())
                }
                Clause::Create(c) => {
                    let (bound, vars) = self.bind_match_create(&c, &variables)?;
                    (BoundClause::BoundCreate(bound), vars)
                }
                Clause::Delete(d) => {
                    let bound = self.bind_delete(&d, &variables)?;
                    (BoundClause::BoundDelete(bound), Vec::new())
                }
                Clause::Set(s) => {
                    let bound = self.bind_set(&s, &variables)?;
                    (BoundClause::BoundSet(bound), Vec::new())
                }
                Clause::Unwind(u) => {
                    let bound = self.bind_unwind(&u)?;
                    let new_var = BoundVariable {
                        name: bound.variable.clone(),
                        table_id: 0,
                        label: None,
                        is_node: false,
                    };
                    (BoundClause::BoundUnwind(bound), vec![new_var])
                }
                Clause::Foreach(f) => {
                    let bound = self.bind_foreach(&f, &variables)?;
                    let new_var = BoundVariable {
                        name: bound.variable.clone(),
                        table_id: 0,
                        label: None,
                        is_node: false,
                    };
                    (BoundClause::BoundForeach(bound), vec![new_var])
                }
                Clause::OptionalMatch(m) => {
                    let (bound, vars) = self.bind_optional_match(&m, &variables)?;
                    (BoundClause::BoundOptionalMatch(bound), vars)
                }
            };
            variables.extend(new_vars);
            clauses.push(bound_clause.clone());

            // Generate implicit WHERE clauses from inline properties for MATCH and CREATE
            if let BoundClause::BoundMatch(bound) = &bound_clause {
                let mut inline_exprs = Vec::new();
                for pattern in &bound.patterns {
                    if let Some(node_var) = &pattern.node_variable {
                        for (key, val_expr) in &pattern.properties {
                            let prop_access = kuzu_parser::ast::Expression::PropertyAccess(
                                Box::new(kuzu_parser::ast::Expression::Variable(node_var.clone())),
                                key.clone(),
                            );
                            let equals = kuzu_parser::ast::Expression::BinaryOp(
                                kuzu_parser::ast::BinaryOp::Equal,
                                Box::new(prop_access),
                                Box::new(val_expr.clone()),
                            );
                            inline_exprs.push(equals);
                        }
                    }
                    if let Some(edge) = &pattern.edge {
                        if let Some(edge_var) = &edge.variable {
                            for (key, val_expr) in &edge.properties {
                                let prop_access = kuzu_parser::ast::Expression::PropertyAccess(
                                    Box::new(kuzu_parser::ast::Expression::Variable(edge_var.clone())),
                                    key.clone(),
                                );
                                let equals = kuzu_parser::ast::Expression::BinaryOp(
                                    kuzu_parser::ast::BinaryOp::Equal,
                                    Box::new(prop_access),
                                    Box::new(val_expr.clone()),
                                );
                                inline_exprs.push(equals);
                            }
                        }
                    }
                }

                if !inline_exprs.is_empty() {
                    let combined = inline_exprs
                        .into_iter()
                        .reduce(|acc, e| {
                            kuzu_parser::ast::Expression::BinaryOp(
                                kuzu_parser::ast::BinaryOp::And,
                                Box::new(acc),
                                Box::new(e),
                            )
                        })
                        .unwrap();

                    let bound_expr = self.resolve_expression(&combined, &variables)?;
                    clauses.push(BoundClause::BoundWhere(BoundWhereClause { expression: bound_expr }));
                }
            }
        }

        Ok(BoundStatement::BoundQuery(BoundQuery { clauses, variables }))
    }

    // ==================== MATCH Binding ====================

    fn bind_match(
        &self,
        m: &MatchClause,
        existing_vars: &[BoundVariable],
    ) -> Result<(BoundMatchClause, Vec<BoundVariable>), String> {
        let mut patterns = Vec::new();
        let mut new_vars = Vec::new();

        for pattern in &m.patterns {
            let all_vars: Vec<BoundVariable> = existing_vars.iter().cloned().chain(new_vars.iter().cloned()).collect();
            let (bound, nv) = self.bind_pattern(pattern, &all_vars, false)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        // Bind optional FTS query
        let fts_query = m.fts_query.as_ref().map(|fq| BoundFtsQuery {
                index_name: fq.index_name.clone(),
                query_string: fq.query_string.clone(),
                docs_table: format!("fts_{}_docs", fq.index_name),
                terms_table: format!("fts_{}_terms", fq.index_name),
                posting_table: format!("fts_{}_appears_in", fq.index_name),
            });

        Ok((
            BoundMatchClause {
                patterns,
                new_variables: new_vars.clone(),
                fts_query,
            },
            new_vars,
        ))
    }

    fn bind_pattern(
        &self,
        pattern: &Pattern,
        existing_vars: &[BoundVariable],
        allow_existing: bool,
    ) -> Result<(BoundPattern, Vec<BoundVariable>), String> {
        let mut new_vars = Vec::new();
        let mut node_table_id = None;
        let mut bound_edge = None;

        // Resolve node
        let (node_var, node_label) = if let Some(ref n) = pattern.node {
            let var = n.variable.clone();
            let label = n.labels.first().cloned();

            // Look up in catalog
            if let Some(ref lbl) = label {
                let catalog = self.catalog.lock().unwrap();
                match catalog.get_entry_by_name(lbl) {
                    Some(entry) if entry.is_node_table() => {
                        node_table_id = Some(entry.table_id());
                    }
                    Some(_entry) => {
                        return Err(format!("'{}' is not a node table", lbl));
                    }
                    None => {
                        return Err(format!("Table '{}' not found", lbl));
                    }
                }
            }

            // Check for duplicate variable names
            if let Some(ref v) = var {
                if let Some(existing) = existing_vars.iter().find(|bv| bv.name == *v) {
                    if allow_existing {
                        // Reference to already-bound variable (e.g. in CREATE after MATCH)
                        // Use the existing variable's table_id if we didn't resolve one
                        if node_table_id.is_none() {
                            node_table_id = Some(existing.table_id);
                        }
                        // Don't add to new_vars ΓÇö it's a reference, not a new binding
                    } else {
                        return Err(format!("Variable '{}' already defined", v));
                    }
                } else {
                    new_vars.push(BoundVariable {
                        name: var.clone().unwrap_or_else(|| "_anon_".to_string()),
                        table_id: node_table_id.unwrap_or(0),
                        label: label.clone(),
                        is_node: true,
                    });
                }
            } else {
                new_vars.push(BoundVariable {
                    name: "_anon_".to_string(),
                    table_id: node_table_id.unwrap_or(0),
                    label: label.clone(),
                    is_node: true,
                });
            }

            (var, label)
        } else {
            (None, None)
        };

        // Resolve edge
        if let Some(ref e) = pattern.edge {
            let edge_var = e.variable.clone();
            let edge_label = e.labels.first().cloned();
            let mut rel_table_id = None;

            if let Some(ref lbl) = edge_label {
                let catalog = self.catalog.lock().unwrap();
                match catalog.get_entry_by_name(lbl) {
                    Some(entry) if entry.is_rel_table() => {
                        rel_table_id = Some(entry.table_id());
                    }
                    Some(_) => {
                        return Err(format!("'{}' is not a rel table", lbl));
                    }
                    None => {
                        return Err(format!("Rel table '{}' not found", lbl));
                    }
                }
            }

            if let Some(ref v) = edge_var {
                if existing_vars.iter().any(|bv| bv.name == *v) || new_vars.iter().any(|bv| bv.name == *v) {
                    return Err(format!("Variable '{}' already defined", v));
                }
            }

            new_vars.push(BoundVariable {
                name: edge_var.clone().unwrap_or_else(|| "_anon_edge_".to_string()),
                table_id: rel_table_id.unwrap_or(0),
                label: edge_label.clone(),
                is_node: false,
            });

            bound_edge = Some(BoundEdgePattern {
                variable: e.variable.clone(),
                label: edge_label,
                rel_table_id,
                direction: e.direction.clone(),
                properties: e.properties.clone(),
                lower_bound: e.lower_bound,
                upper_bound: e.upper_bound,
            });
        }

        Ok((
            BoundPattern {
                node_variable: node_var,
                node_label,
                node_table_id,
                properties: pattern.node.as_ref().map(|n| n.properties.clone()).unwrap_or_default(),
                edge: bound_edge,
            },
            new_vars,
        ))
    }

    // ==================== RETURN Binding ====================

    fn bind_return(&self, r: &ReturnClause, variables: &[BoundVariable]) -> Result<BoundReturnClause, String> {
        let mut expressions = Vec::new();
        for item in &r.expressions {
            match &item.expression {
                Expression::Star => {
                    // Expand * to all variables in scope
                    if variables.is_empty() {
                        return Err("RETURN or WITH * is not allowed when there are no variables in scope.".to_string());
                    }
                    for var in variables {
                        expressions.push(BoundExpression {
                            expression: Expression::Variable(var.name.clone()),
                            resolved_type: if var.is_node {
                                LogicalTypeID::Node
                            } else {
                                LogicalTypeID::Rel
                            },
                            is_constant: false,
                        });
                    }
                }
                _ => {
                    let resolved = self.resolve_expression(&item.expression, variables)?;
                    expressions.push(resolved);
                }
            }
        }
        Ok(BoundReturnClause { expressions })
    }

    // ==================== WHERE Binding ====================

    fn bind_where(&self, w: &WhereClause, variables: &[BoundVariable]) -> Result<BoundWhereClause, String> {
        let resolved = self.resolve_expression(&w.expression, variables)?;
        // WHERE expressions must be boolean
        if resolved.resolved_type != LogicalTypeID::Bool && resolved.resolved_type != LogicalTypeID::Any {
            return Err(format!(
                "WHERE clause must be boolean, got {:?}",
                resolved.resolved_type
            ));
        }
        Ok(BoundWhereClause { expression: resolved })
    }

    // ==================== CREATE (MATCH CREATE) Binding ====================

    fn bind_match_create(
        &self,
        c: &CreateClause,
        existing_vars: &[BoundVariable],
    ) -> Result<(BoundMatchClause, Vec<BoundVariable>), String> {
        // CREATE patterns follow the same structure as MATCH patterns
        let mut patterns = Vec::new();
        let mut new_vars = Vec::new();

        for pattern in &c.patterns {
            let all_vars: Vec<BoundVariable> = existing_vars.iter().cloned().chain(new_vars.iter().cloned()).collect();
            let (bound, nv) = self.bind_pattern(pattern, &all_vars, true)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        Ok((
            BoundMatchClause {
                patterns,
                new_variables: new_vars.clone(),
                fts_query: None, // Optional MATCH in Foreach doesn't carry FTS
            },
            new_vars,
        ))
    }

    // ==================== Expression Resolution ====================

    fn resolve_expression(&self, expr: &Expression, variables: &[BoundVariable]) -> Result<BoundExpression, String> {
        match expr {
            Expression::Constant(c) => {
                let typ = match c {
                    Constant::Null => LogicalTypeID::Any,
                    Constant::Bool(_) => LogicalTypeID::Bool,
                    Constant::Integer(_) => LogicalTypeID::Int64,
                    Constant::Float(_) => LogicalTypeID::Double,
                    Constant::String(_) => LogicalTypeID::String,
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: typ,
                    is_constant: true,
                })
            }
            Expression::Variable(name) => {
                // Check if variable is in scope
                if let Some(var) = variables.iter().find(|v| v.name == *name) {
                    let typ = if var.is_node {
                        LogicalTypeID::Node
                    } else {
                        LogicalTypeID::Rel
                    };
                    Ok(BoundExpression {
                        expression: expr.clone(),
                        resolved_type: typ,
                        is_constant: false,
                    })
                } else if name.to_uppercase() == "COUNT" || name == "*" {
                    // Special handling for COUNT(*)
                    Ok(BoundExpression {
                        expression: expr.clone(),
                        resolved_type: LogicalTypeID::Int64,
                        is_constant: false,
                    })
                } else {
                    // Check catalog for table references
                    let catalog = self.catalog.lock().unwrap();
                    if let Some(entry) = catalog.get_entry_by_name(name) {
                        let typ = if entry.is_node_table() {
                            LogicalTypeID::Node
                        } else {
                            LogicalTypeID::Rel
                        };
                        Ok(BoundExpression {
                            expression: expr.clone(),
                            resolved_type: typ,
                            is_constant: false,
                        })
                    } else {
                        Err(format!("Variable '{}' not in scope", name))
                    }
                }
            }
            Expression::Parameter(_name) => {
                // Parameters are unresolved at bind time; assign Any type.
                // Type checking happens at execute time when values are provided.
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::Any,
                    is_constant: false,
                })
            }
            Expression::PropertyAccess(obj, prop) => {
                let _bound_obj = self.resolve_expression(obj, variables)?;
                // For now, resolve property types to common defaults
                let prop_type = match prop.as_str() {
                    "name" | "title" | "label" => LogicalTypeID::String,
                    "age" | "count" | "length" | "size" | "id" => LogicalTypeID::Int64,
                    "score" | "price" | "rating" => LogicalTypeID::Double,
                    "active" | "is_active" | "deleted" => LogicalTypeID::Bool,
                    "date" | "created_at" | "updated_at" => LogicalTypeID::Date,
                    _ => LogicalTypeID::Any, // Unknown properties default to Any
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: prop_type,
                    is_constant: false,
                })
            }
            Expression::FunctionCall(name, args) => {
                let resolved_args: Result<Vec<BoundExpression>, String> =
                    args.iter().map(|a| self.resolve_expression(a, variables)).collect();
                let _args = resolved_args?;
                let return_type = match name.to_uppercase().as_str() {
                    "COUNT" | "SUM" | "MIN" | "MAX" | "AVG" => LogicalTypeID::Int64,
                    "NEXTVAL" | "CURRVAL" => LogicalTypeID::Int64,
                    "STARTS_WITH" | "ENDS_WITH" | "CONTAINS" => LogicalTypeID::Bool,
                    "TO_UPPER" | "TO_LOWER" | "UPPER" | "LOWER" | "UCASE" | "LCASE" | "TRIM" | "SUBSTRING"
                    | "REPLACE" => LogicalTypeID::String,
                    "ABS" | "CEIL" | "CEILING" | "FLOOR" | "ROUND" | "SQRT" | "LOG" | "EXP" | "SIN" | "COS" | "TAN" => {
                        LogicalTypeID::Double
                    }
                    "DATE" | "TIMESTAMP" => LogicalTypeID::Date,
                    "INT64" | "INT" => LogicalTypeID::Int64,
                    "FLOAT" | "DOUBLE" | "BOOL" | "BOOLEAN" | "STRING" | "BLOB" => LogicalTypeID::String,
                    _ => LogicalTypeID::Any,
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: return_type,
                    is_constant: false,
                })
            }
            Expression::BinaryOp(op, left, right) => {
                let left = self.resolve_expression(left, variables)?;
                let right = self.resolve_expression(right, variables)?;
                let result_type = match op {
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LessThan
                    | BinaryOp::LessThanOrEqual
                    | BinaryOp::GreaterThan
                    | BinaryOp::GreaterThanOrEqual
                    | BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::In
                    | BinaryOp::NotIn
                    | BinaryOp::StartsWith
                    | BinaryOp::EndsWith
                    | BinaryOp::Contains => LogicalTypeID::Bool,
                    BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo => {
                        // Propagate numeric type
                        if left.resolved_type == LogicalTypeID::Double || right.resolved_type == LogicalTypeID::Double {
                            LogicalTypeID::Double
                        } else {
                            LogicalTypeID::Int64
                        }
                    }
                    BinaryOp::Concat => LogicalTypeID::String,
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: result_type,
                    is_constant: left.is_constant && right.is_constant,
                })
            }
            Expression::UnaryOp(op, inner) => {
                let inner = self.resolve_expression(inner, variables)?;
                let result_type = match op {
                    UnaryOp::Not | UnaryOp::IsNull | UnaryOp::IsNotNull => LogicalTypeID::Bool,
                    UnaryOp::Negate => inner.resolved_type,
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: result_type,
                    is_constant: inner.is_constant,
                })
            }
            Expression::List(items) => {
                let resolved: Result<Vec<BoundExpression>, String> =
                    items.iter().map(|i| self.resolve_expression(i, variables)).collect();
                resolved?;
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::List,
                    is_constant: false,
                })
            }
            Expression::Map(entries) => {
                for (_, v) in entries {
                    self.resolve_expression(v, variables)?;
                }
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::Map,
                    is_constant: false,
                })
            }
            Expression::ExistsSubquery(query) => {
                // Bind the inner query. EXISTS returns Bool.
                // For now, do NOT pass outer variables (uncorrelated subquery).
                let _bound = self.bind_query(*query.clone())?;
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::Bool,
                    is_constant: false,
                })
            }
            Expression::Case(case_expr) => {
                // Bind subject (if any), all WHEN/THEN expressions, and ELSE.
                // Return type is inferred from the first THEN branch.
                if let Some(subj) = &case_expr.subject {
                    self.resolve_expression(subj, variables)?;
                }
                let mut result_type = LogicalTypeID::Any;
                for alt in &case_expr.alternatives {
                    self.resolve_expression(&alt.when, variables)?;
                    let then_bound = self.resolve_expression(&alt.then, variables)?;
                    if result_type == LogicalTypeID::Any {
                        result_type = then_bound.resolved_type;
                    }
                }
                if let Some(else_e) = &case_expr.else_expr {
                    let else_bound = self.resolve_expression(else_e, variables)?;
                    if result_type == LogicalTypeID::Any {
                        result_type = else_bound.resolved_type;
                    }
                }
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: result_type,
                    is_constant: false,
                })
            }
            Expression::Star => {
                // Star should be expanded by bind_return before reaching here.
                // If reached, return Any type.
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::Any,
                    is_constant: false,
                })
            }
            Expression::ListPredicate {
                quantifier: _,
                list,
                var_name,
                predicate,
            } => {
                // Bind both list and predicate expressions
                self.resolve_expression(list, variables)?;

                let mut new_vars = variables.to_vec();
                new_vars.push(crate::bound_statement::BoundVariable {
                    name: var_name.clone(),
                    table_id: 0,
                    label: None,
                    is_node: false,
                });

                self.resolve_expression(predicate, &new_vars)?;
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: LogicalTypeID::Bool,
                    is_constant: false,
                })
            }
        }
    }

    // ==================== DDL Binding ====================

    fn bind_create_node_table(&self, t: CreateNodeTable) -> Result<BoundStatement, String> {
        if t.name.is_empty() {
            return Err("Table name cannot be empty".into());
        }

        let mut columns = Vec::new();
        for col in &t.columns {
            let logical_type = Self::parse_type(&col.type_name)?;
            columns.push(CatalogColumn {
                name: col.name.clone(),
                logical_type,
                is_primary_key: col.name == t.primary_key,
                default_value: None,
            });
        }

        if columns.is_empty() {
            return Err("Table must have at least one column".into());
        }

        // Verify primary key exists
        if !columns.iter().any(|c| c.is_primary_key) {
            return Err(format!("Primary key column '{}' not found in columns", t.primary_key));
        }

        // Register with catalog
        let mut catalog = self.catalog.lock().unwrap();
        match catalog.create_node_table(t.name.clone(), columns.clone()) {
            CatalogResult::Created { .. } => {}
            CatalogResult::AlreadyExists => {
                return Err(format!("Table '{}' already exists", t.name));
            }
            _ => return Err("Failed to create table".into()),
        }

        Ok(BoundStatement::BoundCreateNodeTable(BoundCreateNodeTable {
            name: t.name,
            columns,
            primary_key: t.primary_key,
        }))
    }

    fn bind_create_vector_index(&self, v: kuzu_parser::ast::CreateVectorIndex) -> Result<BoundStatement, String> {
        if v.index_name.is_empty() {
            return Err("Index name cannot be empty".into());
        }
        if v.metric.is_empty() {
            return Err("Metric must be specified (cosine, euclidean, l2, or dot)".into());
        }
        if v.dimensions == 0 {
            return Err("Dimensions must be greater than 0".into());
        }

        // Validate the referenced table exists in the catalog
        let catalog = self.catalog.lock().unwrap();
        let entry = catalog
            .get_entry_by_name(&v.table_name)
            .ok_or_else(|| format!("Table '{}' not found", v.table_name))?;

        // Validate the referenced column exists in the table
        let col_exists = entry.columns().iter().any(|c| c.name == v.column_name);
        if !col_exists {
            return Err(format!(
                "Column '{}' not found in table '{}'",
                v.column_name, v.table_name
            ));
        }

        // Validate metric value
        match v.metric.to_lowercase().as_str() {
            "cosine" | "euclidean" | "l2" | "dot" => {}
            other => {
                return Err(format!(
                    "Unknown metric '{other}'. Supported: cosine, euclidean, l2, dot"
                ));
            }
        }

        // Register with catalog
        let mut catalog = self.catalog.lock().unwrap();
        match catalog.create_vector_index(
            v.index_name.clone(),
            v.table_name.clone(),
            v.column_name.clone(),
            v.metric.clone(),
            v.dimensions,
        ) {
            CatalogResult::Created { .. } => {}
            CatalogResult::AlreadyExists => {
                return Err(format!("Vector index '{}' already exists", v.index_name));
            }
            CatalogResult::NotFound => {
                return Err(format!("Table '{}' not found", v.table_name));
            }
            CatalogResult::Dropped { .. } => {
                return Err("Unexpected: Dropped result from create_vector_index".into());
            }
        }

        Ok(BoundStatement::BoundCreateVectorIndex(BoundCreateVectorIndex {
            index_name: v.index_name,
            table_name: v.table_name,
            column_name: v.column_name,
            metric: v.metric,
            dimensions: v.dimensions,
        }))
    }

    fn bind_create_index(&self, v: kuzu_parser::ast::CreateIndex) -> Result<BoundStatement, String> {
        if v.index_name.is_empty() {
            return Err("Index name cannot be empty".into());
        }

        // Parse index type
        let index_type = IndexType::from_str(&v.index_type)
            .ok_or_else(|| format!("Unknown index type '{}'. Use ART or HASH", v.index_type))?;

        // Validate table and column exist
        {
            let catalog = self.catalog.lock().unwrap();
            let entry = catalog
                .get_entry_by_name(&v.table_name)
                .ok_or_else(|| format!("Table '{}' not found", v.table_name))?;

            // Validate column exists and is PK
            let col_exists = entry.columns().iter().any(|c| c.name == v.property);
            if !col_exists {
                return Err(format!("Column '{}' not found in table '{}'", v.property, v.table_name));
            }

            let pk_col = entry.columns().iter().find(|c| c.is_primary_key);
            if pk_col.map(|c| c.name.as_str()) != Some(v.property.as_str()) {
                return Err(format!(
                    "Cannot create index on non-PK column '{}'. Only PK columns are supported.",
                    v.property
                ));
            }
        }

        // Register with catalog (separate lock for mutable access)
        let mut catalog = self.catalog.lock().unwrap();
        catalog.create_index(&v.table_name, v.index_name.clone(), index_type, &v.property)?;

        Ok(BoundStatement::BoundCreateIndex(BoundCreateIndex {
            index_type,
            index_name: v.index_name,
            table_name: v.table_name,
            column_name: v.property,
        }))
    }

    fn bind_drop_index(&self, v: kuzu_parser::ast::DropIndex) -> Result<BoundStatement, String> {
        if v.index_name.is_empty() {
            return Err("Index name cannot be empty".into());
        }

        let mut catalog = self.catalog.lock().unwrap();
        match catalog.drop_index(&v.table_name, &v.index_name) {
            Ok(()) => {}
            Err(e) => return Err(e),
        }

        Ok(BoundStatement::BoundDropIndex(BoundDropIndex {
            index_name: v.index_name,
            table_name: v.table_name,
        }))
    }

    fn bind_create_rel_table(&self, t: CreateRelTable) -> Result<BoundStatement, String> {
        if t.name.is_empty() {
            return Err("Table name cannot be empty".into());
        }

        // Validate FROM and TO tables exist
        let catalog = self.catalog.lock().unwrap();
        let src_id = catalog
            .get_table_id(&t.from)
            .ok_or_else(|| format!("Source table '{}' not found", t.from))?;
        let dst_id = catalog
            .get_table_id(&t.to)
            .ok_or_else(|| format!("Destination table '{}' not found", t.to))?;
        drop(catalog);

        let mut columns = Vec::new();
        for col in &t.columns {
            let logical_type = Self::parse_type(&col.type_name)?;
            columns.push(CatalogColumn {
                name: col.name.clone(),
                logical_type,
                is_primary_key: false,
                default_value: None,
            });
        }

        // Register with catalog
        let mut catalog = self.catalog.lock().unwrap();
        match catalog.create_rel_table(t.name.clone(), src_id, dst_id, columns.clone()) {
            CatalogResult::Created { .. } => {}
            CatalogResult::AlreadyExists => {
                return Err(format!("Rel table '{}' already exists", t.name));
            }
            _ => return Err("Failed to create rel table".into()),
        }

        Ok(BoundStatement::BoundCreateRelTable(BoundCreateRelTable {
            name: t.name,
            from: t.from,
            to: t.to,
            columns,
        }))
    }

    fn bind_drop_table(&self, t: DropTable) -> Result<BoundStatement, String> {
        let mut catalog = self.catalog.lock().unwrap();
        match catalog.drop_table(&t.name) {
            CatalogResult::Dropped { .. } => Ok(BoundStatement::BoundDropTable(BoundDropTable { name: t.name })),
            CatalogResult::NotFound => Err(format!("Table '{}' not found", t.name)),
            _ => Err("Failed to drop table".into()),
        }
    }

    fn bind_unwind(&self, u: &kuzu_parser::ast::UnwindClause) -> Result<BoundUnwindClause, String> {
        // Validate the expression is a list literal or variable reference to a list
        match &u.expression {
            kuzu_parser::ast::Expression::List(_) => {}
            kuzu_parser::ast::Expression::Variable(_) => {}
            _ => return Err(format!("UNWIND requires a list expression, got: {:?}", u.expression)),
        }
        if u.variable.is_empty() {
            return Err("UNWIND requires a variable name".into());
        }
        Ok(BoundUnwindClause {
            expression: u.expression.clone(),
            variable: u.variable.clone(),
        })
    }

    fn bind_foreach(
        &self,
        f: &kuzu_parser::ast::ForeachClause,
        variables: &[BoundVariable],
    ) -> Result<BoundForeachClause, String> {
        // Validate the expression is a list
        match &f.expression {
            kuzu_parser::ast::Expression::List(_) | kuzu_parser::ast::Expression::Variable(_) => {}
            _ => return Err(format!("FOREACH requires a list expression, got: {:?}", f.expression)),
        }
        if f.variable.is_empty() {
            return Err("FOREACH requires a variable name".into());
        }
        // Create a new variable scope for the foreach body
        let mut local_vars = variables.to_vec();
        local_vars.push(BoundVariable {
            name: f.variable.clone(),
            table_id: 0,
            label: None,
            is_node: false,
        });

        // Bind sub-statements
        let mut sub_statements = Vec::new();
        for clause in &f.clauses {
            match clause {
                kuzu_parser::ast::Clause::Create(cc) => {
                    // Bind as DML CREATE (BoundCreateDml), not as a MATCH clause
                    let bound = self.bind_create_dml(cc.clone(), &local_vars)?;
                    sub_statements.push(bound);
                }
                kuzu_parser::ast::Clause::Set(sc) => {
                    // Manually wrap SET in BoundQuery to preserve variable scope
                    let bound_set = self.bind_set(sc, &local_vars)?;
                    sub_statements.push(BoundStatement::BoundQuery(BoundQuery {
                        clauses: vec![BoundClause::BoundSet(bound_set)],
                        variables: local_vars.clone(),
                    }));
                }
                kuzu_parser::ast::Clause::Delete(dc) => {
                    // Manually wrap DELETE in BoundQuery to preserve variable scope
                    let bound_delete = self.bind_delete(dc, &local_vars)?;
                    sub_statements.push(BoundStatement::BoundQuery(BoundQuery {
                        clauses: vec![BoundClause::BoundDelete(bound_delete)],
                        variables: local_vars.clone(),
                    }));
                }
                _ => {
                    return Err(format!("Unsupported FOREACH sub-clause: {:?}", clause));
                }
            }
        }
        Ok(BoundForeachClause {
            variable: f.variable.clone(),
            expression: f.expression.clone(),
            sub_statements,
        })
    }

    fn bind_optional_match(
        &self,
        m: &kuzu_parser::ast::OptionalMatchClause,
        existing_vars: &[BoundVariable],
    ) -> Result<(BoundMatchClause, Vec<BoundVariable>), String> {
        let mut patterns = Vec::new();
        let mut new_vars = Vec::new();

        for pattern in &m.patterns {
            let all_vars: Vec<BoundVariable> = existing_vars.iter().cloned().chain(new_vars.iter().cloned()).collect();
            let (bound, nv) = self.bind_pattern(pattern, &all_vars, false)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        Ok((
            BoundMatchClause {
                patterns,
                new_variables: new_vars.clone(),
                fts_query: None, // Optional MATCH doesn't carry FTS
            },
            new_vars,
        ))
    }

    fn bind_set(&self, s: &kuzu_parser::ast::SetClause, variables: &[BoundVariable]) -> Result<BoundSetClause, String> {
        let mut items = Vec::new();
        for item in &s.items {
            // Property must be of form `variable.property`
            match &item.property {
                kuzu_parser::ast::Expression::PropertyAccess(var_expr, prop_name) => {
                    match var_expr.as_ref() {
                        kuzu_parser::ast::Expression::Variable(var_name) => {
                            let bound_var = variables
                                .iter()
                                .find(|v| v.name == *var_name)
                                .ok_or_else(|| format!("Variable '{}' not in scope for SET", var_name))?;
                            items.push(BoundSetItem {
                                property: item.property.clone(),
                                value: item.value.clone(),
                                column_name: prop_name.clone(),
                                column_idx: 0, // resolved by catalog lookup
                                table_name: bound_var.label.clone().unwrap_or_default(),
                                table_id: bound_var.table_id,
                                is_node: bound_var.is_node,
                            });
                        }
                        _ => return Err("SET property must be on a variable".into()),
                    }
                }
                _ => return Err("SET requires property access expression (e.g., n.age)".into()),
            }
        }
        Ok(BoundSetClause { items })
    }

    fn bind_union(&self, u: kuzu_parser::ast::UnionStatement) -> Result<BoundStatement, String> {
        let left = self.bind_query(u.left)?;
        let right = self.bind_query(u.right)?;
        if let (BoundStatement::BoundQuery(lq), BoundStatement::BoundQuery(rq)) = (&left, &right) {
            if lq.clauses.len() != rq.clauses.len() {
                return Err("UNION queries must have compatible structures".into());
            }
        }
        Ok(BoundStatement::BoundUnion(BoundUnion {
            left: Box::new(match left {
                BoundStatement::BoundQuery(q) => q,
                _ => unreachable!(),
            }),
            right: Box::new(match right {
                BoundStatement::BoundQuery(q) => q,
                _ => unreachable!(),
            }),
            all: u.all,
        }))
    }

    fn bind_merge(&self, m: kuzu_parser::ast::MergeStatement) -> Result<BoundStatement, String> {
        // Use the first pattern from the patterns vector
        let pattern = m.patterns.first().ok_or("MERGE requires at least one pattern")?;
        let node = pattern.node.as_ref().ok_or("MERGE requires a node pattern")?;
        let label = node.labels.first().ok_or("MERGE requires a label (table name)")?;

        // Lookup the table in catalog
        let catalog = self.catalog.lock().unwrap();
        let entry = catalog
            .get_entry_by_name(label)
            .ok_or_else(|| format!("Table '{label}' not found"))?;

        let table_id = entry.table_id();
        let table_name = label.clone();

        // Get properties for matching/creation
        let properties: Vec<(String, kuzu_parser::ast::Expression)> = node.properties.clone();

        // Resolve ON CREATE SET items
        let on_create = resolve_set_items(&catalog, &m.on_create)?;
        let on_match = resolve_set_items(&catalog, &m.on_match)?;

        Ok(BoundStatement::BoundMerge(BoundMerge {
            table_name,
            table_id,
            properties,
            on_create,
            on_match,
        }))
    }

    fn bind_create_dml(
        &self,
        c: kuzu_parser::ast::CreateClause,
        _variables: &[BoundVariable],
    ) -> Result<BoundStatement, String> {
        let node = c
            .patterns
            .first()
            .and_then(|p| p.node.as_ref())
            .ok_or("CREATE DML requires a node pattern")?;
        let label = node.labels.first().ok_or("CREATE DML requires a label (table name)")?;

        let catalog = self.catalog.lock().unwrap();
        let entry = catalog
            .get_entry_by_name(label)
            .ok_or_else(|| format!("Table '{label}' not found"))?;

        let table_id = entry.table_id();
        let table_name = label.clone();

        Ok(BoundStatement::BoundCreateDml(BoundCreateDml {
            table_name,
            table_id,
            properties: node.properties.clone(),
        }))
    }

    fn bind_call(&self, c: kuzu_parser::ast::CallStatement) -> Result<BoundStatement, String> {
        // Note: CALL create_fts_index is superseded by the DDL `CREATE FTS INDEX` statement.
        // CALL is a table function invocation ΓÇö validate the function exists
        // in the function registry. At binding time we just pass through;
        // resolution happens at execution time.
        Ok(BoundStatement::BoundCall(BoundCall {
            function_name: c.function_name,
            args: c.args,
        }))
    }

    fn bind_explain(&self, e: kuzu_parser::ast::ExplainStatement) -> Result<BoundStatement, String> {
        // Bind the inner statement recursively
        let inner = self.bind(*e.statement)?;
        Ok(BoundStatement::BoundExplain(BoundExplain {
            inner: Box::new(inner),
            explain_type: e.explain_type,
        }))
    }

    fn bind_create_sequence(&self, s: kuzu_parser::ast::CreateSequence) -> Result<BoundStatement, String> {
        // Compute defaults matching C++ behavior:
        // - START WITH: 1 for increment > 0, max_value for increment < 0
        // - INCREMENT: 1 (default)
        // - MINVALUE: 1 for increment > 0, i64::MIN for increment < 0
        // - MAXVALUE: i64::MAX for increment > 0, -1 for increment < 0
        // - CYCLE: false (default)
        let increment = s.increment.unwrap_or(1);
        if increment == 0 {
            return Err("INCREMENT must not be zero".into());
        }
        let start_with = s.start_with.unwrap_or(if increment > 0 { 1 } else { -1 });
        let min_value = s.min_value.unwrap_or(if increment > 0 { 1 } else { i64::MIN });
        let max_value = s.max_value.unwrap_or(if increment > 0 { i64::MAX } else { -1 });
        let cycle = s.cycle.unwrap_or(false);

        // Validate min/max/start consistency
        if min_value > max_value {
            return Err(format!(
                "MINVALUE ({}) cannot be greater than MAXVALUE ({})",
                min_value, max_value
            ));
        }
        if start_with < min_value || start_with > max_value {
            return Err(format!(
                "START WITH ({}) must be between MINVALUE ({}) and MAXVALUE ({})",
                start_with, min_value, max_value
            ));
        }

        Ok(BoundStatement::BoundCreateSequence(BoundCreateSequence {
            name: s.name,
            if_not_exists: s.if_not_exists,
            or_replace: s.or_replace,
            start_with,
            increment,
            min_value,
            max_value,
            cycle,
        }))
    }

    fn bind_drop_sequence(&self, s: kuzu_parser::ast::DropSequence) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundDropSequence(BoundDropSequence {
            name: s.name,
            if_exists: s.if_exists,
        }))
    }

    fn bind_create_macro(&self, m: kuzu_parser::ast::CreateMacro) -> Result<BoundStatement, String> {
        // Convert default args to strings
        let default_args: Vec<(String, String)> = m
            .default_args
            .iter()
            .map(|(name, expr)| (name.clone(), expr_to_debug_string(expr)))
            .collect();
        let expression_str = expr_to_debug_string(&m.expression);
        Ok(BoundStatement::BoundCreateMacro(BoundCreateMacro {
            name: m.name,
            positional_args: m.positional_args,
            default_args,
            expression: expression_str,
        }))
    }

    fn bind_export_database(&self, e: kuzu_parser::ast::ExportDatabase) -> Result<BoundStatement, String> {
        let file_type = e
            .options
            .get("FORMAT")
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "csv".to_string());
        if file_type != "csv" && file_type != "parquet" {
            return Err(format!(
                "Unsupported export format '{file_type}'. Supported: csv, parquet"
            ));
        }
        let schema_only = e.options.get("SCHEMA_ONLY").map(|s| s == "true").unwrap_or(false);
        Ok(BoundStatement::BoundExportDatabase(BoundExportDatabase {
            file_path: e.file_path,
            file_type,
            schema_only,
            options: e.options,
        }))
    }

    fn bind_import_database(&self, i: kuzu_parser::ast::ImportDatabase) -> Result<BoundStatement, String> {
        // Validate the import directory exists and read the schema/cypher files
        let path = std::path::Path::new(&i.file_path);
        if !path.exists() {
            return Err(format!("Import directory '{}' not found", i.file_path));
        }
        if !path.is_dir() {
            return Err(format!("'{}' is not a directory", i.file_path));
        }

        let schema_path = path.join("schema.cypher");
        let copy_path = path.join("copy.cypher");
        let index_path = path.join("index.cypher");

        if !schema_path.exists() {
            return Err(format!("schema.cypher not found in '{}'", i.file_path));
        }

        let query = if copy_path.exists() {
            let schema =
                std::fs::read_to_string(&schema_path).map_err(|e| format!("Cannot read schema.cypher: {e}"))?;
            let copy = std::fs::read_to_string(&copy_path).map_err(|e| format!("Cannot read copy.cypher: {e}"))?;
            format!("{schema}\n{copy}")
        } else {
            std::fs::read_to_string(&schema_path).map_err(|e| format!("Cannot read schema.cypher: {e}"))?
        };

        let index_query = if index_path.exists() {
            std::fs::read_to_string(&index_path).map_err(|e| format!("Cannot read index.cypher: {e}"))?
        } else {
            String::new()
        };

        Ok(BoundStatement::BoundImportDatabase(BoundImportDatabase {
            file_path: i.file_path,
            query,
            index_query,
        }))
    }

    /// Bind ANALYZE statement ΓÇö resolve table names to table IDs.
    fn bind_analyze(&self, a: AnalyzeStatement) -> Result<BoundStatement, String> {
        let cat = self.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
        let table_ids = if let Some(ref table_name) = a.table_name {
            let id = cat
                .get_table_id(table_name)
                .ok_or_else(|| format!("Table '{table_name}' not found"))?;
            vec![id]
        } else {
            // ANALYZE * ΓÇö collect stats for all node/rel tables
            cat.all_entries()
                .filter(|e| e.is_node_table() || e.is_rel_table())
                .map(|e| e.table_id())
                .collect()
        };
        Ok(BoundStatement::BoundAnalyze(BoundAnalyze {
            table_name: a.table_name,
            table_ids,
        }))
    }

    /// Bind TRANSACTION statement — trivial (no catalog resolution needed).
    fn bind_transaction(&self, t: TransactionStatement) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundTransaction(BoundTransaction {
            action: t.action,
        }))
    }

    /// Bind EXTENSION statement — trivial (validated at execution time).
    fn bind_extension(&self, e: ExtensionStatement) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundExtension(BoundExtension {
            action: e.action,
            name: e.name,
        }))
    }

    fn bind_attach_database(&self, a: AttachDatabase) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundAttachDatabase(BoundAttachDatabase {
            path: a.path,
            alias: a.alias,
            options: a.options,
        }))
    }

    fn bind_detach_database(&self, d: DetachDatabase) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundDetachDatabase(BoundDetachDatabase {
            alias: d.alias,
        }))
    }

    fn bind_use_database(&self, u: UseDatabase) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundUseDatabase(BoundUseDatabase {
            alias: u.alias,
        }))
    }

    fn bind_load_from(&self, l: LoadFrom) -> Result<BoundStatement, String> {
        Ok(BoundStatement::BoundLoadFrom(BoundLoadFrom {
            path: l.path,
            options: l.options,
        }))
    }

    fn bind_create_fts_index(&self, f: CreateFtsIndex) -> Result<BoundStatement, String> {
        // Validate table and column exist
        {
            let catalog = self.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
            let entry = catalog
                .get_entry_by_name(&f.table_name)
                .ok_or_else(|| format!("Table '{}' not found", f.table_name))?;
            let has_column = entry.columns().iter().any(|c| c.name == f.column_name);
            if !has_column {
                return Err(format!(
                    "Column '{}' not found in table '{}'",
                    f.column_name, f.table_name
                ));
            }
        }
        let index_name = f.index_name.clone();
        let docs_table = format!("fts_{index_name}_docs");
        let terms_table = format!("fts_{index_name}_terms");
        let posting_table = format!("fts_{index_name}_appears_in");

        // Register macro tables in the logical catalog
        {
            let mut catalog = self.catalog.lock().unwrap();

            let docs_cols = vec![
                kuzu_catalog::CatalogColumn {
                    name: "doc_id".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                    default_value: None,
                },
                kuzu_catalog::CatalogColumn {
                    name: "text".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                    default_value: None,
                },
            ];
            let docs_id = match catalog.create_node_table(docs_table.clone(), docs_cols) {
                kuzu_catalog::CatalogResult::Created { table_id } => table_id,
                kuzu_catalog::CatalogResult::AlreadyExists => {
                    return Err(format!("Table '{}' already exists", docs_table));
                }
                _ => return Err("Failed to create docs table".into()),
            };

            let terms_cols = vec![
                kuzu_catalog::CatalogColumn {
                    name: "term_id".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: true,
                    default_value: None,
                },
                kuzu_catalog::CatalogColumn {
                    name: "term".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::String,
                    is_primary_key: false,
                    default_value: None,
                },
                kuzu_catalog::CatalogColumn {
                    name: "doc_freq".into(),
                    logical_type: kuzu_common::types::LogicalTypeID::Int64,
                    is_primary_key: false,
                    default_value: None,
                },
            ];
            let terms_id = match catalog.create_node_table(terms_table.clone(), terms_cols) {
                kuzu_catalog::CatalogResult::Created { table_id } => table_id,
                kuzu_catalog::CatalogResult::AlreadyExists => {
                    return Err(format!("Table '{}' already exists", terms_table));
                }
                _ => return Err("Failed to create terms table".into()),
            };

            let posting_cols = vec![kuzu_catalog::CatalogColumn {
                name: "term_freq".into(),
                logical_type: kuzu_common::types::LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }];
            match catalog.create_rel_table(posting_table.clone(), terms_id, docs_id, posting_cols) {
                kuzu_catalog::CatalogResult::Created { .. } => {}
                kuzu_catalog::CatalogResult::AlreadyExists => {
                    return Err(format!("Table '{}' already exists", posting_table));
                }
                _ => return Err("Failed to create posting table".into()),
            }
        }

        Ok(BoundStatement::BoundCreateFtsIndex(BoundCreateFtsIndex {
            index_name: f.index_name,
            table_name: f.table_name,
            column_name: f.column_name,
            if_not_exists: f.if_not_exists,
            docs_table,
            terms_table,
            posting_table,
        }))
    }

    fn bind_alter_table(&self, a: kuzu_parser::ast::AlterTable) -> Result<BoundStatement, String> {
        // Validate table exists and extract column info
        let col_names: Vec<String> = {
            let catalog = self.catalog.lock().unwrap();
            let entry = catalog
                .get_entry_by_name(&a.table_name)
                .ok_or_else(|| format!("Table '{}' not found", a.table_name))?;
            entry.columns().iter().map(|c| c.name.clone()).collect()
        };

        fn has_name(col_names: &[String], name: &str) -> bool {
            col_names.iter().any(|c| c.eq_ignore_ascii_case(name))
        }

        // Validate alter action
        match &a.action {
            kuzu_parser::ast::AlterAction::AddColumn { name: _, type_name } => {
                Self::parse_type(type_name)?;
            }
            kuzu_parser::ast::AlterAction::DropColumn { name } => {
                if !has_name(&col_names, name) {
                    return Err(format!("Column '{name}' not found in table '{}'", a.table_name));
                }
            }
            kuzu_parser::ast::AlterAction::RenameColumn { old_name, new_name } => {
                if !has_name(&col_names, old_name) {
                    return Err(format!("Column '{old_name}' not found in table '{}'", a.table_name));
                }
                if has_name(&col_names, new_name) {
                    return Err(format!(
                        "Column '{new_name}' already exists in table '{}'",
                        a.table_name
                    ));
                }
            }
            kuzu_parser::ast::AlterAction::RenameTable { new_name: _ } => {
                // Rename table duplicate check happens at execution time in the catalog
            }
        }

        Ok(BoundStatement::BoundAlterTable(BoundAlterTable {
            table_name: a.table_name,
            action: a.action,
        }))
    }

    fn bind_delete(
        &self,
        d: &kuzu_parser::ast::DeleteClause,
        variables: &[BoundVariable],
    ) -> Result<BoundDeleteClause, String> {
        if d.expressions.is_empty() {
            return Err("DELETE requires at least one expression".into());
        }

        let mut items = Vec::new();
        for expr in &d.expressions {
            match expr {
                kuzu_parser::ast::Expression::Variable(var_name) => {
                    let var = variables
                        .iter()
                        .find(|v| v.name == *var_name)
                        .ok_or_else(|| format!("Variable '{}' not found in scope for DELETE", var_name))?;
                    items.push(BoundDeleteItem {
                        expression: expr.clone(),
                        table_name: var.label.clone().unwrap_or_default(),
                        table_id: var.table_id,
                        primary_key_column: String::new(),
                        is_node: var.is_node,
                    });
                }
                _ => return Err(format!("DELETE only supports variable references, got: {:?}", expr)),
            }
        }

        Ok(BoundDeleteClause {
            detach: d.detach,
            items,
        })
    }

    fn bind_copy_from(&self, c: kuzu_parser::ast::CopyFrom) -> Result<BoundStatement, String> {
        // 1. Look up table in catalog and resolve column schema
        let catalog = self.catalog.lock().unwrap();
        let entry = catalog
            .get_entry_by_name(&c.table_name)
            .ok_or_else(|| format!("Table '{}' not found", c.table_name))?;
        let table_id = entry.table_id();
        let columns: Vec<kuzu_catalog::CatalogColumn> = entry.columns().to_vec();
        drop(catalog);

        // 2. Validate file path exists and is accessible
        let path = std::path::Path::new(&c.file_path);
        if !path.exists() {
            return Err(format!("File '{}' not found", c.file_path));
        }
        if !path.is_file() {
            return Err(format!("'{}' is not a file", c.file_path));
        }

        // 3. If HEADER=true and delimiter is known, peek at first CSV line to
        //    validate column count. If no explicit delimiter option was given,
        //    skip validation (the physical operator handles it with config-aware parsing).
        let header_val = c.options.get("HEADER").or_else(|| c.options.get("header"));
        let delim_val = c.options.get("DELIM").or_else(|| c.options.get("delim"));
        if let Some(hv) = header_val {
            if hv.eq_ignore_ascii_case("true") && delim_val.is_some() {
                let delimiter = delim_val.and_then(|d| d.chars().next()).unwrap_or(',');

                let file = std::fs::File::open(&c.file_path)
                    .map_err(|e| format!("Cannot open file '{}': {}", c.file_path, e))?;
                use std::io::{BufRead, BufReader};
                let mut reader = BufReader::new(file);
                let mut first_line = String::new();
                reader
                    .read_line(&mut first_line)
                    .map_err(|e| format!("Cannot read file '{}': {}", c.file_path, e))?;

                let trimmed = first_line.trim();
                if trimmed.is_empty() {
                    return Err(format!("File '{}' is empty, cannot validate header", c.file_path));
                }

                let csv_col_count = trimmed.split(delimiter).count();
                if csv_col_count != columns.len() {
                    return Err(format!(
                        "Column count mismatch: CSV header has {csv_col_count} columns \
                         but table '{}' has {} columns",
                        c.table_name,
                        columns.len()
                    ));
                }
            }
        }

        Ok(BoundStatement::BoundCopyFrom(BoundCopyFrom {
            table_name: c.table_name,
            table_id,
            file_path: c.file_path,
            options: c.options,
            columns,
        }))
    }

    /// Bind COPY TO ΓÇö export query results to a file.
    fn bind_copy_to(&self, c: kuzu_parser::ast::CopyTo) -> Result<BoundStatement, String> {
        // Bind the inner query
        let bound_query = match self.bind(Statement::Query(c.query))? {
            BoundStatement::BoundQuery(q) => q,
            _ => return Err("COPY TO inner statement must be a query".into()),
        };

        Ok(BoundStatement::BoundCopyTo(BoundCopyTo {
            file_path: c.file_path,
            format: c.format,
            header: c.header,
            query: bound_query,
        }))
    }
}

/// Convert an Expression AST to a debug string for storage.
/// Used by macro definition storage.
fn expr_to_debug_string(expr: &kuzu_parser::ast::Expression) -> String {
    format!("{:?}", expr)
}

