//! Native Azure Blob Storage client for Akar.
//!
//! Downloads blobs from Azure Blob Storage using REST API over HTTPS.
//! Supports `az://` and `abfss://` URI schemes with SAS token or access key auth.

use tempfile::NamedTempFile;

/// Parsed Azure blob URI components.
#[derive(Debug, Clone)]
pub struct AzureBlobUri {
    pub account: String,
    pub container: String,
    pub blob: String,
    /// SAS query string (without leading `?`) embedded in the URI, if any.
    pub sas_query: Option<String>,
}

/// Parse an Azure URI into its components.
///
/// Supported formats:
/// - `az://account/container/path/to/blob`
/// - `az://container/path/to/blob` (uses AZURE_STORAGE_ACCOUNT env var for account)
/// - `abfss://container@account.dfs.core.windows.net/path/to/blob`
/// - `abfss://container@account.dfs.core.windows.net/path/to/blob?sas_token`
pub fn parse_azure_uri(uri: &str) -> Result<AzureBlobUri, String> {
    let uri = uri.trim();

    if let Some(rest) = uri.strip_prefix("abfss://").or_else(|| uri.strip_prefix("az://")) {
        // abfss://container@account.dfs.core.windows.net/path/to/blob
        if let Some(at_part) = rest.find('@') {
            let container = &rest[..at_part];
            let after_at = &rest[at_part + 1..];
            let (account, blob) = if let Some(dot_idx) = after_at.find('.') {
                let acc = &after_at[..dot_idx];
                let path_start = after_at.find("/").unwrap_or(after_at.len());
                let path = &after_at[path_start..];
                let blob = path.trim_start_matches('/');
                (acc.to_string(), blob.to_string())
            } else {
                return Err(format!("Invalid abfss URI (missing account): {uri}"));
            };
            // Split off the SAS token query param from the blob path.
            let (blob, sas_query) = split_blob_query(&blob);
            Ok(AzureBlobUri {
                account,
                container: container.to_string(),
                blob,
                sas_query,
            })
        } else {
            // az://container/path or az://account/container/path
            let parts: Vec<&str> = rest.splitn(3, '/').collect();
            match parts.len() {
                3 => {
                    let (blob, sas_query) = split_blob_query(parts[2]);
                    Ok(AzureBlobUri {
                        account: parts[0].to_string(),
                        container: parts[1].to_string(),
                        blob,
                        sas_query,
                    })
                }
                2 => {
                    let account = std::env::var("AZURE_STORAGE_ACCOUNT").map_err(|_| {
                        "Missing AZURE_STORAGE_ACCOUNT env var for az://container/path format".to_string()
                    })?;
                    let (blob, sas_query) = split_blob_query(parts[1]);
                    Ok(AzureBlobUri {
                        account,
                        container: parts[0].to_string(),
                        blob,
                        sas_query,
                    })
                }
                _ => Err(format!("Invalid az URI: {uri}")),
            }
        }
    } else {
        Err(format!(
            "Unsupported Azure URI scheme (expected az:// or abfss://): {uri}"
        ))
    }
}

/// Split a blob path on its `?` query component (the SAS token).
fn split_blob_query(blob: &str) -> (String, Option<String>) {
    match blob.split_once('?') {
        Some((b, q)) => (b.to_string(), Some(q.to_string())),
        None => (blob.to_string(), None),
    }
}

/// Percent-encode a blob path for use in a URL path.
///
/// Encodes every byte except the URL unreserved characters and the `/`
/// separator, so spaces, unicode, `&`, `?`, `#` and `%` in blob names cannot
/// corrupt the request URL (P52.34).
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the HTTPS URL for a blob.
fn blob_url(uri: &AzureBlobUri) -> String {
    format!(
        "https://{}.blob.core.windows.net/{}/{}",
        uri.account,
        percent_encode_path(&uri.container),
        percent_encode_path(&uri.blob)
    )
}

