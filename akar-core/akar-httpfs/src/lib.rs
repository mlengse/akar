//! HTTP File System extension for Akar.
//!
//! Provides HTTP/HTTPS file access for reading remote files:
//! - HTTP GET requests
//! - Basic URL parsing
//! - File download to memory

use akar_common::file_system::{FileRead, FileSystem, FileWrite};
use akar_common::types::Value;
use akar_extension::{Extension, ExtensionContext};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use tempfile::NamedTempFile;

/// A FileSystem implementation for HTTP/HTTPS URLs.
pub struct HttpFileSystem;

impl FileSystem for HttpFileSystem {
    fn can_handle(&self, path: &str) -> bool {
        is_valid_http_url(path)
    }

    fn open_read(&self, path: &str) -> std::io::Result<Box<dyn FileRead>> {
        Ok(Box::new(HttpRandomAccessReader::new(path)?))
    }

    fn open_write(&self, _path: &str) -> std::io::Result<Box<dyn FileWrite>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "HTTPFS is read-only",
        ))
    }

    fn exists(&self, path: &str) -> bool {
        match ureq::head(path).call() {
            Ok(resp) => resp.status() == 200,
            Err(_) => false,
        }
    }

    fn remove(&self, _path: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "HTTPFS is read-only",
        ))
    }

    fn create_dir_all(&self, _path: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "HTTPFS is read-only",
        ))
    }
}

/// Read-ahead window fetched by each HTTP Range request.
const READ_AHEAD: u64 = 256 * 1024;

/// A reader that uses HTTP Range requests for random access.
///
/// Fetches the requested byte range plus a read-ahead window into an internal
/// buffer, so consecutive reads do not issue a request per chunk (P51.19 perf),
/// and verifies the server honors the `Range` header (P52.33): a `200` full-body
/// response or a mismatched `Content-Range` is treated as an error instead of
/// silently returning mis-positioned bytes.
pub struct HttpRandomAccessReader {
    url: String,
    position: u64,
    content_length: Option<u64>,
    /// Absolute offset of the first byte in `buf`.
    buf_start: u64,
    buf: Vec<u8>,
}

impl HttpRandomAccessReader {
    pub fn new(url: &str) -> std::io::Result<Self> {
        // HEAD may be unsupported by some servers; treat it as optional.
        let content_length = ureq::head(url).call().ok().and_then(|resp| {
            resp.headers()
                .get("Content-Length")
                .and_then(|s| s.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
        });
        Ok(Self {
            url: url.to_string(),
            position: 0,
            content_length,
            buf_start: 0,
            buf: Vec::new(),
        })
    }

    /// Fetch a window covering `[self.position, self.position + READ_AHEAD)`
    /// into the internal buffer, verifying the server actually honors the range.
    fn fetch_window(&mut self) -> std::io::Result<()> {
        if let Some(len) = self.content_length {
            if self.position >= len {
                self.buf.clear();
                return Ok(());
            }
        }
        let start = self.position;
        let end = match self.content_length {
            Some(len) => (start + READ_AHEAD - 1).min(len.saturating_sub(1)),
            None => start + READ_AHEAD - 1,
        };
        let range_header = format!("bytes={start}-{end}");

        let resp = match ureq::get(&self.url).header("Range", &range_header).call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(416)) => {
                // Range not satisfiable → we are at/past end of file.
                self.buf.clear();
                return Ok(());
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(std::io::Error::other(format!(
                    "HTTP Range request failed for {} (status {code})",
                    self.url
                )));
            }
            Err(e) => {
                return Err(std::io::Error::other(format!(
                    "HTTP Range request failed for {}: {e}",
                    self.url
                )));
            }
        };

        let status = resp.status();
        if status != 206 {
            return Err(std::io::Error::other(format!(
                "HTTP server did not honor Range request (status {status}) for {}",
                self.url
            )));
        }

        let content_range = resp
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                std::io::Error::other(format!(
                    "HTTP server returned 206 without Content-Range for {}",
                    self.url
                ))
            })?;
        let (actual_start, actual_end) = parse_content_range(content_range).ok_or_else(|| {
            std::io::Error::other(format!(
                "HTTP server returned malformed Content-Range `{content_range}` for {}",
                self.url
            ))
        })?;
        if actual_start != start || actual_end < start {
            return Err(std::io::Error::other(format!(
                "HTTP server returned unexpected range {actual_start}-{actual_end}, requested {start}-{end} for {}",
                self.url
            )));
        }

        // Read exactly the advertised bytes (bounded — never more than requested).
        let expected = actual_end - actual_start + 1;
        let reader = resp.into_body().into_reader();
        let mut bytes = Vec::with_capacity(expected as usize);
        let n = reader.take(expected + 1).read_to_end(&mut bytes)?;
        if n as u64 != expected {
            return Err(std::io::Error::other(format!(
                "HTTP server returned {n} bytes for range {start}-{end}, expected {expected}, for {}",
                self.url
            )));
        }

        self.buf_start = actual_start;
        self.buf = bytes;
        Ok(())
    }
}

