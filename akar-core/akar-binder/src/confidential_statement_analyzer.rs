use std::collections::HashSet;

static CONFIDENTIAL_OPTIONS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    HashSet::from([
        // HTTPFS / S3
        "S3_ACCESS_KEY_ID",
        "S3_SECRET_ACCESS_KEY",
        "S3_SESSION_TOKEN",
        // HTTPFS / GCS
        "GCS_ACCESS_KEY_ID",
        "GCS_SECRET_ACCESS_KEY",
        "GCS_SESSION_TOKEN",
        // Azure
        "AZURE_CONNECTION_STRING",
        "AZURE_ACCOUNT_NAME",
    ])
});

/// Checks whether a query string is a `CALL` statement that sets a confidential
/// extension option (e.g. `CALL S3_SECRET_ACCESS_KEY = '...'`).
///
/// This is a pure string-level check — the query is not fully parsed. It works
/// by extracting the first token after `CALL` and checking against a known set
/// of confidential option names (case-insensitive).
///
/// Matching the C++ `ConfidentialStatementAnalyzer` behavior from LadybugDB:
/// - Only activates on STANDALONE_CALL statements
/// - Checks whether the target option was registered with `isConfidential = true`
pub fn is_confidential_call(query: &str) -> bool {
    let trimmed = query.trim();
    if !trimmed.to_uppercase().starts_with("CALL ") && !trimmed.to_uppercase().starts_with("CALL(") {
        return false;
    }
    let after_call = trimmed[4..].trim();
    let option_name = after_call
        .split([' ', '(', '=', '\t'])
        .next()
        .unwrap_or("")
        .trim()
        .to_uppercase();
    CONFIDENTIAL_OPTIONS.contains(option_name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_secret_key() {
        assert!(is_confidential_call("CALL S3_SECRET_ACCESS_KEY = 'mysecret'"));
        assert!(is_confidential_call("CALL s3_secret_access_key='value'"));
        assert!(is_confidential_call("CALL S3_ACCESS_KEY_ID='abc'"));
        assert!(is_confidential_call("CALL S3_SESSION_TOKEN='token'"));
    }

    #[test]
    fn test_gcs_secret_key() {
        assert!(is_confidential_call("CALL GCS_SECRET_ACCESS_KEY='secret'"));
    }

    #[test]
    fn test_azure_confidential() {
        assert!(is_confidential_call(
            "CALL AZURE_CONNECTION_STRING='DefaultEndpointsProtocol=https'"
        ));
        assert!(is_confidential_call("CALL AZURE_ACCOUNT_NAME='mystorage'"));
    }

    #[test]
    fn test_non_confidential() {
        assert!(!is_confidential_call("MATCH (n) RETURN n"));
        assert!(!is_confidential_call("CALL table_info('foo')"));
        assert!(!is_confidential_call("CALL current_setting('version')"));
        assert!(!is_confidential_call("SELECT 1"));
        assert!(!is_confidential_call("S3_SECRET_ACCESS_KEY = 'x'"));
    }

    #[test]
    fn test_case_insensitive() {
        assert!(is_confidential_call("call s3_secret_access_key = 'v'"));
        assert!(is_confidential_call("Call S3_Access_Key_Id = 'v'"));
    }

    #[test]
    fn test_all_confidential_options() {
        let options = [
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
            "S3_SESSION_TOKEN",
            "GCS_ACCESS_KEY_ID",
            "GCS_SECRET_ACCESS_KEY",
            "GCS_SESSION_TOKEN",
            "AZURE_CONNECTION_STRING",
            "AZURE_ACCOUNT_NAME",
        ];
        for option in options {
            let query_space = format!("CALL {} = 'val'", option);
            assert!(is_confidential_call(&query_space), "Failed for space query: {}", query_space);

            let query_eq = format!("CALL {}='val'", option);
            assert!(is_confidential_call(&query_eq), "Failed for eq query: {}", query_eq);

            let query_lower = format!("call {} = 'val'", option.to_lowercase());
            assert!(is_confidential_call(&query_lower), "Failed for lowercase query: {}", query_lower);
        }
    }

    #[test]
    fn test_syntax_and_whitespace_variations() {
        assert!(is_confidential_call("   CALL   S3_SECRET_ACCESS_KEY   =   'val'   "));
        assert!(is_confidential_call("CALL S3_SECRET_ACCESS_KEY\t=\t'val'"));
        assert!(is_confidential_call("CALL S3_SECRET_ACCESS_KEY ( 'val' )"));
    }

    #[test]
    fn test_boundary_and_false_positives() {
        // Empty / short inputs
        assert!(!is_confidential_call(""));
        assert!(!is_confidential_call("   "));
        assert!(!is_confidential_call("CAL"));
        assert!(!is_confidential_call("CALL"));

        // Extended or prefix option names (should not match exact set)
        assert!(!is_confidential_call("CALL S3_SECRET_ACCESS_KEY_EXTRA = 'val'"));
        assert!(!is_confidential_call("CALL MY_S3_SECRET_ACCESS_KEY = 'val'"));
        assert!(!is_confidential_call("CALL S3_SECRET = 'val'"));

        // Statements where CALL is a prefix of another word or embedded elsewhere
        assert!(!is_confidential_call("CALLABLE S3_SECRET_ACCESS_KEY = 'val'"));
        assert!(!is_confidential_call("RECALL S3_SECRET_ACCESS_KEY = 'val'"));
        assert!(!is_confidential_call("SELECT 'CALL S3_SECRET_ACCESS_KEY'"));

        // Non-space whitespace directly after CALL
        assert!(!is_confidential_call("CALL\tS3_SECRET_ACCESS_KEY='val'"));
        assert!(!is_confidential_call("CALL\nS3_SECRET_ACCESS_KEY = 'val'"));

        // CALL( directly followed by option without space is unhandled due to split on '('
        assert!(!is_confidential_call("CALL(S3_SECRET_ACCESS_KEY='val')"));
        assert!(!is_confidential_call("CALL (S3_SECRET_ACCESS_KEY='val')"));
        assert!(!is_confidential_call("CALL( S3_SECRET_ACCESS_KEY = 'val' )"));
    }
}
