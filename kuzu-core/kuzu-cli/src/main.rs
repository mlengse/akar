//! Kuzu CLI — interactive and script-mode Cypher query shell.
//!
//! Usage:
//!   kuzu-cli [database_path]
//!
//! If no path is given, runs in `:memory:` mode.
//! Supports reading SQL/Cypher statements from stdin.

use kuzu_main::{Connection, Database, SystemConfig};
use std::io::{self, BufRead, Write};
use std::sync::Arc;

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).init();

    // Parse arguments
    let args: Vec<String> = std::env::args().collect();
    let db_path = if args.len() > 1 {
        args[1].clone()
    } else {
        ":memory:".to_string()
    };

    // Initialize database
    let db = match Database::new(&db_path, SystemConfig::default()) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("Error creating database: {e}");
            std::process::exit(1);
        }
    };
    let conn = Connection::new(&db);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    // If stdin is a terminal, run interactive REPL
    if atty::is(atty::Stream::Stdin) {
        run_repl(&conn, &mut stdout);
    } else {
        // Otherwise process piped input
        run_script(&conn, stdin.lock(), &mut stdout);
    }
}

/// Interactive REPL mode.
fn run_repl(conn: &Connection, output: &mut dyn Write) {
    let mut input = String::new();
    let stdin = io::stdin();

    writeln!(output, "Kuzu CLI v{}", env!("CARGO_PKG_VERSION")).ok();
    writeln!(output, "Enter Cypher queries. Type 'exit' to quit, 'help' for help.").ok();
    writeln!(output).ok();

    loop {
        write!(output, "kuzu> ").ok();
        output.flush().ok();

        input.clear();
        if stdin.lock().read_line(&mut input).is_err() || input.trim().is_empty() {
            continue;
        }

        let trimmed = input.trim();

        match trimmed.to_lowercase().as_str() {
            "exit" | "quit" => {
                writeln!(output, "Bye!").ok();
                break;
            }
            "help" => {
                writeln!(output, "Commands:").ok();
                writeln!(output, "  Cypher query  Execute a Cypher query").ok();
                writeln!(output, "  exit/quit    Exit the shell").ok();
                writeln!(output, "  help         Show this help").ok();
                writeln!(output).ok();
                continue;
            }
            _ => {}
        }

        match conn.query(trimmed) {
            Ok(result) => {
                if result.is_success() {
                    writeln!(output, "{}", result.summary()).ok();
                    // Print data chunks in a simple tabular format
                    for chunk in &result.chunks {
                        for row in 0..chunk.size {
                            let vals: Vec<String> = chunk.fields.iter().map(|v| format_value(v, row)).collect();
                            writeln!(output, "| {} |", vals.join(" | ")).ok();
                        }
                    }
                    if let Some(msg) = &result.message {
                        writeln!(output, "{msg}").ok();
                    }
                } else {
                    writeln!(
                        output,
                        "Error: {}",
                        result.error_message.as_deref().unwrap_or("Unknown error")
                    )
                    .ok();
                }
            }
            Err(e) => {
                writeln!(output, "Error: {e}").ok();
            }
        }
    }
}

/// Script mode: process statements from stdin.
fn run_script(conn: &Connection, reader: impl BufRead, output: &mut dyn Write) {
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") || trimmed.starts_with("//") {
            continue;
        }

        match conn.query(trimmed) {
            Ok(result) => {
                if !result.is_success() {
                    writeln!(
                        output,
                        "Error: {}",
                        result.error_message.as_deref().unwrap_or("Unknown error")
                    )
                    .ok();
                }
            }
            Err(e) => {
                writeln!(output, "Error: {e}").ok();
            }
        }
    }
}

/// Format a ValueVector cell to string.
fn format_value(v: &kuzu_common::vector::ValueVector, row: usize) -> String {
    if v.is_null(row) {
        return "NULL".into();
    }
    match v.physical_type() {
        kuzu_common::types::PhysicalTypeID::Int64
        | kuzu_common::types::PhysicalTypeID::Int32
        | kuzu_common::types::PhysicalTypeID::Int16 => v.get_i64(row).map_or("NULL".into(), |n| n.to_string()),
        kuzu_common::types::PhysicalTypeID::Double | kuzu_common::types::PhysicalTypeID::Float => {
            v.get_double(row).map_or("NULL".into(), |n| format!("{:.4}", n))
        }
        kuzu_common::types::PhysicalTypeID::Bool => v.get_bool(row).map_or("NULL".into(), |b| b.to_string()),
        _ => format!("<{:?}>", v.physical_type()),
    }
}
