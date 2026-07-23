//! DML binding — MATCH, RETURN, WHERE, CREATE, DELETE, SET, MERGE, FOREACH, UNWIND, UNION, etc.

use crate::binder::helpers::resolve_set_items;
use crate::bound_statement::*;
use super::Binder;
use akar_parser::ast::*;
use std::sync::{Arc, Mutex};

impl Binder {
    // ==================== Query Binding ====================

    pub(crate) fn bind_query(&self, query: Query) -> Result<BoundStatement, String> {
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
                            let prop_access = akar_parser::ast::Expression::PropertyAccess(
                                Box::new(akar_parser::ast::Expression::Variable(node_var.clone())),
                                key.clone(),
                            );
                            let equals = akar_parser::ast::Expression::BinaryOp(
                                akar_parser::ast::BinaryOp::Equal,
                                Box::new(prop_access),
                                Box::new(val_expr.clone()),
                            );
                            inline_exprs.push(equals);
                        }
                    }
                    if let Some(edge) = &pattern.edge {
                        if let Some(edge_var) = &edge.variable {
                            for (key, val_expr) in &edge.properties {
                                let prop_access = akar_parser::ast::Expression::PropertyAccess(
                                    Box::new(akar_parser::ast::Expression::Variable(edge_var.clone())),
                                    key.clone(),
                                );
                                let equals = akar_parser::ast::Expression::BinaryOp(
                                    akar_parser::ast::BinaryOp::Equal,
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
                            akar_parser::ast::Expression::BinaryOp(
                                akar_parser::ast::BinaryOp::And,
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

    pub(crate) fn bind_match(
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

    pub(crate) fn bind_pattern(
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

    pub(crate) fn bind_return(&self, r: &ReturnClause, variables: &[BoundVariable]) -> Result<BoundReturnClause, String> {
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

        // Bind ORDER BY items
        let order_by = r.order_by.as_ref().map(|items| {
            items.iter().map(|item| {
                let resolved = self.resolve_expression(&item.expression, variables)?;
                Ok(crate::bound_statement::BoundOrderByItem {
                    expression: resolved,
                    ascending: item.ascending,
                })
            }).collect::<Result<Vec<_>, String>>()
        }).transpose()?;

        Ok(BoundReturnClause {
            expressions,
            distinct: r.distinct,
            order_by,
            limit: r.limit,
            skip: r.skip,
        })
    }

    // ==================== WHERE Binding ====================

    pub(crate) fn bind_where(&self, w: &WhereClause, variables: &[BoundVariable]) -> Result<BoundWhereClause, String> {
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

    pub(crate) fn bind_match_create(
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

    pub(crate) fn bind_unwind(&self, u: &akar_parser::ast::UnwindClause) -> Result<BoundUnwindClause, String> {
        // Validate the expression is a list literal or variable reference to a list
        match &u.expression {
            akar_parser::ast::Expression::List(_) => {}
            akar_parser::ast::Expression::Variable(_) => {}
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

    pub(crate) fn bind_foreach(
        &self,
        f: &akar_parser::ast::ForeachClause,
        variables: &[BoundVariable],
    ) -> Result<BoundForeachClause, String> {
        // Validate the expression is a list
        match &f.expression {
            akar_parser::ast::Expression::List(_) | akar_parser::ast::Expression::Variable(_) => {}
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
                akar_parser::ast::Clause::Create(cc) => {
                    // Bind as DML CREATE (BoundCreateDml), not as a MATCH clause
                    let bound = self.bind_create_dml(cc.clone(), &local_vars)?;
                    sub_statements.push(bound);
                }
                akar_parser::ast::Clause::Set(sc) => {
                    // Manually wrap SET in BoundQuery to preserve variable scope
                    let bound_set = self.bind_set(sc, &local_vars)?;
                    sub_statements.push(BoundStatement::BoundQuery(BoundQuery {
                        clauses: vec![BoundClause::BoundSet(bound_set)],
                        variables: local_vars.clone(),
                    }));
                }
                akar_parser::ast::Clause::Delete(dc) => {
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

    pub(crate) fn bind_optional_match(
        &self,
        m: &akar_parser::ast::OptionalMatchClause,
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

    pub(crate) fn bind_set(&self, s: &akar_parser::ast::SetClause, variables: &[BoundVariable]) -> Result<BoundSetClause, String> {
        let mut items = Vec::new();
        for item in &s.items {
            // Property must be of form `variable.property`
            match &item.property {
                akar_parser::ast::Expression::PropertyAccess(var_expr, prop_name) => {
                    match var_expr.as_ref() {
                        akar_parser::ast::Expression::Variable(var_name) => {
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

    pub(crate) fn bind_union(&self, u: akar_parser::ast::UnionStatement) -> Result<BoundStatement, String> {
        let left = self.bind_query(u.left)?;
        let right = self.bind_query(u.right)?;
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

    pub(crate) fn bind_merge(&self, m: akar_parser::ast::MergeStatement) -> Result<BoundStatement, String> {
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
        let properties: Vec<(String, akar_parser::ast::Expression)> = node.properties.clone();

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

    pub(crate) fn bind_create_dml(
        &self,
        c: akar_parser::ast::CreateClause,
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

    pub(crate) fn bind_delete(
        &self,
        d: &akar_parser::ast::DeleteClause,
        variables: &[BoundVariable],
    ) -> Result<BoundDeleteClause, String> {
        let mut items = Vec::new();
        for expr in &d.expressions {
            match expr {
                akar_parser::ast::Expression::Variable(var_name) => {
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

}

