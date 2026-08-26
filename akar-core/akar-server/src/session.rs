//! Client session handling — one thread per TCP connection (P47.2).
//!
//! Each client gets its own [`Connection`] to the shared [`Database`]. Requests
//! are length-prefixed JSON frames; the session loop reads a frame, executes the
//! query through the normal `Connection::query` pipeline (which wraps writes in
//! OCC transactions and surfaces `WriteConflict` as errors), and replies with a
//! serialized [`WireResponse`].
//!
//! P62 adds:
//! - **Auth token validation** — first frame must carry the correct token.
//! - **Operation dispatch** — `ping`, `flush`, `stats`, `export`, `shutdown`.
//! - **Idle tracking** — `last_activity` timestamp for the server idle monitor.

use akar_common::types::{PhysicalTypeID, Value};
use akar_main::connection::Connection;
use akar_main::database::Database;
use akar_main::query_result::QueryResult;
use akar_main::remote::{PartialFrame, ServerStats, WireRequest, WireResponse, read_frame, write_frame};
use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the session waits for the next frame before re-checking the server
/// shutdown flag. A slow client is never disconnected; this only bounds the
/// shutdown detection latency.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// How long a response write may block before the session gives up. Bounds how
/// long [`Server::shutdown`](crate::Server::shutdown) waits for a client that
/// stops reading its responses.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Session configuration for P62 features.
pub struct SessionConfig {
    /// Required auth token. If `None`, auth is disabled.
    pub auth_token: Option<String>,
    /// Shared atomic for recording the last activity timestamp (epoch seconds).
    /// The server idle monitor reads this to detect idleness.
    pub last_activity: Arc<AtomicU64>,
    /// Shared atomic for total query counter (incremented on each `query` op).
    pub total_queries: Arc<AtomicU64>,
    /// Database path (for stats response).
    pub db_path: String,
    /// Shared flag to trigger server shutdown (set by `shutdown` op).
    pub shutdown: Arc<AtomicBool>,
}

/// Serve one client connection until the peer disconnects or the server shuts
/// down. All per-session state is dropped on return.
pub fn handle_client(mut stream: TcpStream, db: Arc<Database>, config: &SessionConfig) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let conn = Connection::new(&db);
    let mut partial: Option<PartialFrame> = None;
    let mut authenticated = config.auth_token.is_none(); // auto-auth if no token

    loop {
        if config.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let frame = match read_frame(&mut stream, &mut partial) {
            Ok(Some(f)) => f,
            Ok(None) => return, // clean EOF — client disconnected
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                continue;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return,
        };

        let request: WireRequest = match serde_json::from_slice(&frame) {
            Ok(r) => r,
            Err(_) => {
                let resp = WireResponse::error("Malformed request frame".to_string());
                if let Ok(payload) = serde_json::to_vec(&resp) {
                    if write_frame(&mut stream, &payload).is_err() {
                        return;
                    }
                }
                continue;
            }
        };

        // Auth check on first request.
        if !authenticated {
            match &config.auth_token {
                Some(expected) => {
                    match &request.token {
                        Some(provided) if provided == expected => {
                            authenticated = true;
                        }
                        _ => {
                            let resp =
                                WireResponse::error("Authentication required: invalid or missing token".to_string());
                            if let Ok(payload) = serde_json::to_vec(&resp) {
                                let _ = write_frame(&mut stream, &payload);
                            }
                            return; // disconnect after auth failure
                        }
                    }
                }
                None => {
                    authenticated = true;
                }
            }
        }

        // Update last activity timestamp.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        config.last_activity.store(now, Ordering::Relaxed);

        let response = match request.op.as_deref() {
            Some("ping") => handle_ping(),
            Some("flush") => handle_flush(&conn),
            Some("stats") => handle_stats(&config.db_path, &config.total_queries),
            Some("export") => handle_export(&conn, request.path.as_deref()),
            Some("shutdown") => handle_shutdown(&config.shutdown),
            None | Some("query") => {
                config.total_queries.fetch_add(1, Ordering::Relaxed);
                execute_query(&conn, &request)
            }
            Some(op) => serialize_response(&WireResponse::error(format!("Unknown operation: {op}"))),
        };

        if write_frame(&mut stream, &response).is_err() {
            return; // client disconnected mid-reply
        }
    }
}

/// Liveness check — always succeeds.
fn handle_ping() -> Vec<u8> {
    let resp = WireResponse::success_message("pong".to_string());
    serialize_response(&resp)
}

