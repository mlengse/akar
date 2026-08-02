//! Remote client for the Akar embedded server (P47).
//!
//! The embedded server (crate `akar-server`) owns the [`Database`] instance
//! and its exclusive file lock. Remote clients connect over TCP using a
//! length-prefixed JSON protocol and never open the database directory
//! themselves — the server holds every file lock on their behalf.
//!
//! # Usage
//!
//! ```no_run
//! use akar_main::Database;
//!
//! // On the server side (separate process):
//! // let db = Arc::new(Database::new("./my_db", SystemConfig::default())?);
//! // let mut server = akar_server::Server::bind("127.0.0.1:9876", db)?;
//! // server.start()?;
//!
//! // On the client side:
//! let client = Database::connect_tcp("127.0.0.1:9876")?;
//! let res = client.query("MATCH (n) RETURN n LIMIT 5")?;
//! assert!(res.success);
//! # Ok::<(), String>(())
//! ```

use akar_common::types::Value;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

/// Default port the Akar server binds to when none is specified.
pub const DEFAULT_PORT: u16 = 9876;

/// Maximum accepted frame size (128 MiB).
///
/// Protects both the server and the client from unbounded allocations caused
/// by a corrupt or hostile peer.
pub const MAX_FRAME_SIZE: usize = 128 * 1024 * 1024;

/// A request sent from a client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRequest {
    /// The Cypher query to execute.
    pub query: String,
    /// Optional client identifier (currently informational only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

/// The response the server returns for a query.
///
/// `rows` is row-major: `rows[row][col]`, with `None` for SQL NULLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub column_names: Vec<String>,
    pub rows: Vec<Vec<Option<Value>>>,
}

impl WireResponse {
    /// Build a success response with a human-readable message (e.g. DDL).
    pub fn success_message(msg: String) -> Self {
        Self {
            success: true,
            message: Some(msg),
            error_message: None,
            column_names: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Build an error response.
    pub fn error(msg: String) -> Self {
        Self {
            success: false,
            message: None,
            error_message: Some(msg),
            column_names: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Number of result rows.
    pub fn num_rows(&self) -> usize {
        self.rows.len()
    }

    /// Number of result columns.
    pub fn num_columns(&self) -> usize {
        self.column_names.len()
    }

    /// Read the cell at `(row, col)`, or `None` when out of range or NULL.
    pub fn cell(&self, row: usize, col: usize) -> Option<&Value> {
        self.rows.get(row)?.get(col)?.as_ref()
    }

    /// Collect all non-NULL values in a column.
    pub fn column_values(&self, col: usize) -> Vec<Value> {
        self.rows
            .iter()
            .filter_map(|r| r.get(col).cloned().flatten())
            .collect()
    }

    /// Human-readable summary mirroring [`crate::QueryResult::result_summary`].
    pub fn result_summary(&self) -> String {
        if let Some(msg) = &self.message {
            return msg.clone();
        }
        if !self.success {
            return format!("Error: {}", self.error_message.as_deref().unwrap_or("Unknown error"));
        }
        if self.rows.is_empty() {
            return "(empty result)".into();
        }
        format!("Returned {} rows in {} columns", self.rows.len(), self.column_names.len())
    }
}

impl fmt::Display for WireResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.message {
            return write!(f, "{msg}");
        }
        if !self.success {
            return write!(f, "Error: {}", self.error_message.as_deref().unwrap_or("Unknown error"));
        }
        if self.rows.is_empty() {
            return write!(f, "(empty result)");
        }
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "Row {}: ", i)?;
            for (col, cell) in row.iter().enumerate() {
                if col > 0 {
                    write!(f, ", ")?;
                }
                match cell {
                    Some(v) => write!(f, "{v:?}")?,
                    None => write!(f, "null")?,
                }
            }
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Length-prefixed framing
// ─────────────────────────────────────────────────────────────────────────────

/// State for a partially-read frame (survives read timeouts).
#[derive(Debug)]
pub enum PartialFrame {
    /// 4-byte length header partially read.
    Header([u8; 4], usize),
    /// Payload partially read.
    Payload {
        len: usize,
        buf: Vec<u8>,
        filled: usize,
    },
}

/// Write `payload` as a single length-prefixed frame: `[u32 LE len][bytes]`.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len();
    if len > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("Frame too large: {len} bytes"),
        ));
    }
    writer.write_all(&(len as u32).to_le_bytes())?;
    writer.write_all(payload)
}

