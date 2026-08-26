//! `akar-server` daemon binary (P62).
//!
//! A standalone process that owns the Akar `Database` and serves TCP clients.
//! Designed to replace the Python daemon (`kairos.kuzu_daemon`) as the single
//! DB owner, eliminating race conditions between interpreters.
//!
//! # Usage
//!
//! ```text
//! akar-server --db <path> [--port 9876] [--addr 127.0.0.1]
//!             [--auth-token <hex>] [--idle 86400] [--json-sidecar <path>]
//! ```

use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Parser;
use serde::{Deserialize, Serialize};

use akar_main::{Database, SystemConfig};
use akar_server::Server;

/// Akar embedded database server daemon.
#[derive(Parser)]
#[command(
    name = "akar-server",
    about = "Akar embedded database server — TCP daemon for multi-client access"
)]
struct Args {
    /// Path to the database directory.
    #[arg(long)]
    db: PathBuf,

    /// TCP port to listen on (default: 9876).
    #[arg(long, default_value_t = 9876)]
    port: u16,

    /// Address to bind to (default: 127.0.0.1).
    #[arg(long, default_value = "127.0.0.1")]
    addr: String,

    /// 32-byte hex-encoded authentication token. If empty, auth is disabled
    /// and any local client may connect.
    #[arg(long)]
    auth_token: Option<String>,

    /// Idle timeout in seconds. Server shuts down after this many seconds of
    /// inactivity (no client requests). 0 = no timeout (default).
    #[arg(long, default_value_t = 0)]
    idle: u64,

    /// Path to write the JSON sidecar file on startup (removed on shutdown).
    /// The sidecar allows clients to discover the running server.
    #[arg(long)]
    json_sidecar: Option<PathBuf>,

    /// Enable read-only mode (no writes accepted).
    #[arg(long)]
    read_only: bool,

    /// WAL size in bytes that triggers an auto-checkpoint. Defaults to a
    /// positive threshold so the daemon checkpoints only when the WAL grows
    /// past this size, NOT after every write (the historical -1 default
    /// caused ~1s persist_all_tables rewrites per write). Set to 0 to
    /// disable auto-checkpoint, or -1 to restore checkpoint-per-write.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    checkpoint_threshold: i64,
}

/// Sidecar file written to `--json-sidecar` path on startup.
#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    /// Protocol version (currently "json-v1").
    protocol: String,
    /// Bound address (host:port).
    host: String,
    /// TCP port.
    port: u16,
    /// Auth token (hex), or empty if auth is disabled.
    token: String,
    /// Server process ID.
    pid: u32,
    /// Database path.
    db_path: String,
    /// Server start time (epoch seconds).
    started_at: u64,
}

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let args = Args::parse();

    // Filter out empty auth token.
    let auth_token = args.auth_token.filter(|t| !t.is_empty());

    // Open database.
    let config = SystemConfig {
        auto_checkpoint: true,
        checkpoint_threshold: args.checkpoint_threshold,
        concurrent_writes: true,
        read_only: args.read_only,
        ..Default::default()
    };
    let db = match Database::new(&args.db, config) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("Failed to open database at '{}': {e}", args.db.display());
            process::exit(1);
        }
    };

    // Bind server.
    let bind_addr = format!("{}:{}", args.addr, args.port);
    let mut server = match Server::bind(&bind_addr, db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to bind server to '{bind_addr}': {e}");
            process::exit(1);
        }
    };

    server.set_db_path(args.db.to_string_lossy().to_string());
    if let Some(ref token) = auth_token {
        server.set_auth_token(token.clone());
    }
    if args.idle > 0 {
        server.set_idle_timeout(Duration::from_secs(args.idle));
    }

    // Write sidecar file.
    let sidecar_path = args.json_sidecar.clone();
    if let Some(ref path) = sidecar_path {
        let sidecar = Sidecar {
            protocol: "json-v1".to_string(),
            host: server.local_addr().to_string(),
            port: server.local_addr().port(),
            token: auth_token.clone().unwrap_or_default(),
            pid: process::id(),
            db_path: args.db.to_string_lossy().to_string(),
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        let json = serde_json::to_string_pretty(&sidecar).expect("sidecar serialization");
        if let Err(e) = fs::write(path, &json) {
            eprintln!("Failed to write sidecar to '{}': {e}", path.display());
            process::exit(1);
        }
        tracing::info!("Sidecar written to {}", path.display());
    }

    // Start accepting connections.
    if let Err(e) = server.start() {
        eprintln!("Failed to start server: {e}");
        remove_sidecar(&sidecar_path);
        process::exit(1);
    }

    let local_addr = server.local_addr();
    println!("Akar server listening on {local_addr}");
    println!("Database: {}", args.db.display());
    if auth_token.is_some() {
        println!("Authentication: enabled");
    } else {
        println!("Authentication: disabled (local only)");
    }
    if args.idle > 0 {
        println!("Idle timeout: {}s", args.idle);
    }

    // Wait for shutdown signal (Ctrl+C).
    wait_for_signal();

    println!("\nShutting down...");
    server.shutdown();
    remove_sidecar(&sidecar_path);
    println!("Server stopped.");
}

/// Block until Ctrl+C (SIGINT) is received.
fn wait_for_signal() {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .expect("Error setting Ctrl-C handler");
    let _ = rx.recv();
}

/// Remove the sidecar file if it exists.
fn remove_sidecar(path: &Option<PathBuf>) {
    if let Some(p) = path.as_ref() {
        if p.exists() {
            if let Err(e) = fs::remove_file(p) {
                eprintln!("Warning: failed to remove sidecar '{}': {e}", p.display());
            } else {
                tracing::info!("Sidecar removed: {}", p.display());
            }
        }
    }
}
