use crate::{Connection, Database, QueryResult};
use akar_common::types::Value;
use arrow::datatypes::{Field, Schema};
use arrow::record_batch::RecordBatch;
use std::sync::Arc;

pub struct AdbcDatabase {
    pub db: Arc<Database>,
}

impl AdbcDatabase {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn connect(&self) -> AdbcConnection {
        AdbcConnection {
            conn: Arc::new(Connection::new(&self.db)),
        }
    }
}

pub struct AdbcConnection {
    pub conn: Arc<Connection>,
}

impl AdbcConnection {
    pub fn create_statement(&self) -> AdbcStatement {
        AdbcStatement {
            conn: self.conn.clone(),
            query: None,
        }
    }
}

pub struct AdbcStatement {
    conn: Arc<Connection>,
    query: Option<String>,
}

/// ADBC prepared statement backed by Akar's prepared-statement pipeline.
///
/// Parameters are bound positionally (ADBC `bind` semantics) and executed
/// through `Connection::prepare` + `Connection::execute` so `$name` references
/// are substituted exactly like the server/client wire path.
pub struct AdbcPreparedStatement {
    conn: Arc<Connection>,
    query: String,
    bind_values: Vec<Value>,
}

impl AdbcPreparedStatement {
    pub fn new(conn: Arc<Connection>, query: String) -> Self {
        Self {
            conn,
            query,
            bind_values: Vec::new(),
        }
    }

    /// Bind parameter values (one per `$name` parameter, in the order the
    /// parameters first appear in the query).
    ///
    /// Each value is parsed from its string form into a typed [`Value`]
    /// (bool / integer / float / fallback string). The number of bound values
    /// is validated against the prepared statement's declared parameters.
    pub fn bind(&mut self, params: Vec<String>) -> Result<(), String> {
        let prepared = self.conn.prepare(&self.query)?;
        let expected = prepared.parameter_names();
        if params.len() != expected.len() {
            return Err(format!(
                "Expected {} parameter(s) (${}), got {}",
                expected.len(),
                expected.join(", $"),
                params.len()
            ));
        }
        self.bind_values = params.into_iter().map(|p| parse_bind_value(&p)).collect();
        Ok(())
    }

    /// Execute the bound statement, returning the raw [`QueryResult`].
    pub fn execute(&self) -> Result<QueryResult, String> {
        if !self.bind_values.is_empty() {
            let prepared = self.conn.prepare(&self.query)?;
            // Bind positionally: name the parameters in declaration order.
            let names: Vec<String> = prepared.parameter_names().to_vec();
            let params: Vec<(String, Value)> = names.iter().cloned().zip(self.bind_values.iter().cloned()).collect();
            let params: Vec<(&str, Value)> = params.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
            self.conn.execute(&prepared, params).map_err(|e| e.to_string())
        } else {
            self.conn.query(&self.query).map_err(|e| e.to_string())
        }
    }
}

/// Parse a raw string bind value into a typed [`Value`].
///
/// Order of preference: `true`/`false` → Bool, integer literal → Int64,
/// floating-point literal → Double, otherwise the string itself.
fn parse_bind_value(raw: &str) -> Value {
    match raw.trim() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        other => {
            if let Ok(n) = other.parse::<i64>() {
                Value::Int64(n)
            } else if let Ok(f) = other.parse::<f64>() {
                Value::Double(f)
            } else {
                Value::String(other.to_string())
            }
        }
    }
}

pub struct AdbcPartitions {
    pub num_partitions: usize,
}

impl AdbcStatement {
    pub fn set_sql_query(&mut self, query: &str) {
        self.query = Some(query.to_string());
    }

    pub fn prepare(&self) -> Result<AdbcPreparedStatement, String> {
        let query = self.query.as_ref().ok_or("No query set")?;
        Ok(AdbcPreparedStatement::new(self.conn.clone(), query.clone()))
    }

    pub fn execute_partitions(&self) -> Result<AdbcPartitions, String> {
        // Mock implementation of partitioned execution
        Ok(AdbcPartitions { num_partitions: 1 })
    }

    pub fn execute(&self) -> Result<QueryResult, String> {
        let query = self.query.as_ref().ok_or("No query set")?;
        self.conn.query(query).map_err(|e| e.to_string())
    }

    pub fn execute_arrow(&self) -> Result<Vec<RecordBatch>, String> {
        let res = self.execute()?;
        if !res.success {
            return Err(res.error_message.unwrap_or_else(|| "Unknown error".to_string()));
        }

        if res.chunks.is_empty() {
            return Ok(Vec::new());
        }

        // The query result chunks are already column-major Arrow arrays; wrap
        // each chunk's columns into a schema'd RecordBatch (one batch per
        // chunk, preserving the chunked/streaming shape). Column names come
        // from the chunk's field metadata, falling back to `column_{i}`.
        res.chunks.iter().map(chunk_to_record_batch).collect()
    }
}

/// Wrap a single [`DataChunk`]'s Arrow columns into a [`RecordBatch`].
///
/// The Arrow data type of each column is taken directly from the underlying
/// array (correct even for all-null columns, since the array carries its own
/// type). Column names come from `field_names`, falling back to `column_{i}`.
fn chunk_to_record_batch(chunk: &akar_common::data_chunk::DataChunk) -> Result<RecordBatch, String> {
    let num_cols = chunk.num_fields();
    let fields: Vec<Field> = (0..num_cols)
        .map(|i| {
            let name = chunk
                .field_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("column_{}", i));
            // Field is nullable: query output may contain nulls in any column.
            Field::new(name, chunk.field(i).data_type().clone(), true)
        })
        .collect();
    let schema = Arc::new(Schema::new(fields));

    let arrays: Vec<arrow::array::ArrayRef> = (0..num_cols).map(|i| chunk.field(i).clone()).collect();

    RecordBatch::try_new(schema, arrays).map_err(|e| format!("Failed to construct Arrow RecordBatch: {e}"))
}

