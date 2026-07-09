use std::sync::Arc;
use kuzu_common::vector::DataChunk;
use kuzu_common::types::Value;
use kuzu_parser::ast::Expression;
use kuzu_processor::processor::StandaloneCallHandler;

use crate::database::Database;
use crate::connection::utils::rows_to_datachunk;

pub struct DbStandaloneCallHandler {
    pub database: Arc<Database>,
}

impl StandaloneCallHandler for DbStandaloneCallHandler {
    fn execute_call(
        &self,
        name: &str,
        args: &[Expression],
    ) -> Result<Vec<DataChunk>, String> {
        let fn_lower = name.to_lowercase();
        
        let eval_ast_expr_to_value = |expr: &Expression| -> Value {
            match expr {
                Expression::Constant(c) => crate::connection::utils::ast_constant_to_value(c),
                _ => Value::Null,
            }
        };
        
        let extract_arg_string = |args: &[Expression], idx: usize| -> Result<String, String> {
            if idx >= args.len() {
                return Err(format!("Expected argument at index {}", idx));
            }
            match &args[idx] {
                Expression::Constant(c) => match c {
                    kuzu_parser::ast::Constant::String(s) => Ok(s.clone()),
                    _ => Err("Expected string argument".into()),
                },
                _ => Err("Expected string constant argument".into()),
            }
        };

        let result: Result<Vec<Vec<Value>>, String> = match fn_lower.as_str() {
            "show_tables" | "show tables" | "list_tables" | "list tables" | "tables" => {
                let catalog = self.database.catalog.lock().unwrap();
                let entries: Vec<Vec<Value>> = catalog
                    .all_entries()
                    .map(|e| {
                        let kind = match e {
                            kuzu_catalog::CatalogEntry::NodeTable(_) => "NODE",
                            kuzu_catalog::CatalogEntry::RelTable(_) => "REL",
                            kuzu_catalog::CatalogEntry::Sequence(_) => "SEQUENCE",
                            kuzu_catalog::CatalogEntry::Macro(_) => "MACRO",
                            kuzu_catalog::CatalogEntry::VectorIndex(_) => "VECTOR_INDEX",
                            kuzu_catalog::CatalogEntry::Foreign(_) => "FOREIGN",
                        };
                        vec![Value::String(e.name().to_string()), Value::String(kind.to_string())]
                    })
                    .collect();
                Ok(entries)
            }
            "table_info" => {
                let table_name = extract_arg_string(args, 0)?;
                let cat = self.database.catalog.lock().unwrap();
                let entry = cat
                    .get_entry_by_name(&table_name)
                    .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                let columns = entry.columns();
                let rows: Vec<Vec<Value>> = columns
                    .iter()
                    .map(|col| {
                        vec![
                            Value::String(table_name.clone()),
                            Value::String(col.name.clone()),
                            Value::String(format!("{:?}", col.logical_type)),
                            Value::String(if col.is_primary_key { "NO" } else { "YES" }.into()),
                        ]
                    })
                    .collect();
                Ok(rows)
            }
            "show_functions" => {
                let registry = self.database.function_registry.lock().unwrap();
                let funcs = registry.list_all();
                Ok(funcs
                    .into_iter()
                    .map(|(name, kind)| vec![Value::String(name), Value::String(kind)])
                    .collect())
            }
            "show_indexes" => {
                let cat = self.database.catalog.lock().unwrap();
                let indexes = cat.indexes();
                Ok(indexes
                    .into_iter()
                    .map(|(name, table, kind, col)| {
                        vec![
                            Value::String(name),
                            Value::String(table),
                            Value::String(kind),
                            Value::String(col),
                        ]
                    })
                    .collect())
            }
            "show_sequences" => {
                let cat = self.database.catalog.lock().unwrap();
                let seqs = cat.sequences();
                Ok(seqs
                    .into_iter()
                    .map(|s| vec![Value::String(s.name.clone()), Value::Int64(s.curr_val())])
                    .collect())
            }
            "show_macros" => {
                let cat = self.database.catalog.lock().unwrap();
                let macros = cat.macros();
                Ok(macros
                    .into_iter()
                    .map(|m| {
                        vec![
                            Value::String(m.name.clone()),
                            Value::String(
                                m.default_args
                                    .iter()
                                    .map(|(k, v)| format!("{k}={v}"))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ),
                        ]
                    })
                    .collect())
            }
            "show_connection" => {
                let table_name = extract_arg_string(args, 0)?;
                let cat = self.database.catalog.lock().unwrap();
                let info = cat
                    .connection_info(&table_name)
                    .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                Ok(vec![info])
            }
            "db_version" => {
                let version = env!("CARGO_PKG_VERSION");
                Ok(vec![vec![Value::String(version.to_string())]])
            }
            "catalog_version" => {
                let cat = self.database.catalog.lock().unwrap();
                let ver = cat.version();
                Ok(vec![vec![Value::Int64(ver as i64)]])
            }
            "current_setting" => {
                let key = extract_arg_string(args, 0).unwrap_or_else(|_| String::new());
                let (k, v) = match key.to_lowercase().as_str() {
                    "spill_threshold" => (
                        "spill_threshold",
                        self.database.effective_spill_threshold().to_string(),
                    ),
                    "checkpoint_threshold" => (
                        "checkpoint_threshold",
                        self.database.config.checkpoint_threshold.to_string(),
                    ),
                    "buffer_pool_size" => (
                        "buffer_pool_size",
                        self.database.config.buffer_pool_size.to_string(),
                    ),
                    "max_num_threads" => ("max_num_threads", self.database.config.max_num_threads.to_string()),
                    "concurrent_writes" => (
                        "concurrent_writes",
                        self.database.config.concurrent_writes.to_string(),
                    ),
                    "read_only" => ("read_only", self.database.config.read_only.to_string()),
                    _ => (key.as_str(), "UNKNOWN".to_string()),
                };
                Ok(vec![vec![Value::String(k.to_string()), Value::String(v)]])
            }
            "stats_info" => {
                let table_name = extract_arg_string(args, 0)?;
                let (row_count, storage_size) = {
                    let cat = self.database.catalog.lock().unwrap();
                    let table_id = cat
                        .get_table_id(&table_name)
                        .ok_or_else(|| format!("Table '{table_name}' not found"))?;
                    let stats = self.database.stats_store.lock().unwrap();
                    stats.table_stats_by_id(table_id)
                };
                Ok(vec![vec![
                    Value::String(table_name),
                    Value::Int64(row_count as i64),
                    Value::String(crate::connection::utils::format_storage_size(storage_size)),
                ]])
            }
            "storage_info" => {
                let sm = &self.database.storage_manager;
                let info = sm.storage_info();
                Ok(vec![vec![
                    Value::String(info.db_path),
                    Value::Int64(info.page_size as i64),
                    Value::Int64(info.total_pages as i64),
                    Value::Int64(info.free_pages as i64),
                ]])
            }
            "show_attached_databases" => Ok(vec![vec![
                Value::String("main".to_string()),
                Value::String("local".to_string()),
            ]]),
            "bm_info" => {
                let bm = &self.database.storage_manager;
                let info = bm.buffer_info();
                Ok(vec![vec![
                    Value::String("buffer_pool".to_string()),
                    Value::Int64(info.total_memory as i64),
                    Value::Int64(info.used_memory as i64),
                    Value::Int64(info.num_pinned as i64),
                ]])
            }
            "file_info" => {
                let sm = &self.database.storage_manager;
                let info = sm.file_info();
                Ok(vec![vec![
                    Value::Int64(info.total_file_size as i64),
                    Value::Int64(info.num_data_pages as i64),
                    Value::Int64(info.wal_size as i64),
                ]])
            }
            "free_space_info" => {
                let sm = &self.database.storage_manager;
                let info = sm.fsm_info();
                Ok(vec![vec![
                    Value::Int64(info.total_free_pages as i64),
                    Value::Int64(info.num_entries as i64),
                ]])
            }
            "disk_size_info" => {
                let sm = &self.database.storage_manager;
                let info = sm.file_info();
                Ok(vec![vec![
                    Value::Int64(info.total_file_size as i64),
                    Value::Int64(info.num_data_pages as i64),
                    Value::Int64(info.wal_size as i64),
                ]])
            }
            "storage_version" => Ok(vec![vec![Value::String(
                kuzu_storage::version_info::STORAGE_VERSION.to_string(),
            )]]),
            "show_loaded_extensions" => {
                let reg = self.database.extension_registry.lock().unwrap();
                let names: Vec<Vec<Value>> =
                    reg.names().iter().map(|n| vec![Value::String(n.clone())]).collect();
                Ok(names)
            }
            "show_official_extensions" => Ok(vec![
                vec![Value::String("json".into()), Value::String("JSON functions".into())],
                vec![
                    Value::String("fts".into()),
                    Value::String("Full-Text Search".into()),
                ],
                vec![
                    Value::String("vector".into()),
                    Value::String("Vector similarity search".into()),
                ],
                vec![
                    Value::String("httpfs".into()),
                    Value::String("HTTP/S3 file access".into()),
                ],
                vec![
                    Value::String("duckdb".into()),
                    Value::String("DuckDB integration".into()),
                ],
                vec![
                    Value::String("sqlite".into()),
                    Value::String("SQLite integration".into()),
                ],
                vec![
                    Value::String("postgres".into()),
                    Value::String("PostgreSQL integration".into()),
                ],
                vec![
                    Value::String("delta".into()),
                    Value::String("Delta Lake integration".into()),
                ],
                vec![
                    Value::String("iceberg".into()),
                    Value::String("Apache Iceberg integration".into()),
                ],
                vec![
                    Value::String("azure".into()),
                    Value::String("Azure Blob Storage".into()),
                ],
                vec![
                    Value::String("unity_catalog".into()),
                    Value::String("Unity Catalog integration".into()),
                ],
                vec![
                    Value::String("neo4j".into()),
                    Value::String("Neo4j integration".into()),
                ],
                vec![Value::String("llm".into()), Value::String("LLM integration".into())],
                vec![
                    Value::String("algo".into()),
                    Value::String("Graph algorithms".into()),
                ],
            ]),
            "clear_warnings" => {
                Ok(vec![vec![Value::String("Warnings cleared".into())]])
            }
            "show_warnings" => {
                Ok(vec![])
            }
            "page_rank" | "pr" | "wcc" | "weakly_connected_components"
            | "scc" | "strongly_connected_components" | "k_core" | "kcore"
            | "louvain" | "spanning_forest" | "sf"
            | "shortest_path" | "sp" | "weighted_shortest_path" => {
                let args_vals: Vec<Value> = args.iter().map(eval_ast_expr_to_value).collect();
                let registry = self.database.function_registry.lock().unwrap();
                registry.execute_table_function(name, &args_vals)
            }
            _ => {
                let args_vals: Vec<Value> = args.iter().map(eval_ast_expr_to_value).collect();
                let registry = self.database.function_registry.lock().unwrap();
                registry.execute_table_function(name, &args_vals)
            }
        };

        let result_rows = result?;

        // Format result rows to a single column string to match previous API behavior for StandaloneCall,
        // or just build a DataChunk if not empty.
        if result_rows.is_empty() {
            Ok(vec![])
        } else {
            let num_cols = result_rows[0].len();
            let col_names_strings = (0..num_cols).map(|i| format!("col_{}", i)).collect::<Vec<_>>();
            let col_names = col_names_strings.iter().map(|s| s.as_str()).collect::<Vec<_>>();
            Ok(vec![rows_to_datachunk(result_rows, &col_names)])
        }
    }
}
