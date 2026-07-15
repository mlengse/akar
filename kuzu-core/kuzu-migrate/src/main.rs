use clap::Parser;
use std::path::PathBuf;
use std::fs;
use anyhow::{Result, Context};

/// Kuzu C++ to Rust database migration tool
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the source C++ database directory
    #[arg(short, long)]
    from: PathBuf,

    /// Path to the destination Rust database directory
    #[arg(short, long)]
    to: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    println!("Starting migration from {:?} to {:?}", args.from, args.to);
    
    // 1. Open C++ DB
    let cpp_db = kuzu::Database::new(&args.from, kuzu::SystemConfig::default())?;
    let cpp_conn = kuzu::Connection::new(&cpp_db)?;
    
    // 2. Open Rust DB
    let rust_db = std::sync::Arc::new(
        kuzu_main::Database::new(&args.to, kuzu_main::SystemConfig::default())
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
    );
    let rust_conn = kuzu_main::Connection::new(&rust_db);
    
    // 3. Extract schema from C++ DB
    println!("Extracting schema...");
    let mut result = cpp_conn.query("CALL show_tables() RETURN *")?;
    let mut table_names = Vec::new();
    
    for row in result {
        if let kuzu::Value::String(name) = &row[1] { // row[0] is id, row[1] is name
            table_names.push(name.clone());
        }
    }
    
    for table in &table_names {
        println!("Migrating table: {}", table);
        
        let mut info_res = cpp_conn.query(&format!("CALL table_info('{}') RETURN *", table))?;
        let mut columns = Vec::new();
        let mut primary_key = String::new();

        for info_row in info_res {
            let col_name = if let kuzu::Value::String(s) = &info_row[1] { s.clone() } else { continue };
            let col_type = if let kuzu::Value::String(s) = &info_row[2] { s.clone() } else { continue };
            let is_pk = if let kuzu::Value::Bool(b) = &info_row[3] { *b } else { false };
            
            columns.push(format!("{} {}", col_name, col_type));
            if is_pk {
                primary_key = col_name;
            }
        }
        
        let ddl = if !primary_key.is_empty() {
            format!("CREATE NODE TABLE {} ({}, PRIMARY KEY({}));", table, columns.join(", "), primary_key)
        } else {
            // Rel tables don't have PKs in the same way, but let's assume nodes for MVP
            format!("CREATE NODE TABLE {} ({});", table, columns.join(", "))
        };
        
        println!("Executing: {}", ddl);
        rust_conn.query(&ddl).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        
        // 4. Dump to Parquet
        let temp_parquet = format!("{}_temp.parquet", table);
        let export_query = format!("COPY (MATCH (a:{}) RETURN a.*) TO '{}'", table, temp_parquet);
        println!("Exporting data...");
        cpp_conn.query(&export_query)?;
        
        // 5. COPY FROM into Rust DB
        let import_query = format!("COPY {} FROM '{}'", table, temp_parquet);
        println!("Importing data...");
        rust_conn.query(&import_query).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        
        let _ = fs::remove_file(&temp_parquet);
    }
    
    println!("Migration complete!");
    Ok(())
}