/// Force a CHECKPOINT to persist all data to disk.
fn handle_flush(conn: &Connection) -> Vec<u8> {
    let resp = match conn.query("CHECKPOINT") {
        Ok(_) => WireResponse::success_message("Flushed (checkpoint committed)".to_string()),
        Err(e) => WireResponse::error(format!("Flush failed: {e}")),
    };
    serialize_response(&resp)
}

/// Return server statistics.
fn handle_stats(db_path: &str, total_queries: &Arc<AtomicU64>) -> Vec<u8> {
    let stats = ServerStats {
        num_clients: 0, // full count requires client_handles; left as 0 for now
        total_queries: total_queries.load(Ordering::Relaxed),
        uptime_secs: 0,
        db_path: db_path.to_string(),
        pid: std::process::id(),
    };
    let resp = WireResponse {
        success: true,
        message: None,
        error_message: None,
        column_names: Vec::new(),
        rows: Vec::new(),
        stats: Some(stats),
    };
    serialize_response(&resp)
}

/// Export the database to the given path.
fn handle_export(conn: &Connection, path: Option<&str>) -> Vec<u8> {
    let path = match path {
        Some(p) if !p.is_empty() => p,
        _ => {
            return serialize_response(&WireResponse::error(
                "Export requires a non-empty 'path' field".to_string(),
            ));
        }
    };
    let query = format!("EXPORT DATABASE '{path}'");
    let resp = match conn.query(&query) {
        Ok(result) => execute_query_result_to_wire(&result),
        Err(e) => WireResponse::error(format!("Export failed: {e}")),
    };
    serialize_response(&resp)
}

/// Request graceful server shutdown.
fn handle_shutdown(shutdown_flag: &Arc<AtomicBool>) -> Vec<u8> {
    shutdown_flag.store(true, Ordering::SeqCst);
    let resp = WireResponse::success_message("Shutdown requested".to_string());
    serialize_response(&resp)
}

/// Run a single query against the session's connection and serialize the result.
///
/// When the request carries `params`, the query is executed through the
/// prepared-statement pipeline with parameter substitution.  Otherwise the
/// plain string query path is used.
fn execute_query(conn: &Connection, request: &WireRequest) -> Vec<u8> {
    let wire = match &request.params {
        Some(params) if !params.is_empty() => execute_parameterized_query(conn, request, params),
        _ => match conn.query(&request.query) {
            Ok(result) => query_result_to_wire(&result),
            Err(e) => WireResponse::error(e),
        },
    };
    serialize_response(&wire)
}

/// Convert a JSON value to an Akar [`Value`] for parameter binding.
///
/// JSON numbers are mapped to `Int64` when they fit, `Double` otherwise.
/// JSON strings, booleans, and null map directly.
fn json_value_to_akar_value(val: &serde_json::Value) -> Result<Value, String> {
    match val {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int64(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Double(f))
            } else {
                Err(format!("JSON number {n} is not a valid i64 or f64"))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(_) => Err("Array parameters are not supported".into()),
        serde_json::Value::Object(_) => Err("Object parameters are not supported".into()),
    }
}

/// Execute a query with parameter binding via the prepared-statement pipeline.
fn execute_parameterized_query(
    conn: &Connection,
    request: &WireRequest,
    params: &HashMap<String, serde_json::Value>,
) -> WireResponse {
    // Convert JSON params → Akar Values
    let mut akar_params: Vec<(&str, Value)> = Vec::with_capacity(params.len());
    for (name, json_val) in params {
        match json_value_to_akar_value(json_val) {
            Ok(v) => akar_params.push((name.as_str(), v)),
            Err(e) => return WireResponse::error(format!("Parameter '{name}': {e}")),
        }
    }

    // Prepare the query
    let prepared = match conn.prepare(&request.query) {
        Ok(p) => p,
        Err(e) => return WireResponse::error(format!("Prepare error: {e}")),
    };

    // Execute with bound parameters
    match conn.execute(&prepared, akar_params) {
        Ok(result) => query_result_to_wire(&result),
        Err(e) => WireResponse::error(e),
    }
}

/// Serialize a `WireResponse` to JSON bytes.
fn serialize_response(resp: &WireResponse) -> Vec<u8> {
    serde_json::to_vec(resp).unwrap_or_else(|_| {
        br#"{"success":false,"error_message":"response serialization failure","column_names":[],"rows":[]}"#.to_vec()
    })
}

