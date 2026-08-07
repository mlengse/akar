//! Client session handling — one thread per TCP connection (P47.2).
//!
//! Each client gets its own [`Connection`] to the shared [`Database`]. Requests
//! are length-prefixed JSON frames; the session loop reads a frame, executes the
//! query through the normal `Connection::query` pipeline (which wraps writes in
//! OCC transactions and surfaces `WriteConflict` as errors), and replies with a
//! serialized [`WireResponse`].

use akar_common::types::{PhysicalTypeID, Value};
use akar_main::connection::Connection;
use akar_main::database::Database;
use akar_main::query_result::QueryResult;
use akar_main::remote::{PartialFrame, WireRequest, WireResponse, read_frame, write_frame};
use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long the session waits for the next frame before re-checking the server
/// shutdown flag. A slow client is never disconnected; this only bounds the
/// shutdown detection latency.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// Serve one client connection until the peer disconnects or the server shuts
/// down. All per-session state is dropped on return.
pub fn handle_client(mut stream: TcpStream, db: Arc<Database>, shutdown: &Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let conn = Connection::new(&db);
    let mut partial: Option<PartialFrame> = None;

    loop {
        if shutdown.load(Ordering::SeqCst) {
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

        let response = execute_query(&conn, &request);
        if write_frame(&mut stream, &response).is_err() {
            return; // client disconnected mid-reply
        }
    }
}

/// Run a single query against the session's connection and serialize the result.
fn execute_query(conn: &Connection, request: &WireRequest) -> Vec<u8> {
    let wire = match conn.query(&request.query) {
        Ok(result) => query_result_to_wire(&result),
        Err(e) => WireResponse::error(e),
    };
    serde_json::to_vec(&wire).unwrap_or_else(|_| {
        br#"{"success":false,"error_message":"response serialization failure","column_names":[],"rows":[]}"#.to_vec()
    })
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