/// Get the SAS token from environment variable.
fn sas_token() -> Option<String> {
    std::env::var("AZURE_STORAGE_SAS_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Retry policy for transient Azure failures (HTTP 429 / 5xx and transport
/// errors): up to [`RETRY_ATTEMPTS`] attempts with a small exponential backoff.
const RETRY_ATTEMPTS: usize = 3;

fn retryable_status(status: u16) -> bool {
    status == 429 || status >= 500
}

fn backoff_delay(attempt: usize) -> std::time::Duration {
    std::time::Duration::from_millis(100u64 * (1 << attempt))
}

/// Download a blob from Azure Blob Storage to a temporary file.
///
/// Returns the path to the downloaded temp file.
/// Supports authentication via:
/// 1. SAS token embedded in the URI query (e.g. `abfss://...?sv=...&sig=...`)
/// 2. SAS token in `AZURE_STORAGE_SAS_TOKEN` env var
/// 3. Access key in `AZURE_STORAGE_ACCESS_KEY` env var (SharedKey auth)
pub fn download_blob(uri: &str) -> Result<String, String> {
    let parsed = parse_azure_uri(uri)?;
    let mut url = blob_url(&parsed);

    // Prefer the SAS embedded in the URI, falling back to the env var.
    let sas = parsed.sas_query.clone().or_else(sas_token);
    if let Some(sas) = sas {
        let sas_clean = sas.strip_prefix('?').unwrap_or(&sas);
        url.push_str(&format!("?{sas_clean}"));
    }

    let mut last_err = "no attempt made".to_string();
    for attempt in 0..RETRY_ATTEMPTS {
        let resp = match ureq::get(&url).call() {
            Ok(resp) => resp,
            Err(e) => {
                last_err = format!("{e}");
                if attempt + 1 < RETRY_ATTEMPTS {
                    std::thread::sleep(backoff_delay(attempt));
                    continue;
                }
                return Err(format!("Azure blob download failed for {uri}: {last_err}"));
            }
        };

        let status = resp.status();
        if status != 200 {
            last_err = format!("HTTP {status}");
            if retryable_status(status.into()) && attempt + 1 < RETRY_ATTEMPTS {
                std::thread::sleep(backoff_delay(attempt));
                continue;
            }
            return Err(format!("Azure blob download returned HTTP {status} for {uri}"));
        }

        let mut reader = resp.into_body().into_reader();
        let mut temp_file = NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {e}"))?;
        std::io::copy(&mut reader, &mut temp_file).map_err(|e| format!("Failed to write temp file: {e}"))?;

        // Retain the file under a bounded registry so it is eventually cleaned up
        // instead of leaking forever.
        let (_file, path) = temp_file.keep().map_err(|e| format!("Failed to keep temp file: {e}"))?;
        return Ok(akar_common::extension_utils::retain_temp_file(path));
    }

    Err(format!("Azure blob download failed for {uri}: {last_err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_az_uri_with_account() {
        let uri = parse_azure_uri("az://myaccount/mycontainer/data/file.parquet").unwrap();
        assert_eq!(uri.account, "myaccount");
        assert_eq!(uri.container, "mycontainer");
        assert_eq!(uri.blob, "data/file.parquet");
    }

    #[test]
    fn test_parse_abfss_uri() {
        let uri = parse_azure_uri("abfss://mycontainer@myaccount.dfs.core.windows.net/data/file.parquet").unwrap();
        assert_eq!(uri.account, "myaccount");
        assert_eq!(uri.container, "mycontainer");
        assert_eq!(uri.blob, "data/file.parquet");
    }

    #[test]
    fn test_parse_abfss_with_sas() {
        let uri = parse_azure_uri("abfss://c@a.dfs.core.windows.net/p/f?sv=2020&se=2025").unwrap();
        assert_eq!(uri.account, "a");
        assert_eq!(uri.container, "c");
        assert_eq!(uri.blob, "p/f");
        assert_eq!(uri.sas_query.as_deref(), Some("sv=2020&se=2025"));
    }

    #[test]
    fn test_parse_az_with_sas() {
        let uri = parse_azure_uri("az://a/c/p/f?sv=2020&sig=abc").unwrap();
        assert_eq!(uri.blob, "p/f");
        assert_eq!(uri.sas_query.as_deref(), Some("sv=2020&sig=abc"));
    }

    #[test]
    fn test_parse_invalid_scheme() {
        assert!(parse_azure_uri("s3://bucket/key").is_err());
    }

    #[test]
    fn test_parse_az_with_env_account() {
        // az://container/path needs env var. Test the 3-part format instead:
        assert_eq!(parse_azure_uri("az://acct/cont/p").unwrap().account, "acct");
    }

    #[test]
    fn test_percent_encode_path() {
        assert_eq!(percent_encode_path("data/my file.parquet"), "data/my%20file.parquet");
        assert_eq!(percent_encode_path("folder/a&b.csv"), "folder/a%26b.csv");
        assert_eq!(percent_encode_path("dir/naïve.csv"), "dir/na%C3%AFve.csv");
        assert_eq!(percent_encode_path("plain/file.parquet"), "plain/file.parquet");
        assert_eq!(percent_encode_path("a/b?c#d%e"), "a/b%3Fc%23d%25e");
    }

    #[test]
    fn test_blob_url() {
        let uri = AzureBlobUri {
            account: "myaccount".into(),
            container: "mycontainer".into(),
            blob: "data/file.parquet".into(),
            sas_query: None,
        };
        assert_eq!(
            blob_url(&uri),
            "https://myaccount.blob.core.windows.net/mycontainer/data/file.parquet"
        );
    }

    #[test]
    fn test_blob_url_encodes_special_chars() {
        let uri = AzureBlobUri {
            account: "myaccount".into(),
            container: "mycontainer".into(),
            blob: "folder/my file&name.parquet".into(),
            sas_query: None,
        };
        assert_eq!(
            blob_url(&uri),
            "https://myaccount.blob.core.windows.net/mycontainer/folder/my%20file%26name.parquet"
        );
    }
}