/// Convert a `QueryResult` to a `WireResponse` and serialize it.
fn execute_query_result_to_wire(result: &QueryResult) -> WireResponse {
    query_result_to_wire(result)
}

/// Convert a `QueryResult` into a row-major [`WireResponse`].
fn query_result_to_wire(result: &QueryResult) -> WireResponse {
    let mut column_names: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Option<Value>>> = Vec::new();

    for chunk in &result.chunks {
        if column_names.is_empty() && chunk.num_fields() > 0 {
            column_names = if chunk.field_names.len() == chunk.num_fields() {
                chunk.field_names.clone()
            } else {
                (0..chunk.num_fields()).map(|i| format!("col{i}")).collect()
            };
        }
        for row_idx in chunk.iter_rows() {
            let mut row = Vec::with_capacity(chunk.num_fields());
            for col_idx in 0..chunk.num_fields() {
                // Defensive bounds checks: a malformed chunk (fields /
                // field_types / row length disagreement) must never panic the
                // session thread — render the offending cell instead.
                if col_idx >= chunk.fields.len()
                    || col_idx >= chunk.field_types.len()
                    || row_idx >= chunk.fields[col_idx].len()
                {
                    row.push(Some(Value::String("<malformed chunk>".to_string())));
                    continue;
                }
                let cell = if chunk.is_null(col_idx, row_idx) {
                    None
                } else {
                    cell_value(chunk.field(col_idx), chunk.field_types[col_idx], row_idx)
                };
                row.push(cell);
            }
            rows.push(row);
        }
    }

    WireResponse {
        success: true,
        message: result.message.clone(),
        error_message: None,
        column_names,
        rows,
        stats: None,
    }
}

/// Convert one Arrow cell into an Akar [`Value`] based on its physical type.
///
/// Falls back to a `Debug` string representation for types without a dedicated
/// mapping, so unexpected cells are never silently lost on the wire.
fn cell_value(field: &ArrayRef, field_type: PhysicalTypeID, row: usize) -> Option<Value> {
    let arr = field.as_ref();
    let value = match field_type {
        PhysicalTypeID::Bool => arr
            .as_any()
            .downcast_ref::<BooleanArray>()
            .map(|a| Value::Bool(a.value(row))),
        PhysicalTypeID::Int64 => arr
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|a| Value::Int64(a.value(row))),
        PhysicalTypeID::Int32 => arr
            .as_any()
            .downcast_ref::<Int32Array>()
            .map(|a| Value::Int32(a.value(row))),
        PhysicalTypeID::Int16 => arr
            .as_any()
            .downcast_ref::<Int16Array>()
            .map(|a| Value::Int16(a.value(row))),
        PhysicalTypeID::Int8 => arr
            .as_any()
            .downcast_ref::<Int8Array>()
            .map(|a| Value::Int8(a.value(row))),
        PhysicalTypeID::UInt64 => arr
            .as_any()
            .downcast_ref::<UInt64Array>()
            .map(|a| Value::UInt64(a.value(row))),
        PhysicalTypeID::UInt32 => arr
            .as_any()
            .downcast_ref::<UInt32Array>()
            .map(|a| Value::UInt32(a.value(row))),
        PhysicalTypeID::UInt16 => arr
            .as_any()
            .downcast_ref::<UInt16Array>()
            .map(|a| Value::UInt16(a.value(row))),
        PhysicalTypeID::UInt8 => arr
            .as_any()
            .downcast_ref::<UInt8Array>()
            .map(|a| Value::UInt8(a.value(row))),
        PhysicalTypeID::Double => arr
            .as_any()
            .downcast_ref::<Float64Array>()
            .map(|a| Value::Double(a.value(row))),
        PhysicalTypeID::Float => arr
            .as_any()
            .downcast_ref::<Float32Array>()
            .map(|a| Value::Float(a.value(row))),
        PhysicalTypeID::String => arr
            .as_any()
            .downcast_ref::<StringArray>()
            .map(|a| Value::String(a.value(row).to_string()))
            .or_else(|| {
                arr.as_any()
                    .downcast_ref::<LargeStringArray>()
                    .map(|a| Value::String(a.value(row).to_string()))
            }),
        PhysicalTypeID::Blob => arr
            .as_any()
            .downcast_ref::<BinaryArray>()
            .map(|a| Value::Blob(a.value(row).to_vec())),
        _ => None,
    };
    value.or_else(|| Some(Value::String(format!("{:?}", field.slice(row, 1)))))
}
