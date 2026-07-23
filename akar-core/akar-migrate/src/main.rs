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
        let python_script = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("export_cpp.py");

        println!("1. Extracting data and schema from C++ Akar (via Python)...");
        let status = Command::new("python")
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
