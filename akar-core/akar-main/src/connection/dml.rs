use super::Connection;
use crate::connection::substitute::substitute_foreach_var;
use crate::connection::utils::ast_constant_to_value;
use crate::query_result::QueryResult;

impl Connection {
    pub(crate) fn handle_foreach(
        &self,
        fc: &akar_binder::bound_statement::BoundForeachClause,
        mut txn_opt: Option<&mut akar_transaction::Transaction>,
    ) -> Result<Option<QueryResult>, String> {
        tracing::info!("FOREACH '{}'", fc.variable);

        // Evaluate the list expression
        let list_val = match &fc.expression {
            akar_parser::ast::Expression::List(items) => {
                let mut vals = Vec::with_capacity(items.len());
                for item in items {
                    if let akar_parser::ast::Expression::Constant(c) = item {
                        vals.push(ast_constant_to_value(c));
                    } else {
                        vals.push(akar_common::types::Value::Null);
                    }
                }
                akar_common::types::Value::List(vals)
            }
            _ => {
                return Err("FOREACH requires a list expression".to_string());
            }
        };

        let list_items = match &list_val {
            akar_common::types::Value::List(items) => items.clone(),
            _ => return Ok(Some(QueryResult::success_message("FOREACH: empty list".into()))),
        };

        if list_items.is_empty() {
            return Ok(Some(QueryResult::success_message("FOREACH: empty list".into())));
        }

        // For each list item, substitute the loop variable and execute sub-statements
        for item_val in &list_items {
            for sub_stmt in &fc.sub_statements {
                // Substitute the FOREACH variable with the current item value
                let substituted = substitute_foreach_var(sub_stmt, &fc.variable, item_val)?;
                tracing::info!("FOREACH executing sub-statement for item={:?}", item_val);
                // Execute the sub-statement directly. `handle_ddl` returns
                // `Some(...)` for DDL and `None` for DML (query) statements —
                // DML must be routed through the full processor pipeline so
                // SET/DELETE/CREATE clauses actually execute.
                match self.handle_ddl(&substituted, txn_opt.as_deref_mut())? {
                    Some(result) => {
                        self.database.persist_catalog()?;
                        tracing::info!("FOREACH sub-statement result: {:?}", result);
                    }
                    None => {
                        let result = self.execute_query_inner(&substituted, txn_opt.as_deref_mut(), None)?;
                        tracing::info!("FOREACH DML sub-statement result: {:?}", result);
                    }
                }
            }
        }

        Ok(Some(QueryResult::success_message(format!(
            "FOREACH: processed {} items with {} statements",
            list_items.len(),
            fc.sub_statements.len()
        ))))
    }
}