#[cfg(all(test, feature = "adbc"))]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use arrow::array::{Array, BooleanArray, Int64Array, StringArray};
    use arrow::datatypes::DataType as ArrowDataType;

    fn adbc_conn() -> (tempfile::TempDir, AdbcConnection) {
        let (dir, db, conn) = setup_db_on_disk();
        exec_ok(
            &conn,
            "CREATE NODE TABLE Person(name STRING, age INT64, active BOOL, PRIMARY KEY (name))",
        )
        .unwrap();
        exec_ok(
            &conn,
            "CREATE (:Person {name: 'Alice', age: 30, active: true}), (:Person {name: 'Bob', age: 25, active: false})",
        )
        .unwrap();
        let adbc = AdbcDatabase::new(db).connect();
        (dir, adbc)
    }

    #[test]
    fn test_execute_arrow_produces_real_batches() {
        let (_dir, adbc) = adbc_conn();
        let mut stmt = adbc.create_statement();
        stmt.set_sql_query("MATCH (p:Person) RETURN p.name, p.age, p.active ORDER BY p.name");
        let batches = stmt.execute_arrow().unwrap();

        assert!(!batches.is_empty(), "expected at least one record batch");
        let batch = &batches[0];

        // Schema carries the real column names, not a "dummy" field.
        let schema = batch.schema();
        let fields = schema.fields();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name(), "p.name");
        assert_eq!(fields[1].name(), "p.age");
        assert_eq!(fields[2].name(), "p.active");
        assert_eq!(fields[0].data_type(), &ArrowDataType::Utf8);
        assert_eq!(fields[1].data_type(), &ArrowDataType::Int64);
        assert_eq!(fields[2].data_type(), &ArrowDataType::Boolean);

        // Values are real, not an empty dummy batch.
        let names = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("column 0 to be StringArray");
        let ages = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("column 1 to be Int64Array");
        let actives = batch
            .column(2)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("column 2 to be BooleanArray");

        assert_eq!(names.len(), 2);
        assert_eq!(names.value(0), "Alice");
        assert_eq!(names.value(1), "Bob");
        assert_eq!(ages.value(0), 30i64);
        assert_eq!(ages.value(1), 25i64);
        assert_eq!(actives.value(0), true);
        assert_eq!(actives.value(1), false);
    }

    #[test]
    fn test_execute_arrow_empty_result() {
        let (_dir, adbc) = adbc_conn();
        let mut stmt = adbc.create_statement();
        stmt.set_sql_query("MATCH (p:Person) WHERE p.age > 100 RETURN p.name");
        // A 0-row match still yields a schema-typed, 0-length batch (one per
        // chunk) rather than no batches.
        let batches = stmt.execute_arrow().unwrap();
        assert_eq!(batches.len(), 1);
        let schema = batches[0].schema();
        assert_eq!(schema.fields().len(), 1);
        assert_eq!(schema.fields()[0].name(), "p.name");
        assert_eq!(batches[0].num_rows(), 0);
    }

    #[test]
    fn test_execute_arrow_ddl_no_chunks() {
        let (_dir, adbc) = adbc_conn();
        let mut stmt = adbc.create_statement();
        stmt.set_sql_query("CREATE NODE TABLE T2(x INT64, PRIMARY KEY (x))");
        // DDL has no result chunks -> empty batch list, not an error.
        let batches = stmt.execute_arrow().unwrap();
        assert!(batches.is_empty());
    }

    #[test]
    fn test_bind_and_execute_parameters() {
        let (_dir, adbc) = adbc_conn();
        let mut stmt = adbc.create_statement();
        stmt.set_sql_query("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name");
        let mut ps = stmt.prepare().unwrap();

        // Bind one value for the single $min_age parameter.
        ps.bind(vec!["26".to_string()]).unwrap();
        let res = ps.execute().unwrap();
        assert!(res.is_success(), "result should succeed");
        assert_eq!(query_arrow_names(&res), vec!["Alice"]);

        // Re-bind with a lower threshold (Bob age 25 passes > 24).
        ps.bind(vec!["24".to_string()]).unwrap();
        let res = ps.execute().unwrap();
        assert_eq!(query_arrow_names(&res), vec!["Alice", "Bob"]);
    }

    #[test]
    fn test_bind_wrong_arity_errors() {
        let (_dir, adbc) = adbc_conn();
        let mut stmt = adbc.create_statement();
        stmt.set_sql_query("MATCH (p:Person) WHERE p.age > $min_age RETURN p.name");
        let mut ps = stmt.prepare().unwrap();
        let err = ps.bind(vec!["1".to_string(), "2".to_string()]).unwrap_err();
        assert!(err.contains("Expected 1 parameter"), "unexpected error: {err}");
    }

    fn query_arrow_names(res: &QueryResult) -> Vec<String> {
        res.chunks
            .iter()
            .flat_map(|c| {
                (0..c.size).filter_map(|i| match c.get_value(0, i) {
                    Some(Value::String(s)) => Some(s),
                    _ => None,
                })
            })
            .collect()
    }
}
