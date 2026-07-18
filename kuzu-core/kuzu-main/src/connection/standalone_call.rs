use std::sync::Arc;
use kuzu_common::types::Value;
use kuzu_common::vector::DataChunk;
use kuzu_parser::ast::Expression;
use kuzu_processor::processor::{StandaloneCallFn, StandaloneCallHandler, StandaloneCallRegistry};

use crate::database::Database;
use crate::connection::utils::{ast_constant_to_value, rows_to_datachunk};

pub struct DbStandaloneCallHandler {
    database: Arc<Database>,
    registry: StandaloneCallRegistry,
}

impl DbStandaloneCallHandler {
    pub fn new(database: Arc<Database>) -> Self {
        let mut registry = StandaloneCallRegistry::new();
        registry.register(Arc::new(ShowTablesHandler { database: database.clone() }));
        registry.register(Arc::new(TableInfoHandler { database: database.clone() }));
        registry.register(Arc::new(ShowFunctionsHandler { database: database.clone() }));
        registry.register(Arc::new(ShowIndexesHandler { database: database.clone() }));
        registry.register(Arc::new(ShowSequencesHandler { database: database.clone() }));
        registry.register(Arc::new(ShowMacrosHandler { database: database.clone() }));
        registry.register(Arc::new(ShowConnectionHandler { database: database.clone() }));
        registry.register(Arc::new(DbVersionHandler));
        registry.register(Arc::new(CatalogVersionHandler { database: database.clone() }));
        registry.register(Arc::new(CurrentSettingHandler { database: database.clone() }));
        registry.register(Arc::new(StatsInfoHandler { database: database.clone() }));
        registry.register(Arc::new(StorageInfoHandler { database: database.clone() }));
        registry.register(Arc::new(ShowAttachedDatabasesHandler));
        registry.register(Arc::new(BmInfoHandler { database: database.clone() }));
        registry.register(Arc::new(FileInfoHandler { database: database.clone() }));
        registry.register(Arc::new(FreeSpaceInfoHandler { database: database.clone() }));
        registry.register(Arc::new(DiskSizeInfoHandler { database: database.clone() }));
        registry.register(Arc::new(StorageVersionHandler));
        registry.register(Arc::new(ShowLoadedExtensionsHandler { database: database.clone() }));
        registry.register(Arc::new(ShowOfficialExtensionsHandler));
        registry.register(Arc::new(ClearWarningsHandler));
        registry.register(Arc::new(ShowWarningsHandler));
        registry.register(Arc::new(ShowProjectedGraphsHandler { database: database.clone() }));
        registry.register(Arc::new(ProjectedGraphInfoHandler { database: database.clone() }));
        registry.register(Arc::new(DropProjectedGraphHandler { database: database.clone() }));
        Self { database, registry }
    }
}

fn eval_ast_expr_to_value(expr: &Expression) -> Value {
    match expr {
        Expression::Constant(c) => ast_constant_to_value(c),
        _ => Value::Null,
    }
}

fn extract_arg_string(args: &[Expression], idx: usize) -> Result<String, String> {
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
}

impl StandaloneCallHandler for DbStandaloneCallHandler {
    fn execute_call(
        &self,
        name: &str,
        args: &[Expression],
    ) -> Result<Vec<DataChunk>, String> {
        if let Some(handler) = self.registry.get(name) {
            let result_rows = handler.execute(args)?;
            return Self::format_result(result_rows);
        }

        let args_vals: Vec<Value> = args.iter().map(eval_ast_expr_to_value).collect();
        let registry = self.database.function_registry.lock().unwrap();
        let result_rows = registry.execute_table_function(name, &args_vals)?;
        Self::format_result(result_rows)
    }
}

impl DbStandaloneCallHandler {
    fn format_result(result_rows: Vec<Vec<Value>>) -> Result<Vec<DataChunk>, String> {
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

struct ShowTablesHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowTablesHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_tables", "show tables", "list_tables", "list tables", "tables"]
    }
}

struct TableInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for TableInfoHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["table_info"]
    }
}