impl Read for HttpRandomAccessReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(len) = self.content_length {
            if self.position >= len {
                return Ok(0);
            }
        }

        let mut filled = 0;
        while filled < buf.len() {
            let rel = self.position.checked_sub(self.buf_start);
            if let Some(rel) = rel {
                if let Some(avail) = (self.buf.len() as u64).checked_sub(rel) {
                    if avail > 0 {
                        let off = rel as usize;
                        let n = (avail as usize).min(buf.len() - filled);
                        buf[filled..filled + n].copy_from_slice(&self.buf[off..off + n]);
                        self.position += n as u64;
                        filled += n;
                        continue;
                    }
                }
            }
            self.fetch_window()?;
            if self.buf.is_empty() {
                break;
            }
        }
        Ok(filled)
    }
}

impl Seek for HttpRandomAccessReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Start(offset) => {
                self.position = offset;
            }
            SeekFrom::Current(offset) => {
                let new_pos = self.position as i64 + offset;
                if new_pos < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid seek to a negative position",
                    ));
                }
                self.position = new_pos as u64;
            }
            SeekFrom::End(offset) => {
                if let Some(len) = self.content_length {
                    let new_pos = len as i64 + offset;
                    if new_pos < 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "invalid seek to a negative position",
                        ));
                    }
                    self.position = new_pos as u64;
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "SeekFrom::End requires Content-Length",
                    ));
                }
            }
        }
        Ok(self.position)
    }
}

impl FileRead for HttpRandomAccessReader {}

/// Parse a `Content-Range` header value of the form `bytes {start}-{end}/{total}`.
fn parse_content_range(header: &str) -> Option<(u64, u64)> {
    let rest = header.strip_prefix("bytes ")?;
    let (range, _total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

/// The HTTPFS extension adds HTTP file system support to Akar.
pub struct HttpfsExtension;

impl Default for HttpfsExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpfsExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for HttpfsExtension {
    fn name(&self) -> &'static str {
        "HTTPFS"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use akar_function::registry::{ScalarFunction, TableFunction};

        // http_get: Synchronously fetches a URL and returns the body as a String.
        context.register_scalar_function(
            "http_get",
            ScalarFunction::CustomScalar {
                name: "http_get".into(),
                execute: Arc::new(|args: &[Value]| {
                    if args.is_empty() {
                        return Err("http_get requires 1 argument (URL)".into());
                    }
                    if let Value::String(url) = &args[0] {
                        let body = ureq::get(url).call().map_err(|e| format!("HTTP GET failed: {}", e))?;
                        // Cap the response body so an untrusted server cannot
                        // stream an unbounded body and OOM the embedded process
                        // (P52.60).
                        const MAX_BODY: u64 = 64 * 1024 * 1024;
                        let mut body_text = String::new();
                        let n = body
                            .into_body()
                            .into_reader()
                            .take(MAX_BODY + 1)
                            .read_to_string(&mut body_text)
                            .map_err(|e| format!("Failed to read response body: {}", e))?;
                        if n as u64 > MAX_BODY {
                            return Err(format!(
                                "http_get response body for {url} exceeds the {MAX_BODY}-byte limit"
                            ));
                        }
                        Ok(Value::String(body_text))
                    } else {
                        Err("http_get argument must be a string".into())
                    }
                }),
            },
        );

        // http_scan: Downloads a remote file to a temp file, but since we cannot easily yield rows here,
        // we map it to a CustomTable that returns the path to the downloaded file.
        // A full integration would rewrite the AST to ScanCsv/ScanParquet.
        let http_scan_exec = Arc::new(
            |args: &[Value], chunk: &mut akar_common::vector::DataChunk| -> Result<(), String> {
                if args.is_empty() {
                    return Err("http_scan requires 1 argument (URL)".into());
                }
                if chunk.size > 0 {
                    return Ok(()); // Only yield 1 row
                }
                if let Value::String(url) = &args[0] {
                    let resp = ureq::get(url).call().map_err(|e| format!("HTTP GET failed: {}", e))?;
                    let mut reader = resp.into_body().into_reader();
                    let mut temp_file =
                        NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {}", e))?;
                    std::io::copy(&mut reader, &mut temp_file)
                        .map_err(|e| format!("Failed to write temp file: {}", e))?;

                    let (_file, path) = temp_file
                        .keep()
                        .map_err(|e| format!("Failed to keep temp file: {}", e))?;
                    // Retain the file under a bounded registry so it is eventually
                    // cleaned up instead of leaking forever.
                    let path_str = akar_common::extension_utils::retain_temp_file(path);

                    akar_common::extension_utils::fill_chunk_with_strings(chunk, "path", &[path_str]);
                    Ok(())
                } else {
                    Err("http_scan argument must be a string".into())
                }
            },
        );

        context.register_table_function(
            "http_scan",
            TableFunction::CustomTable {
                name: "http_scan".into(),
                execute: http_scan_exec.clone(),
            },
        );
        context.register_table_function(
            "https_scan",
            TableFunction::CustomTable {
                name: "https_scan".into(),
                execute: http_scan_exec,
            },
        );

        // Register the virtual file system
        context.register_file_system(Box::new(HttpFileSystem));

        tracing::info!("HTTPFS extension loaded: 3 functions registered, 1 VFS registered");
        Ok(())
    }
}

/// Parse a URL into its components.
#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: Option<String>,
}

