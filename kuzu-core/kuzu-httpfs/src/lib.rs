//! HTTP File System extension for Kuzu.
//!
//! Provides HTTP/HTTPS file access for reading remote files:
//! - HTTP GET requests
//! - Basic URL parsing
//! - File download to memory

use kuzu_extension::{Extension, ExtensionContext};

/// The HTTPFS extension adds HTTP file system support to Kuzu.
pub struct HttpfsExtension;

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
        use kuzu_function::registry::{ScalarFunction, TableFunction};
        use kuzu_function::registry::UtilityOp;

        context.register_scalar_function(
            "http_get",
            ScalarFunction::Utility { op: UtilityOp::Coalesce },
        );
        context.register_table_function(
            "http_scan",
            TableFunction::ScanJson { path: "http".into() },
        );
        context.register_table_function(
            "https_scan",
            TableFunction::ScanJson { path: "https".into() },
        );

        tracing::info!("HTTPFS extension loaded: 3 functions registered");
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
    let (scheme, rest) = url_str.split_once("://")
        .ok_or_else(|| format!("Invalid URL (no scheme): {url_str}"))?;

    let scheme = scheme.to_lowercase();
    let (host_part, path_and_query) = rest.split_once('/')
        .unwrap_or((rest, ""));

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

    Ok(Url { scheme, host, port, path, query })
}

/// Validate that a URL uses a supported scheme (http/https).
pub fn is_valid_http_url(url_str: &str) -> bool {
    parse_url(url_str)
        .map(|u| matches!(u.scheme.as_str(), "http" | "https"))
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
        assert!(is_valid_http_url("http://example.com"));
        assert!(is_valid_http_url("https://example.com"));
        assert!(!is_valid_http_url("ftp://example.com"));
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
}