struct ShowFunctionsHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowFunctionsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let registry = self.database.function_registry.lock().unwrap();
        let funcs = registry.list_all();
        Ok(funcs
            .into_iter()
            .map(|(name, kind)| vec![Value::String(name), Value::String(kind)])
            .collect())
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_functions"]
    }
}

struct ShowIndexesHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowIndexesHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_indexes"]
    }
}

struct ShowSequencesHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowSequencesHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let cat = self.database.catalog.lock().unwrap();
        let seqs = cat.sequences();
        Ok(seqs
            .into_iter()
            .map(|s| vec![Value::String(s.name.clone()), Value::Int64(s.curr_val())])
            .collect())
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_sequences"]
    }
}

struct ShowMacrosHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowMacrosHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_macros"]
    }
}

struct ShowConnectionHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowConnectionHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let table_name = extract_arg_string(args, 0)?;
        let cat = self.database.catalog.lock().unwrap();
        let info = cat
            .connection_info(&table_name)
            .ok_or_else(|| format!("Table '{table_name}' not found"))?;
        Ok(vec![info])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_connection"]
    }
}

struct DbVersionHandler;

impl StandaloneCallFn for DbVersionHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let version = env!("CARGO_PKG_VERSION");
        Ok(vec![vec![Value::String(version.to_string())]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["db_version"]
    }
}

struct CatalogVersionHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for CatalogVersionHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let cat = self.database.catalog.lock().unwrap();
        let ver = cat.version();
        Ok(vec![vec![Value::Int64(ver as i64)]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["catalog_version"]
    }
}

struct CurrentSettingHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for CurrentSettingHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["current_setting"]
    }
}

struct StatsInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for StatsInfoHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
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

    fn aliases(&self) -> Vec<&'static str> {
        vec!["stats_info"]
    }
}

struct StorageInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for StorageInfoHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let sm = &self.database.storage_manager;
        let info = sm.storage_info();
        Ok(vec![vec![
            Value::String(info.db_path),
            Value::Int64(info.page_size as i64),
            Value::Int64(info.total_pages as i64),
            Value::Int64(info.free_pages as i64),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["storage_info"]
    }
}

struct ShowAttachedDatabasesHandler;

impl StandaloneCallFn for ShowAttachedDatabasesHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        Ok(vec![vec![
            Value::String("main".to_string()),
            Value::String("local".to_string()),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_attached_databases"]
    }
}

struct BmInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for BmInfoHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let bm = &self.database.storage_manager;
        let info = bm.buffer_info();
        Ok(vec![vec![
            Value::String("buffer_pool".to_string()),
            Value::Int64(info.total_memory as i64),
            Value::Int64(info.used_memory as i64),
            Value::Int64(info.num_pinned as i64),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["bm_info"]
    }
}

struct FileInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for FileInfoHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let sm = &self.database.storage_manager;
        let info = sm.file_info();
        Ok(vec![vec![
            Value::Int64(info.total_file_size as i64),
            Value::Int64(info.num_data_pages as i64),
            Value::Int64(info.wal_size as i64),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["file_info"]
    }
}

struct FreeSpaceInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for FreeSpaceInfoHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let sm = &self.database.storage_manager;
        let info = sm.fsm_info();
        Ok(vec![vec![
            Value::Int64(info.total_free_pages as i64),
            Value::Int64(info.num_entries as i64),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["free_space_info"]
    }
}

struct DiskSizeInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for DiskSizeInfoHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let sm = &self.database.storage_manager;
        let info = sm.file_info();
        Ok(vec![vec![
            Value::Int64(info.total_file_size as i64),
            Value::Int64(info.num_data_pages as i64),
            Value::Int64(info.wal_size as i64),
        ]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["disk_size_info"]
    }
}

struct StorageVersionHandler;

impl StandaloneCallFn for StorageVersionHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        Ok(vec![vec![Value::String(
            kuzu_storage::version_info::STORAGE_VERSION.to_string(),
        )]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["storage_version"]
    }
}

struct ShowLoadedExtensionsHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowLoadedExtensionsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let reg = self.database.extension_registry.lock().unwrap();
        let names: Vec<Vec<Value>> =
            reg.names().iter().map(|n| vec![Value::String(n.clone())]).collect();
        Ok(names)
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_loaded_extensions"]
    }
}

struct ShowOfficialExtensionsHandler;

impl StandaloneCallFn for ShowOfficialExtensionsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        Ok(vec![
            vec![Value::String("json".into()), Value::String("JSON functions".into())],
            vec![Value::String("fts".into()), Value::String("Full-Text Search".into())],
            vec![Value::String("vector".into()), Value::String("Vector similarity search".into())],
            vec![Value::String("httpfs".into()), Value::String("HTTP/S3 file access".into())],
            vec![Value::String("duckdb".into()), Value::String("DuckDB integration".into())],
            vec![Value::String("sqlite".into()), Value::String("SQLite integration".into())],
            vec![Value::String("postgres".into()), Value::String("PostgreSQL integration".into())],
            vec![Value::String("delta".into()), Value::String("Delta Lake integration".into())],
            vec![Value::String("iceberg".into()), Value::String("Apache Iceberg integration".into())],
            vec![Value::String("azure".into()), Value::String("Azure Blob Storage".into())],
            vec![Value::String("unity_catalog".into()), Value::String("Unity Catalog integration".into())],
            vec![Value::String("neo4j".into()), Value::String("Neo4j integration".into())],
            vec![Value::String("llm".into()), Value::String("LLM integration".into())],
            vec![Value::String("algo".into()), Value::String("Graph algorithms".into())],
        ])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_official_extensions"]
    }
}

struct ClearWarningsHandler;

impl StandaloneCallFn for ClearWarningsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        Ok(vec![vec![Value::String("Warnings cleared".into())]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["clear_warnings"]
    }
}

struct ShowWarningsHandler;

impl StandaloneCallFn for ShowWarningsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        Ok(vec![])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_warnings"]
    }
}