/// Read one complete frame from `reader`.
///
/// Returns:
/// - `Ok(Some(bytes))` — a complete frame;
/// - `Ok(None)` — clean end-of-stream (peer closed before any header byte);
/// - `Err(WouldBlock | TimedOut)` — progress was made or not, partial state is
///   kept in `partial`, call again later;
/// - `Err(other)` — protocol or I/O failure.
pub fn read_frame<R: Read>(
    reader: &mut R,
    partial: &mut Option<PartialFrame>,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut header: ([u8; 4], usize) = match partial.take() {
        Some(PartialFrame::Payload { len, mut buf, mut filled }) => {
            if len > MAX_FRAME_SIZE {
                return Err(std::io::Error::new(ErrorKind::InvalidData, "Frame too large"));
            }
            loop {
                match reader.read(&mut buf[filled..]) {
                    Ok(0) => {
                        return Err(std::io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "Unexpected EOF in frame payload",
                        ))
                    }
                    Ok(n) => {
                        filled += n;
                        if filled == len {
                            return Ok(Some(buf));
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => {
                        *partial = Some(PartialFrame::Payload { len, buf, filled });
                        return Err(e);
                    }
                }
            }
        }
        Some(PartialFrame::Header(h, filled)) => (h, filled),
        None => ([0u8; 4], 0),
    };

    loop {
        match reader.read(&mut header.0[header.1..]) {
            Ok(0) => {
                if header.1 == 0 {
                    return Ok(None);
                }
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "Unexpected EOF in frame header",
                ));
            }
            Ok(n) => {
                header.1 += n;
                if header.1 == 4 {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                *partial = Some(PartialFrame::Header(header.0, header.1));
                return Err(e);
            }
        }
    }

    let len = u32::from_le_bytes(header.0) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("Frame too large: {len} bytes"),
        ));
    }
    if len == 0 {
        return Ok(Some(Vec::new()));
    }
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    loop {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "Unexpected EOF in frame payload",
                ))
            }
            Ok(n) => {
                filled += n;
                if filled == len {
                    return Ok(Some(buf));
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                *partial = Some(PartialFrame::Payload { len, buf, filled });
                return Err(e);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Client
// ─────────────────────────────────────────────────────────────────────────────

/// A client connection to a remote Akar server.
///
/// Created via [`RemoteDatabase::connect_tcp`] (or [`crate::Database::connect_tcp`]).
/// Executes queries over a length-prefixed JSON protocol; the server holds the
/// database open, so the client never touches the database directory or its
/// file locks.
pub struct RemoteDatabase {
    stream: TcpStream,
    address: String,
    partial: Mutex<Option<PartialFrame>>,
}

impl RemoteDatabase {
    /// Connect to an Akar server listening at `addr` (e.g. `"127.0.0.1:9876"`).
    pub fn connect_tcp(addr: impl Into<String>) -> Result<Self, String> {
        let addr = addr.into();
        let stream = TcpStream::connect(&addr)
            .map_err(|e| format!("Failed to connect to Akar server at '{addr}': {e}"))?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
        Ok(Self {
            stream,
            address: addr,
            partial: Mutex::new(None),
        })
    }

    /// The address this client is connected to.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Execute a Cypher query on the remote database.
    ///
    /// Mirrors [`crate::Connection::query`]: returns the response on success and
    /// an error message when the query failed (including OCC `WriteConflict`s).
    pub fn query(&self, query_str: &str) -> Result<WireResponse, String> {
        let request = WireRequest {
            query: query_str.to_string(),
            client_name: None,
        };
        let payload = serde_json::to_vec(&request).map_err(|e| format!("Failed to serialize request: {e}"))?;

        {
            let mut writer = &self.stream;
            write_frame(&mut writer, &payload).map_err(|e| format!("Failed to send request: {e}"))?;
            writer.flush().map_err(|e| format!("Failed to flush request: {e}"))?;
        }

        let mut partial = self.partial.lock().map_err(|e| format!("Lock poisoned: {e}"))?;
        let frame = match read_frame(&mut &self.stream, &mut partial) {
            Ok(Some(f)) => f,
            Ok(None) => return Err("Connection closed by server".to_string()),
            Err(e) => {
                return Err(format!("Failed to read response: {e}"));
            }
        };

        let response: WireResponse =
            serde_json::from_slice(&frame).map_err(|e| format!("Failed to parse response: {e}"))?;
        if response.success {
            Ok(response)
        } else {
            Err(response
                .error_message
                .clone()
                .unwrap_or_else(|| "Unknown server error".to_string()))
        }
    }

    /// Verify the connection is alive by round-tripping a trivial query.
    pub fn ping(&self) -> Result<(), String> {
        self.query("RETURN 1").map(|_| ())
    }

    /// Close the connection. The server notices the disconnect and reclaims the
    /// session resources.
    pub fn close(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_roundtrip() {
        let payloads = [b"".to_vec(), b"hello".to_vec(), vec![0u8; 4096], b"{}".to_vec()];
        for payload in payloads {
            let mut buf = Vec::new();
            write_frame(&mut buf, &payload).unwrap();
            let mut cursor = &buf[..];
            let mut partial = None;
            let read = read_frame(&mut cursor, &mut partial).unwrap();
            assert_eq!(read.as_deref(), Some(payload.as_slice()));
        }
    }

    /// A reader that yields at most 3 bytes per `read` call, exercising the
    /// partial-frame state machine inside `read_frame`.
    struct ChunkedReader<'a> {
        inner: &'a mut &'a [u8],
        first_read: bool,
    }

    impl<'a> ChunkedReader<'a> {
        fn new(inner: &'a mut &'a [u8]) -> Self {
            Self { inner, first_read: true }
        }
    }

    impl<'a> Read for ChunkedReader<'a> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.first_read {
                self.first_read = false;
                // Simulate a timeout before any data is available; the caller
                // must retain its partial state and retry.
                return Err(std::io::Error::new(ErrorKind::WouldBlock, "no data yet"));
            }
            if self.inner.is_empty() {
                return Ok(0);
            }
            let n = 3.min(buf.len()).min(self.inner.len());
            buf[..n].copy_from_slice(&self.inner[..n]);
            *self.inner = &self.inner[n..];
            Ok(n)
        }
    }

    #[test]
    fn test_frame_partial_reads() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"partial-frame-test").unwrap();

        let mut cursor = &buf[..];
        let mut partial = None;
        let mut reader = ChunkedReader::new(&mut cursor);

        // First attempt hits a simulated WouldBlock before any bytes arrive.
        let err = read_frame(&mut reader, &mut partial).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        assert!(partial.is_some(), "partial state must be retained across timeouts");

        // Second attempt must reassemble the frame from the retained state.
        let result = read_frame(&mut reader, &mut partial).unwrap();
        assert_eq!(result.as_deref(), Some(b"partial-frame-test".as_slice()));
    }

    #[test]
    fn test_eof_returns_none() {
        let mut cursor = &b""[..];
        let mut partial = None;
        assert!(read_frame(&mut cursor, &mut partial).unwrap().is_none());
    }

    #[test]
    fn test_frame_too_large_rejected() {
        let mut buf = Vec::new();
        // u32 length bigger than MAX_FRAME_SIZE
        buf.extend_from_slice(&(MAX_FRAME_SIZE as u32 + 1).to_le_bytes());
        let mut cursor = &buf[..];
        let mut partial = None;
        assert!(read_frame(&mut cursor, &mut partial).is_err());
    }

    #[test]
    fn test_wire_response_accessors() {
        let resp = WireResponse {
            success: true,
            message: None,
            error_message: None,
            column_names: vec!["name".into(), "age".into()],
            rows: vec![
                vec![Some(Value::String("alice".into())), Some(Value::Int64(30))],
                vec![None, Some(Value::Int64(25))],
            ],
        };
        assert_eq!(resp.num_rows(), 2);
        assert_eq!(resp.num_columns(), 2);
        assert_eq!(resp.cell(0, 0), Some(&Value::String("alice".into())));
        assert_eq!(resp.cell(1, 0), None);
        assert_eq!(resp.cell(9, 9), None);
        assert_eq!(
            resp.column_values(1),
            vec![Value::Int64(30), Value::Int64(25)]
        );
        assert_eq!(resp.result_summary(), "Returned 2 rows in 2 columns");
    }
}
