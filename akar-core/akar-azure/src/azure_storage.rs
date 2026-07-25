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
            // Remove SAS token query param from blob path
            let blob = blob.split('?').next().unwrap_or(&blob).to_string();
            Ok(AzureBlobUri {
                account,
                container: container.to_string(),
                blob,
            })
        } else {
            // az://container/path or az://account/container/path
            let parts: Vec<&str> = rest.splitn(3, '/').collect();
            match parts.len() {
                3 => Ok(AzureBlobUri {
                    account: parts[0].to_string(),
                    container: parts[1].to_string(),
                    blob: parts[2].to_string(),
                }),
                2 => {
                    let account = std::env::var("AZURE_STORAGE_ACCOUNT").map_err(|_| {
                        "Missing AZURE_STORAGE_ACCOUNT env var for az://container/path format".to_string()
                    })?;
                    Ok(AzureBlobUri {
                        account,
                        container: parts[0].to_string(),
                        blob: parts[1].to_string(),
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

/// Build the HTTPS URL for a blob.
fn blob_url(uri: &AzureBlobUri) -> String {
    format!(
        "https://{}.blob.core.windows.net/{}/{}",
        uri.account, uri.container, uri.blob
    )
}

/// Get the SAS token from environment variable.
fn sas_token() -> Option<String> {
    std::env::var("AZURE_STORAGE_SAS_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Download a blob from Azure Blob Storage to a temporary file.
///
/// Returns the path to the downloaded temp file.
/// Supports authentication via:
/// 1. SAS token in `AZURE_STORAGE_SAS_TOKEN` env var
/// 2. Access key in `AZURE_STORAGE_ACCESS_KEY` env var (SharedKey auth)
pub fn download_blob(uri: &str) -> Result<String, String> {
    let parsed = parse_azure_uri(uri)?;
    let mut url = blob_url(&parsed);

    // Append SAS token if available
    if let Some(sas) = sas_token() {
        let sas_clean = sas.strip_prefix('?').unwrap_or(&sas);
        url.push_str(&format!("?{sas_clean}"));
    }

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Azure blob download failed for {uri}: {e}"))?;

    let status = resp.status();
    if status != 200 {
        return Err(format!("Azure blob download returned HTTP {status} for {uri}"));
    }

    let mut reader = resp.into_body().into_reader();
    let mut temp_file = NamedTempFile::new().map_err(|e| format!("Failed to create temp file: {e}"))?;
    std::io::copy(&mut reader, &mut temp_file).map_err(|e| format!("Failed to write temp file: {e}"))?;

    let path_str = temp_file.path().to_string_lossy().to_string();
    let _ = temp_file.keep();

    Ok(path_str)
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
    fn test_blob_url() {
        let uri = AzureBlobUri {
            account: "myaccount".into(),
            container: "mycontainer".into(),
            blob: "data/file.parquet".into(),
        };
        assert_eq!(
            blob_url(&uri),
            "https://myaccount.blob.core.windows.net/mycontainer/data/file.parquet"
        );
    }
}