// ==================== Projected Graph handlers ====================

struct ShowProjectedGraphsHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ShowProjectedGraphsHandler {
    fn execute(&self, _args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let cat = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
        let graphs = cat.projected_graph_entries();
        let rows: Vec<Vec<Value>> = graphs
            .into_iter()
            .map(|g| {
                vec![
                    Value::String(g.name.clone()),
                    Value::String(g.entry_type.clone()),
                ]
            })
            .collect();
        Ok(rows)
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["show_projected_graphs"]
    }
}

struct ProjectedGraphInfoHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for ProjectedGraphInfoHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let graph_name = extract_arg_string(args, 0)?;
        let cat = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
        let info = cat
            .get_projected_graph(&graph_name)
            .ok_or_else(|| format!("Projected graph '{}' not found", graph_name))?;
        match info.entry_type.as_str() {
            "NATIVE" => {
                // NATIVE projected graph: return name, type marker
                Ok(vec![vec![
                    Value::String(info.name.clone()),
                    Value::String("NATIVE".into()),
                    Value::String("Node/rel tables defined at creation".into()),
                ]])
            }
            "CYPHER" => {
                let query = info.cypher_query.clone().unwrap_or_default();
                Ok(vec![vec![
                    Value::String(info.name.clone()),
                    Value::String("CYPHER".into()),
                    Value::String(query),
                ]])
            }
            other => Err(format!("Unknown projected graph type: {other}")),
        }
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["projected_graph_info"]
    }
}

struct DropProjectedGraphHandler {
    database: Arc<Database>,
}

impl StandaloneCallFn for DropProjectedGraphHandler {
    fn execute(&self, args: &[Expression]) -> Result<Vec<Vec<Value>>, String> {
        let graph_name = extract_arg_string(args, 0)?;
        let mut cat = self.database.catalog.lock().map_err(|e| format!("Lock error: {e}"))?;
        cat.drop_projected_graph(&graph_name)?;
        Ok(vec![vec![Value::String(format!(
            "Projected graph '{}' dropped",
            graph_name
        ))]])
    }

    fn aliases(&self) -> Vec<&'static str> {
        vec!["drop_projected_graph"]
    }
}
