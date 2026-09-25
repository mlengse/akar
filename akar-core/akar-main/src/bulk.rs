//! Typed batch writes and batched graph traversal (P123.2).
//!
//! The hot path of an agent memory engine is bulk *formation*: a turn produces
//! many memory rows and the edges between them. Writing those one parsed,
//! planned and committed Cypher statement at a time spends most of the work in
//! the parser and planner, and the plan cache cannot help because every
//! statement text differs.
//!
//! These helpers amortise that cost over a whole batch: an entire chunk goes
//! through **one** `UNWIND $rows AS r ...` statement, so the statement is parsed
//! once (the connection's statement cache keys on the statement text, which is
//! constant per call site) and planned once per chunk instead of once per row —
//! and the whole chunk commits as one transaction. Reading follows the same
//! rule: [`neighbors`] fetches the edges of many nodes in a single round trip
//! instead of one query per node.
//!
//! # What this deliberately does *not* do
//!
//! These helpers do not bypass the transaction and WAL layers by appending
//! straight to storage. That shortcut is tempting for "hot path" work, but a
//! row written outside a transaction has no WAL record and is silently lost on
//! crash — the exact defect P60.7 fixed for prepared DML. Every helper here
//! goes through the normal write path; only the parse/plan overhead is
//! amortised.
//!
//! # Edges are batched like nodes
//!
//! [`insert_edges`] writes a whole chunk of edges through **one**
//! `UNWIND $rows AS r MATCH (a), (b) MERGE (a)-[e]->(b) SET ... RETURN count(e)`
//! statement, so the parse/plan/commit cost is once per chunk instead of once
//! per edge. Resolving the `UNWIND` variable from inside a `MATCH` pattern and
//! carrying it through to the `MERGE` output required a processor fix (P131):
//! `PhysicalMergeRel` used to drop pipeline columns (including `r`) from its
//! output chunk, so a downstream `SET e.prop = r.prop` could not resolve `r`.
//! The batched shape was restored once that was fixed.
//!
//! # Missing endpoints
//!
//! [`insert_edges`] matches its endpoints by primary key. A row whose source or
//! target does not exist is **not** an error and is **not** counted as written:
//! Cypher's `MATCH ... CREATE` drops non-matching rows. Check the returned count
//! against the number of rows supplied.

use crate::connection::Connection;
use akar_common::types::Value;
use std::collections::HashMap;

/// Default number of rows packed into one `UNWIND` statement.
pub const DEFAULT_ROWS_PER_STATEMENT: usize = 1000;

/// A Rust type that maps onto one node table.
///
/// Implement this on the row struct your host wants to write; [`insert_nodes`]
/// then turns a slice of them into a single batched statement.
///
/// The contract between `COLUMNS` and [`to_values`](TypedNode::to_values) is
/// positional, and the two are checked at run time so a mismatch is an error
/// rather than a silently mis-populated row.
pub trait TypedNode {
    /// Name of the node table, as it appears in the catalog.
    const TABLE: &'static str;
    /// Property names, in the order [`to_values`](TypedNode::to_values) returns.
    const COLUMNS: &'static [&'static str];
    /// This row's property values, in `COLUMNS` order.
    fn to_values(&self) -> Vec<Value>;
}

/// How a relationship table is shaped, for [`insert_edges`] and [`neighbors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelSpec {
    /// Relationship table name.
    pub table: &'static str,
    /// Node table the edge leaves.
    pub from_table: &'static str,
    /// Node table the edge arrives at.
    pub to_table: &'static str,
    /// Primary-key property on both endpoint tables. Defaults to `id`.
    pub primary_key: &'static str,
}

impl RelSpec {
    /// Describe a relationship table whose endpoints are keyed by `id`.
    pub const fn new(table: &'static str, from_table: &'static str, to_table: &'static str) -> Self {
        Self {
            table,
            from_table,
            to_table,
            primary_key: "id",
        }
    }

    /// Describe a relationship table whose endpoints use a different key.
    pub const fn with_primary_key(mut self, primary_key: &'static str) -> Self {
        self.primary_key = primary_key;
        self
    }
}

/// One edge to write, or one edge read back.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRow {
    /// Source node's primary-key value.
    pub source: i64,
    /// Target node's primary-key value.
    pub target: i64,
    /// Property values, positionally matched against the property list given to
    /// [`insert_edges`].
    pub values: Vec<Value>,
}

