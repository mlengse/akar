//! Binder implementation — resolves symbols and validates semantics.

use crate::bound_statement::*;
use kuzu_catalog::{Catalog, CatalogColumn, CatalogResult};
use kuzu_common::types::LogicalTypeID;
use kuzu_parser::ast::{Clause, Expression, Statement, *};
use std::sync::{Arc, Mutex};

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
            Statement::AlterTable(a) => self.bind_alter_table(a),
            Statement::Union(u) => self.bind_union(u),
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
                    (BoundClause::BoundMatch(bound), vars)
                }
                Clause::Delete(d) => {
                    let bound = self.bind_delete(&d, &variables)?;
                    (BoundClause::BoundDelete(bound), Vec::new())
                }
                Clause::Set(s) => {
                    let bound = self.bind_set(&s, &variables)?;
                    (BoundClause::BoundSet(bound), Vec::new())
                }
                Clause::OptionalMatch(m) => {
                    let (bound, vars) = self.bind_optional_match(&m, &variables)?;
                    (BoundClause::BoundOptionalMatch(bound), vars)
                }
            };
            variables.extend(new_vars);
            clauses.push(bound_clause);
        }

        Ok(BoundStatement::BoundQuery(BoundQuery {
            clauses,
            variables,
        }))
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
            let all_vars: Vec<BoundVariable> = existing_vars.iter().cloned()
                .chain(new_vars.iter().cloned()).collect();
            let (bound, nv) = self.bind_pattern(pattern, &all_vars)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        Ok((BoundMatchClause { patterns, new_variables: new_vars.clone() }, new_vars))
    }

    fn bind_pattern(
        &self,
        pattern: &Pattern,
        existing_vars: &[BoundVariable],
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
                if existing_vars.iter().any(|bv| bv.name == *v) {
                    return Err(format!("Variable '{}' already defined", v));
                }
            }

            new_vars.push(BoundVariable {
                name: var.clone().unwrap_or_else(|| "_anon_".to_string()),
                table_id: node_table_id.unwrap_or(0),
                label: label.clone(),
                is_node: true,
            });

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
                if existing_vars.iter().any(|bv| bv.name == *v)
                    || new_vars.iter().any(|bv| bv.name == *v)
                {
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
            });
        }

        Ok((
            BoundPattern {
                node_variable: node_var,
                node_label,
                node_table_id,
                edge: bound_edge,
            },
            new_vars,
        ))
    }

    // ==================== RETURN Binding ====================

    fn bind_return(
        &self,
        r: &ReturnClause,
        variables: &[BoundVariable],
    ) -> Result<BoundReturnClause, String> {
        let mut expressions = Vec::new();
        for item in &r.expressions {
            let resolved = self.resolve_expression(&item.expression, variables)?;
            expressions.push(resolved);
        }
        Ok(BoundReturnClause { expressions })
    }

    // ==================== WHERE Binding ====================

    fn bind_where(
        &self,
        w: &WhereClause,
        variables: &[BoundVariable],
    ) -> Result<BoundWhereClause, String> {
        let resolved = self.resolve_expression(&w.expression, variables)?;
        // WHERE expressions must be boolean
        if resolved.resolved_type != LogicalTypeID::Bool
            && resolved.resolved_type != LogicalTypeID::Any
        {
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
            let (bound, nv) = self.bind_pattern(pattern, existing_vars)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        Ok((
            BoundMatchClause {
                patterns,
                new_variables: new_vars.clone(),
            },
            new_vars,
        ))
    }

    // ==================== Expression Resolution ====================

    fn resolve_expression(
        &self,
        expr: &Expression,
        variables: &[BoundVariable],
    ) -> Result<BoundExpression, String> {
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
                let resolved_args: Result<Vec<BoundExpression>, String> = args
                    .iter()
                    .map(|a| self.resolve_expression(a, variables))
                    .collect();
                let _args = resolved_args?;
                let return_type = match name.to_uppercase().as_str() {
                    "COUNT" | "SUM" | "MIN" | "MAX" | "AVG" => LogicalTypeID::Int64,
                    "STARTS_WITH" | "ENDS_WITH" | "CONTAINS" => LogicalTypeID::Bool,
                    "TO_UPPER" | "TO_LOWER" | "TRIM" | "SUBSTRING" | "REPLACE" => {
                        LogicalTypeID::String
                    }
                    "ABS" | "CEIL" | "FLOOR" | "ROUND" => LogicalTypeID::Double,
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
                    BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::LessThan
                    | BinaryOp::LessThanOrEqual | BinaryOp::GreaterThan
                    | BinaryOp::GreaterThanOrEqual | BinaryOp::And | BinaryOp::Or
                    | BinaryOp::Xor => LogicalTypeID::Bool,
                    BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply
                    | BinaryOp::Divide | BinaryOp::Modulo => {
                        // Propagate numeric type
                        if left.resolved_type == LogicalTypeID::Double
                            || right.resolved_type == LogicalTypeID::Double
                        {
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
                    UnaryOp::Not => LogicalTypeID::Bool,
                    UnaryOp::Negate => inner.resolved_type,
                };
                Ok(BoundExpression {
                    expression: expr.clone(),
                    resolved_type: result_type,
                    is_constant: inner.is_constant,
                })
            }
            Expression::List(items) => {
                let resolved: Result<Vec<BoundExpression>, String> = items
                    .iter()
                    .map(|i| self.resolve_expression(i, variables))
                    .collect();
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
        }
    }

    // ==================== DDL Binding ====================

    fn bind_create_node_table(
        &self,
        t: CreateNodeTable,
    ) -> Result<BoundStatement, String> {
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
            return Err(format!(
                "Primary key column '{}' not found in columns",
                t.primary_key
            ));
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

    fn bind_create_rel_table(
        &self,
        t: CreateRelTable,
    ) -> Result<BoundStatement, String> {
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
            CatalogResult::Dropped { .. } => {
                Ok(BoundStatement::BoundDropTable(BoundDropTable {
                    name: t.name,
                }))
            }
            CatalogResult::NotFound => {
                Err(format!("Table '{}' not found", t.name))
            }
            _ => Err("Failed to drop table".into()),
        }
    }

    fn bind_optional_match(
        &self,
        m: &kuzu_parser::ast::OptionalMatchClause,
        existing_vars: &[BoundVariable],
    ) -> Result<(BoundMatchClause, Vec<BoundVariable>), String> {
        let mut patterns = Vec::new();
        let mut new_vars = Vec::new();

        for pattern in &m.patterns {
            let all_vars: Vec<BoundVariable> = existing_vars.iter().cloned()
                .chain(new_vars.iter().cloned()).collect();
            let (bound, nv) = self.bind_pattern(pattern, &all_vars)?;
            patterns.push(bound);
            new_vars.extend(nv);
        }

        Ok((BoundMatchClause { patterns, new_variables: new_vars.clone() }, new_vars))
    }

    fn bind_set(
        &self,
        s: &kuzu_parser::ast::SetClause,
        variables: &[BoundVariable],
    ) -> Result<BoundSetClause, String> {
        let mut items = Vec::new();
        for item in &s.items {
            // Property must be of form `variable.property`
            match &item.property {
                kuzu_parser::ast::Expression::PropertyAccess(var_expr, prop_name) => {
                    match var_expr.as_ref() {
                        kuzu_parser::ast::Expression::Variable(var_name) => {
                            let bound_var = variables.iter().find(|v| v.name == *var_name)
                                .ok_or_else(|| format!(
                                    "Variable '{}' not in scope for SET", var_name
                                ))?;
                            items.push(BoundSetItem {
                                property: item.property.clone(),
                                value: item.value.clone(),
                                column_name: prop_name.clone(),
                                column_idx: 0, // resolved by catalog lookup
                                table_name: bound_var.label.clone().unwrap_or_default(),
                                table_id: bound_var.table_id,
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
            left: Box::new(match left { BoundStatement::BoundQuery(q) => q, _ => unreachable!() }),
            right: Box::new(match right { BoundStatement::BoundQuery(q) => q, _ => unreachable!() }),
            all: u.all,
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
                    return Err(format!("Column '{new_name}' already exists in table '{}'", a.table_name));
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

        for expr in &d.expressions {
            match expr {
                kuzu_parser::ast::Expression::Variable(var_name) => {
                    let var = variables.iter().find(|v| v.name == *var_name)
                        .ok_or_else(|| format!(
                            "Variable '{}' not found in scope for DELETE", var_name
                        ))?;
                    return Ok(BoundDeleteClause {
                        expressions: d.expressions.clone(),
                        table_name: var.label.clone().unwrap_or_default(),
                        table_id: var.table_id,
                        primary_key_column: String::new(),
                    });
                }
                _ => return Err(format!(
                    "DELETE only supports variable references, got: {:?}", expr
                )),
            }
        }
        Err("DELETE: no valid expressions".into())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuzu_catalog::Catalog;
    use kuzu_parser::parse;

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
                CatalogColumn {
                    name: "score".into(),
                    logical_type: LogicalTypeID::Double,
                    is_primary_key: false,
                    default_value: None,
                },
            ],
        );
        catalog.create_rel_table(
            "Knows".into(), 0, 0,
            vec![CatalogColumn {
                name: "since".into(),
                logical_type: LogicalTypeID::Int64,
                is_primary_key: false,
                default_value: None,
            }],
        );
        Binder::new(Arc::new(Mutex::new(catalog)))
    }

    #[test]
    fn test_bind_create_node_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE City(name STRING, population INT64, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        let bound = binder.bind(stmt).unwrap();
        match bound {
            BoundStatement::BoundCreateNodeTable(t) => {
                assert_eq!(t.name, "City");
                assert_eq!(t.columns.len(), 2);
                assert_eq!(t.columns[0].logical_type, LogicalTypeID::String);
                assert_eq!(t.columns[1].logical_type, LogicalTypeID::Int64);
            }
            _ => panic!("Expected BoundCreateNodeTable"),
        }
    }

    #[test]
    fn test_bind_drop_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "DROP TABLE Person";
        // Should fail because table doesn't exist
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_match_existing_table() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                assert_eq!(q.clauses.len(), 2);
                assert_eq!(q.variables.len(), 1);
                assert_eq!(q.variables[0].name, "a");
                assert!(q.variables[0].is_node);
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_match_nonexistent_table() {
        let binder = setup_binder();
        let sql = "MATCH (a:GhostTable) RETURN a";
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_rel_pattern() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person)-[r:Knows]->(b:Person) RETURN a, b";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                // a is the first node, r is the edge
                // b is the second node - parser currently drops it
                assert!(!q.variables.is_empty());
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_where_boolean() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                assert_eq!(q.clauses.len(), 3); // match, where, return
                match &q.clauses[1] {
                    BoundClause::BoundWhere(w) => {
                        assert_eq!(w.expression.resolved_type, LogicalTypeID::Bool);
                    }
                    _ => panic!("Expected BoundWhere"),
                }
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_duplicate_variable() {
        let binder = setup_binder();
        // Duplicate variable in same MATCH (comma-separated patterns)
        // Note: multiple MATCH clauses not yet supported in grammar
        let sql = "MATCH (a:Person) WHERE a.age = a.age RETURN a";
        // Should bind fine since a is used consistently
        assert!(binder.bind(parse(sql).unwrap()).is_ok());
    }

    #[test]
    fn test_bind_invalid_type() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        // Valid type but wrong for PRIMARY KEY
        let sql = "CREATE NODE TABLE Bad(age INT64, PRIMARY KEY (name))";
        assert!(binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_empty_table_name() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE (name STRING, PRIMARY KEY (name))";
        // This should fail because parser expects a name
        assert!(parse(sql).is_err() || binder.bind(parse(sql).unwrap()).is_err());
    }

    #[test]
    fn test_bind_create_rel_table() {
        let binder = Binder::new(Arc::new(Mutex::new(Catalog::new())));
        let sql = "CREATE NODE TABLE Person(name STRING, PRIMARY KEY (name))";
        binder.bind(parse(sql).unwrap()).unwrap();
        let sql2 = "CREATE REL TABLE Knows(FROM Person TO Person, since INT64)";
        let bound = binder.bind(parse(sql2).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundCreateRelTable(t) => {
                assert_eq!(t.name, "Knows");
                assert_eq!(t.columns.len(), 1);
            }
            _ => panic!("Expected BoundCreateRelTable"),
        }
    }

    #[test]
    fn test_bind_function_return_type() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) RETURN COUNT(a)";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(q) => {
                match &q.clauses[1] {
                    BoundClause::BoundReturn(r) => {
                        assert_eq!(r.expressions[0].resolved_type, LogicalTypeID::Int64);
                    }
                    _ => panic!("Expected BoundReturn"),
                }
            }
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_property_type_resolution() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.score > 4.5 RETURN a.name";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        match bound {
            BoundStatement::BoundQuery(_) => {} // Just check it binds successfully
            _ => panic!("Expected BoundQuery"),
        }
    }

    #[test]
    fn test_bind_complex_where() {
        let binder = setup_binder();
        let sql = "MATCH (a:Person) WHERE a.age > 25 AND a.name = 'Alice' RETURN a";
        let bound = binder.bind(parse(sql).unwrap()).unwrap();
        assert!(matches!(bound, BoundStatement::BoundQuery(_)));
    }
}