/// Parse a URL string into its components.
pub fn parse_url(url_str: &str) -> Result<Url, String> {
    let url_str = url_str.trim();
    let (scheme, rest) = url_str
        .split_once("://")
        .ok_or_else(|| format!("Invalid URL (no scheme): {url_str}"))?;

    let scheme = scheme.to_lowercase();
    let (host_part, path_and_query) = rest.split_once('/').unwrap_or((rest, ""));

    let path = if path_and_query.is_empty() {
        "/".to_string()
    } else {
        format!("/{path_and_query}")
    };

    let (host, port) = if let Some((h, p)) = host_part.split_once(':') {
        let port: u16 = p.parse().map_err(|_| format!("Invalid port: {p}"))?;
        (h.to_string(), Some(port))
    } else {
        (host_part.to_string(), None)
    };

    let (path, query) = if let Some((p, q)) = path.split_once('?') {
        (p.to_string(), Some(q.to_string()))
    } else {
        (path, None)
    };

    Ok(Url {
        scheme,
        host,
        port,
        path,
        query,
    })
}

/// Validate that a URL uses a supported scheme (http/https) and has a host.
pub fn is_valid_http_url(url_str: &str) -> bool {
    parse_url(url_str)
        .map(|u| !u.host.is_empty() && matches!(u.scheme.as_str(), "http" | "https"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_http_url() {
        let url = parse_url("http://example.com/data.csv").unwrap();
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.path, "/data.csv");
    }

    #[test]
    fn test_parse_https_url() {
        let url = parse_url("https://api.example.com/v1/data?format=json").unwrap();
        assert_eq!(url.scheme, "https");
        assert_eq!(url.host, "api.example.com");
        assert_eq!(url.path, "/v1/data");
        assert_eq!(url.query, Some("format=json".into()));
    }

    #[test]
    fn test_parse_url_with_port() {
        let url = parse_url("http://localhost:8080/query").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, Some(8080));
    }

    #[test]
    fn test_parse_url_no_path() {
        let url = parse_url("https://example.com").unwrap();
        assert_eq!(url.path, "/");
    }

    #[test]
    fn test_is_valid_http_url() {
        let valid = [
            "http://example.com",
            "https://example.com",
            "http://example.com/data.csv",
            "https://api.example.com/v1/data?format=json",
            "http://localhost:8080/query",
            "http://example.com:8080",
            "HTTP://example.com",
            "  https://example.com  ",
        ];
        for url in valid {
            assert!(is_valid_http_url(url), "expected `{url}` to be a valid HTTP URL");
        }

        let invalid = [
            "",
            "not-a-url",
            "example.com/data.csv",
            "ftp://example.com",
            "file:///tmp/data.csv",
            "ws://example.com",
            "gs://bucket/data.csv",
            "http:/example.com",
            "http//example.com",
            "://example.com",
            "http://",
            "https://",
            "http://:8080",
            "http://example.com:notaport",
        ];
        for url in invalid {
            assert!(
                !is_valid_http_url(url),
                "expected `{url}` to be rejected as an HTTP URL"
            );
        }
    }

    #[test]
    fn test_can_handle_http_url() {
        let fs = HttpFileSystem;
        assert!(fs.can_handle("http://example.com/data.csv"));
        assert!(fs.can_handle("https://example.com"));
        assert!(!fs.can_handle("ftp://example.com"));
        assert!(!fs.can_handle("local/file.csv"));
    }

    #[test]
    fn test_invalid_url() {
        assert!(parse_url("not-a-url").is_err());
    }

    #[test]
    fn test_httpfs_extension_name() {
        let ext = HttpfsExtension::new();
        assert_eq!(ext.name(), "HTTPFS");
    }

    #[test]
    fn test_parse_content_range() {
        assert_eq!(parse_content_range("bytes 0-1023/2048"), Some((0, 1023)));
        assert_eq!(parse_content_range("bytes 1024-2047/2048"), Some((1024, 2047)));
        assert_eq!(parse_content_range("bytes 0-0/1"), Some((0, 0)));
        assert_eq!(parse_content_range("bytes 0-1023/*"), Some((0, 1023)));
        assert_eq!(parse_content_range("bytes 0-1023"), None);
        assert_eq!(parse_content_range("chunked 0-10/100"), None);
        assert_eq!(parse_content_range("bytes a-b/100"), None);
        assert_eq!(parse_content_range(""), None);
    }

    #[test]
    fn test_read_ahead_window_size() {
        assert_eq!(READ_AHEAD, 256 * 1024);
    }
}