impl EdgeRow {
    /// An edge with no properties beyond its endpoints.
    pub fn new(source: i64, target: i64) -> Self {
        Self {
            source,
            target,
            values: Vec::new(),
        }
    }

    /// An edge carrying `values`, positionally matched against the property
    /// list passed to [`insert_edges`].
    pub fn with_values(source: i64, target: i64, values: Vec<Value>) -> Self {
        Self { source, target, values }
    }
}

/// An edge returned by [`neighbors`].
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor {
    /// Source node's primary-key value.
    pub source: i64,
    /// Target node's primary-key value.
    pub target: i64,
    /// The edge's weight, when a weight property was requested.
    pub weight: Option<f64>,
}

/// Write `rows` into `T::TABLE` using one batched statement per chunk.
///
/// Rows are split into chunks of `rows_per_statement` so a large batch does not
/// build an unbounded statement; each chunk is parsed, bound and planned once.
/// Because the statement uses `CREATE`, a row whose primary key already exists
/// fails the chunk — use the primary key to make writes idempotent only if the
/// table has no such row.
///
/// Returns the number of rows the engine reported writing.
///
/// # Errors
///
/// Returns an error when `rows_per_statement` is zero, when a row's
/// [`TypedNode::to_values`] length disagrees with [`TypedNode::COLUMNS`], or
/// when the engine rejects a chunk.
pub fn insert_nodes<T: TypedNode>(conn: &Connection, rows: &[T], rows_per_statement: usize) -> Result<usize, String> {
    if rows_per_statement == 0 {
        return Err("rows_per_statement must be greater than zero".into());
    }
    if rows.is_empty() {
        return Ok(0);
    }

    let statement = create_nodes_statement::<T>();
    let mut written = 0;
    for chunk in rows.chunks(rows_per_statement) {
        let payload = encode_chunk::<T>(chunk)?;
        let result = conn.execute_params(&statement, vec![("rows", payload)])?;
        written += written_count(&result)?;
    }
    Ok(written)
}

/// Write `edges` into `rel.table` using one batched `UNWIND` statement per chunk.
///
/// `properties` names the relationship properties positionally matched against
/// each row's [`EdgeRow::values`]; pass an empty slice for a property-less
/// relationship table.
///
/// The statement uses `MERGE` on the endpoint pattern, so writing the same edge
/// twice updates it in place rather than duplicating it.
///
/// Returns the number of edges the engine reported writing. An edge whose
/// endpoint does not exist is dropped by the `MATCH` and is not counted (see
/// the module docs).
///
/// Rows are split into chunks of [`DEFAULT_ROWS_PER_STATEMENT`]; call
/// [`insert_edges_chunked`] to size the chunks explicitly.
///
/// # Errors
///
/// Returns an error when `properties` and a row's value count disagree, or when
/// the engine rejects a chunk.
pub fn insert_edges(conn: &Connection, rel: &RelSpec, properties: &[&str], edges: &[EdgeRow]) -> Result<usize, String> {
    insert_edges_chunked(conn, rel, properties, edges, DEFAULT_ROWS_PER_STATEMENT)
}

/// Write `edges` into `rel.table`, `rows_per_statement` edges per statement.
///
/// Same semantics as [`insert_edges`] with explicit chunk sizing, mirroring
/// [`insert_nodes`]: a large batch is split into chunks so no statement grows
/// without bound, and each chunk is parsed, bound, planned and committed once.
/// The reduction shows up as statements per batch — one per chunk, not one per
/// edge — observable through [`Connection::executed_statements`].
///
/// # Errors
///
/// Returns an error when `rows_per_statement` is zero, when `properties` and a
/// row's value count disagree, or when the engine rejects a chunk.
pub fn insert_edges_chunked(
    conn: &Connection,
    rel: &RelSpec,
    properties: &[&str],
    edges: &[EdgeRow],
    rows_per_statement: usize,
) -> Result<usize, String> {
    if rows_per_statement == 0 {
        return Err("rows_per_statement must be greater than zero".into());
    }
    if edges.is_empty() {
        return Ok(0);
    }

    let statement = merge_edges_batch_statement(rel, properties);
    let mut written = 0;
    for chunk in edges.chunks(rows_per_statement) {
        let payload = encode_edge_chunk(chunk, properties)?;
        let result = conn.execute_params(&statement, vec![("rows", payload)])?;
        written += written_count(&result)?;
    }
    Ok(written)
}

