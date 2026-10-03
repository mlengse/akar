use anyhow::{Context, Result};
use clap::Parser;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Akar C++ to Rust database migration tool
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the source C++ database directory
    #[arg(short, long)]
    from: PathBuf,

    /// Path to the destination Rust database directory
    #[arg(short, long)]
    to: PathBuf,

    /// Skip the Python extraction step (assumes `from` contains schema.json and Parquet files)
    #[arg(long, default_value_t = false)]
    skip_extract: bool,

    /// Explicit path to the Python executable
    #[arg(long)]
    python_path: Option<PathBuf>,
}

fn find_in_path(name: &Path) -> Option<PathBuf> {
    if name.components().count() > 1 {
        if name.is_file() {
            return name.canonicalize().ok().or_else(|| Some(name.to_path_buf()));
        }
        return None;
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate.canonicalize().ok().or(Some(candidate));
            }
        }
    }
    None
}

fn resolve_python_executable(custom_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = custom_path {
        if path.is_absolute() {
            if path.is_file() {
                return Ok(path.to_path_buf());
            } else {
                anyhow::bail!("Specified --python-path does not exist or is not a file: {:?}", path);
            }
        } else if let Some(found) = find_in_path(path) {
            return Ok(found);
        } else if path.exists() {
            return path.canonicalize().context("Failed to canonicalize python-path");
        } else {
            anyhow::bail!("Specified --python-path not found: {:?}", path);
        }
    }

    if let Ok(env_python) = std::env::var("PYTHON") {
        if !env_python.trim().is_empty() {
            let path = Path::new(&env_python);
            if path.is_absolute() && path.is_file() {
                return Ok(path.to_path_buf());
            } else if let Some(found) = find_in_path(path) {
                return Ok(found);
            } else if path.exists() {
                return path.canonicalize().context("Failed to canonicalize PYTHON env path");
            }
        }
    }

    let candidates = if cfg!(windows) {
        vec!["python3.exe", "python.exe", "python3", "python"]
    } else {
        vec!["python3", "python"]
    };

    for candidate in candidates {
        if let Some(found) = find_in_path(Path::new(candidate)) {
            return Ok(found);
        }
    }

    anyhow::bail!(
        "Could not find a valid Python executable. Please specify using --python-path or set the PYTHON environment variable."
    )
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Starting migration from {:?} to {:?}", args.from, args.to);

    let temp_dir = if args.skip_extract {
        args.from.clone()
    } else {
        let dir = args.to.join(".migration_tmp");
        fs::create_dir_all(&dir)?;
        dir
    };

    if !args.skip_extract {
        let python_exec = resolve_python_executable(args.python_path.as_deref())?;
        let python_script = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("export_cpp.py");

        println!(
            "1. Extracting data and schema from C++ Akar (via Python: {:?})...",
            python_exec
        );
        let status = Command::new(&python_exec)
            .arg(&python_script)
            .arg("--db_path")
            .arg(&args.from)
            .arg("--out_dir")
            .arg(&temp_dir)
            .status()
            .context(
                "Failed to execute python extraction script. Make sure Python and akar/ladybug packages are installed.",
            )?;

        if !status.success() {
            anyhow::bail!("Python extraction script failed");
        }
    } else {
        println!("1. Skipping extraction, reading directly from {:?}", temp_dir);
    }

    println!("2. Connecting to Rust Akar Database...");
    let rust_db = std::sync::Arc::new(
        akar_main::Database::new(&args.to, akar_main::SystemConfig::default())
            .map_err(|e| anyhow::anyhow!("DB Init Error: {}", e))?,
    );
    let rust_conn = akar_main::Connection::new(&rust_db);

    let schema_file = temp_dir.join("schema.json");
    let schema_json = fs::read_to_string(&schema_file)?;
    let schema: Value = serde_json::from_str(&schema_json)?;

    let tables = schema["tables"]
        .as_array()
        .context("Invalid schema: tables is not an array")?;
    let connections = schema["connections"]
        .as_array()
        .context("Invalid schema: connections is not an array")?;

    println!("3. Reconstructing DDL and loading data...");

    // First pass: Create all Node Tables
    for table in tables {
        let table_type = table["type"].as_str().unwrap_or("");
        if table_type != "NODE" {
            continue;
        }

        let table_name = table["name"].as_str().unwrap();

        // Idempotency: skip CREATE + COPY for tables already present in the
        // destination DB (e.g. re-running migration on an already-migrated DB).
        if rust_db
            .get_table_id(table_name)
            .map_err(|e| anyhow::anyhow!(e))?
            .is_some()
        {
            println!("Skipping {} (already exists)", table_name);
            continue;
        }

        let properties = table["properties"].as_array().unwrap();

        let mut columns = Vec::new();
        let mut primary_key = String::new();

        for prop in properties {
            let col_name = prop["name"].as_str().unwrap();
            let col_type = prop["type"].as_str().unwrap();
            let is_pk = prop["is_primary_key"].as_bool().unwrap_or(false);

            columns.push(format!("{} {}", col_name, col_type));
            if is_pk {
                primary_key = col_name.to_string();
            }
        }

        let ddl = if !primary_key.is_empty() {
            format!(
                "CREATE NODE TABLE {} ({}, PRIMARY KEY({}))",
                table_name,
                columns.join(", "),
                primary_key
            )
        } else {
            format!("CREATE NODE TABLE {} ({})", table_name, columns.join(", "))
        };

        println!("Executing: {}", ddl);
        rust_conn.query(&ddl).map_err(|e| anyhow::anyhow!("DDL Error: {}", e))?;

        // Load data
        let parquet_path = temp_dir.join(format!("{}.parquet", table_name));
        let parquet_path_str = parquet_path.to_str().unwrap().replace("\\", "/");
        let import_query = format!("COPY {} FROM '{}'", table_name, parquet_path_str);

        println!("Importing data to {}...", table_name);
        rust_conn
            .query(&import_query)
            .map_err(|e| anyhow::anyhow!("COPY Error: {}", e))?;
    }

    // Second pass: Create all Rel Tables
    for table in tables {
        let table_type = table["type"].as_str().unwrap_or("");
        if table_type != "REL" {
            continue;
        }

        let table_name = table["name"].as_str().unwrap();

        // Idempotency: skip CREATE + COPY for tables already present in the
        // destination DB (e.g. re-running migration on an already-migrated DB).
        if rust_db
            .get_table_id(table_name)
            .map_err(|e| anyhow::anyhow!(e))?
            .is_some()
        {
            println!("Skipping {} (already exists)", table_name);
            continue;
        }

        // Find connection info
        let mut from_table = "UNKNOWN";
        let mut to_table = "UNKNOWN";
        for conn_info in connections {
            if conn_info["rel"].as_str() == Some(table_name) {
                from_table = conn_info["src"].as_str().unwrap();
                to_table = conn_info["dst"].as_str().unwrap();
                break;
            }
        }

        let properties = table["properties"].as_array().unwrap();
        let mut columns = Vec::new();
        for prop in properties {
            let col_name = prop["name"].as_str().unwrap();
            let col_type = prop["type"].as_str().unwrap();
            columns.push(format!("{} {}", col_name, col_type));
        }

        let props_str = if columns.is_empty() {
            String::new()
        } else {
            format!(", {}", columns.join(", "))
        };
        let ddl = format!(
            "CREATE REL TABLE {} (FROM {} TO {}{})",
            table_name, from_table, to_table, props_str
        );

        println!("Executing: {}", ddl);
        rust_conn.query(&ddl).map_err(|e| anyhow::anyhow!("DDL Error: {}", e))?;

        // Load data
        let parquet_path = temp_dir.join(format!("{}.parquet", table_name));
        let parquet_path_str = parquet_path.to_str().unwrap().replace("\\", "/");
        let import_query = format!("COPY {} FROM '{}'", table_name, parquet_path_str);

        println!("Importing data to {}...", table_name);
        rust_conn
            .query(&import_query)
            .map_err(|e| anyhow::anyhow!("COPY Error: {}", e))?;
    }

    // Cleanup
    if !args.skip_extract {
        println!("4. Cleaning up temporary files...");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    println!("Migration complete!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_python_executable() {
        // If system has python3 or python, resolve_python_executable should succeed and return an absolute path
        if let Ok(path) = resolve_python_executable(None) {
            assert!(path.is_absolute());
            assert!(path.is_file());
        }

        // Test with custom path that exists
        let temp_dir = tempfile::tempdir().unwrap();
        let fake_python = temp_dir.path().join("fake_python");
        fs::write(&fake_python, "dummy").unwrap();

        let resolved = resolve_python_executable(Some(&fake_python)).unwrap();
        assert!(resolved.is_absolute());

        // Test with non-existent custom path
        let non_existent = temp_dir.path().join("non_existent_python");
        assert!(resolve_python_executable(Some(&non_existent)).is_err());
    }
}