/// Fetch the outgoing edges of `node_ids` in one round trip.
///
/// `weight_property` names the relationship property to return and to order by
/// (Sulur's `Connected.weight`); pass `None` for a property-less relationship
/// table, in which case the result order is the engine's.
///
/// `limit_per_node` truncates the neighbours of each node after the descending
/// weight sort, so a hub node cannot flood the caller. Pass `0` for no limit.
/// `node_ids` is de-duplicated before the query.
///
/// # Errors
///
/// Returns an error when `rows_per_statement`/`node_ids` cannot form a valid
/// query, or when the engine rejects the lookup.
pub fn neighbors(
    conn: &Connection,
    rel: &RelSpec,
    weight_property: Option<&str>,
    node_ids: &[i64],
    limit_per_node: usize,
    rows_per_statement: usize,
) -> Result<Vec<Neighbor>, String> {
    if rows_per_statement == 0 {
        return Err("rows_per_statement must be greater than zero".into());
    }
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut unique: Vec<i64> = node_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();

    let statement = neighbors_statement(rel, weight_property);
    let mut neighbors: Vec<Neighbor> = Vec::new();
    for chunk in unique.chunks(rows_per_statement) {
        let payload = Value::List(chunk.iter().map(|id| Value::Int64(*id)).collect());
        let result = conn.execute_params(&statement, vec![("ids", payload)])?;
        neighbors.extend(decode_neighbors(&result, weight_property.is_some())?);
    }

    if limit_per_node == 0 {
        return Ok(neighbors);
    }

    // The order within one source's neighbours is the engine's descending
    // weight sort; keep the first `limit_per_node` of each.
    let mut seen: HashMap<i64, usize> = HashMap::new();
    let mut limited: Vec<Neighbor> = Vec::with_capacity(neighbors.len());
    for neighbor in neighbors {
        let count = seen.entry(neighbor.source).or_insert(0);
        if *count < limit_per_node {
            *count += 1;
            limited.push(neighbor);
        }
    }
    Ok(limited)
}

/// `${COLUMNS}` -> `r.${COLUMNS}` projection list for a batched CREATE, plus a
/// `RETURN count(...)` so the caller learns the real write count.
fn create_nodes_statement<T: TypedNode>() -> String {
    let table = T::TABLE;
    let properties = T::COLUMNS
        .iter()
        .map(|column| format!("{column}: r.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("UNWIND $rows AS r CREATE (n:{table} {{{properties}}}) RETURN count(n) AS written")
}

/// `UNWIND $rows AS r MATCH ... MERGE (a)-[e:REL]->(b) SET ... RETURN count(e)`
/// for an entire chunk of edges.
fn merge_edges_batch_statement(rel: &RelSpec, properties: &[&str]) -> String {
    let key = rel.primary_key;
    let set_clause = if properties.is_empty() {
        String::new()
    } else {
        let assignments = properties
            .iter()
            .map(|property| format!("e.{property} = r.{property}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" SET {assignments}")
    };
    format!(
        "UNWIND $rows AS r MATCH (a:{from} {{{key}: r.source}}), (b:{to} {{{key}: r.target}}) \
         MERGE (a)-[e:{table}]->(b){set_clause} RETURN count(e) AS written",
        from = rel.from_table,
        to = rel.to_table,
        table = rel.table,
    )
}

/// `UNWIND $ids AS iid MATCH (a)-[e]->(b) RETURN ...` for a batched lookup.
fn neighbors_statement(rel: &RelSpec, weight_property: Option<&str>) -> String {
    let key = rel.primary_key;
    let weight_projection = match weight_property {
        Some(property) => format!(", e.{property} AS weight"),
        None => String::new(),
    };
    // Order by the *projected aliases*: Akar resolves ORDER BY against the
    // output chunk, so referring to `a.id` here fails with
    // "Variable 'a' not found in chunk field_names".
    let order_by = match weight_property {
        Some(property) => format!(" ORDER BY source, e.{property} DESC"),
        None => " ORDER BY source".to_string(),
    };
    format!(
        "UNWIND $ids AS iid MATCH (a:{from} {{{key}: iid}})-[e:{table}]->(b:{to}) \
         RETURN a.{key} AS source, b.{key} AS target{weight_projection}{order_by}",
        from = rel.from_table,
        to = rel.to_table,
        table = rel.table,
    )
}

/// Encode typed rows as the `$rows` list-of-structs parameter.
fn encode_chunk<T: TypedNode>(rows: &[T]) -> Result<Value, String> {
    let expected = T::COLUMNS.len();
    let mut encoded = Vec::with_capacity(rows.len());
    for row in rows {
        let values = row.to_values();
        if values.len() != expected {
            return Err(format!(
                "{}::to_values returned {} values for {} columns",
                T::TABLE,
                values.len(),
                expected
            ));
        }
        let fields = T::COLUMNS
            .iter()
            .zip(values)
            .map(|(column, value)| ((*column).to_string(), value))
            .collect();
        encoded.push(Value::Struct(fields));
    }
    Ok(Value::List(encoded))
}

/// Encode a chunk of edges as the `$rows` list-of-structs parameter.
///
/// Each struct carries the endpoints under `source`/`target` (so the `MATCH`
/// patterns can reference them) plus the given property names positionally,
/// matching `e.{property} = r.{property}` in the set clause.
fn encode_edge_chunk(edges: &[EdgeRow], properties: &[&str]) -> Result<Value, String> {
    let mut encoded = Vec::with_capacity(edges.len());
    for edge in edges {
        if edge.values.len() != properties.len() {
            return Err(format!(
                "edge ({} -> {}) carries {} values for {} properties",
                edge.source,
                edge.target,
                edge.values.len(),
                properties.len()
            ));
        }
        let mut fields = Vec::with_capacity(properties.len() + 2);
        fields.push(("source".to_string(), Value::Int64(edge.source)));
        fields.push(("target".to_string(), Value::Int64(edge.target)));
        for (property, value) in properties.iter().zip(edge.values.iter()) {
            fields.push(((*property).to_string(), value.clone()));
        }
        encoded.push(Value::Struct(fields));
    }
    Ok(Value::List(encoded))
}

/// Read the `written` column the batched statements return.
///
/// The count comes from the engine (`RETURN count(...)`) rather than from
/// parsing the result message, so a batch that created fewer rows than it was
/// given — every edge whose endpoint is missing — reports the truth.
fn written_count(result: &crate::QueryResult) -> Result<usize, String> {
    let chunk = result
        .chunks
        .first()
        .ok_or_else(|| "batched write returned no chunk".to_string())?;
    let index = column_index(chunk, "written")?;
    let value = chunk
        .get_value(index, 0)
        .ok_or_else(|| "batched write returned a NULL count".to_string())?;
    match value {
        Value::Int64(count) => Ok(count.max(0) as usize),
        Value::UInt64(count) => Ok(count as usize),
        other => Err(format!("batched write returned a non-integer count: {other:?}")),
    }
}

/// Pull the `source`/`target`/`weight` columns out of a neighbour result.
fn decode_neighbors(result: &crate::QueryResult, with_weight: bool) -> Result<Vec<Neighbor>, String> {
    let chunk = match result.chunks.first() {
        Some(chunk) => chunk,
        None => return Ok(Vec::new()),
    };
    let source_index = column_index(chunk, "source")?;
    let target_index = column_index(chunk, "target")?;
    let weight_index = if with_weight {
        Some(column_index(chunk, "weight")?)
    } else {
        None
    };

    let mut neighbors = Vec::with_capacity(chunk.size);
    for row in 0..chunk.size {
        let source = as_i64(chunk.get_value(source_index, row), "source")?;
        let target = as_i64(chunk.get_value(target_index, row), "target")?;
        let weight = match weight_index {
            Some(index) => match chunk.get_value(index, row) {
                Some(Value::Double(value)) => Some(value),
                Some(Value::Float(value)) => Some(value as f64),
                Some(Value::Int64(value)) => Some(value as f64),
                _ => None,
            },
            None => None,
        };
        neighbors.push(Neighbor { source, target, weight });
    }
    Ok(neighbors)
}

/// Locate a column by name, so a change in projection order fails loudly.
fn column_index(chunk: &akar_common::vector::DataChunk, name: &str) -> Result<usize, String> {
    chunk
        .field_names
        .iter()
        .position(|field| field == name)
        .ok_or_else(|| format!("result is missing the '{name}' column (got {:?})", chunk.field_names))
}

fn as_i64(value: Option<Value>, column: &str) -> Result<i64, String> {
    match value {
        Some(Value::Int64(value)) => Ok(value),
        Some(Value::UInt64(value)) => Ok(value as i64),
        other => Err(format!("column '{column}' is not an integer: {other:?}")),
    }
}
