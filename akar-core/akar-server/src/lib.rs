//! Akar embedded server — multi-process access over TCP (P47).
//!
//! One process owns the [`Database`] (and its exclusive file lock) while N
//! client processes connect over TCP to query the same database. This is the
//! multi-process access layer on top of the single-writer embedded engine:
//!
//! - The server serializes all writes through the existing [`TransactionManager`]
//!   (optimistic row-level conflicts surface as `WriteConflict` errors).
//! - Read-only clients use the normal MVCC snapshot path.
//! - Clients never open the database directory — the server holds every file
//!   lock on their behalf ([`crate::Database::connect_tcp`]).
//!
//! True shared-storage multi-process *writers* over the same files remain
//! deliberately out of scope: the storage format (durable column mirrors +
//! `BufferManager` mmap + `.ovf` sidecars) assumes a single owner, and a
//! distributed buffer-pool protocol is beyond an embedded database.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use akar_main::{Database, SystemConfig};
//! use akar_server::Server;
//!
//! let db = Arc::new(Database::new("./my_db", SystemConfig::default())?);
//! let mut server = Server::bind("127.0.0.1:9876", db)?;
//! server.start()?;
//! # Ok::<(), String>(())
//! ```

pub mod dream;
pub mod session;

use akar_main::database::Database;
use session::SessionConfig;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The Akar embedded server.
///
/// Owns a [`Database`] instance (and therefore the exclusive file lock) and
/// accepts TCP client connections. Each client gets its own
/// [`Connection`](akar_main::connection::Connection), so transactions are
/// per-session; the shared [`TransactionManager`](akar_transaction::TransactionManager)
/// serializes commit and detects row-level write-write conflicts.
///
/// Bind with port `0` to let the OS assign an ephemeral port; read it back via
/// [`Server::local_addr`].
pub struct Server {
    db: Arc<Database>,
    listener: TcpListener,
    local_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    accept_handle: Option<JoinHandle<()>>,
    client_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Shared last-activity timestamp (epoch seconds) for idle detection.
    last_activity: Arc<AtomicU64>,
    /// Total query counter (incremented per `query` op).
    total_queries: Arc<AtomicU64>,
    /// Database path (for stats responses).
    db_path: String,
    /// Optional auth token. If set, clients must include this in every request.
    auth_token: Option<String>,
    /// Optional idle timeout monitor thread.
    idle_handle: Option<JoinHandle<()>>,
}

impl Server {
    /// Bind the server to `addr` without starting it yet.
    pub fn bind<A: ToSocketAddrs>(addr: A, db: Arc<Database>) -> Result<Self, String> {
        let listener = TcpListener::bind(addr).map_err(|e| format!("Failed to bind Akar server: {e}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read local address: {e}"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(Self {
            db,
            listener,
            local_addr,
            shutdown: Arc::new(AtomicBool::new(false)),
            accept_handle: None,
            client_handles: Arc::new(Mutex::new(Vec::new())),
            last_activity: Arc::new(AtomicU64::new(now)),
            total_queries: Arc::new(AtomicU64::new(0)),
            db_path: String::new(),
            auth_token: None,
            idle_handle: None,
        })
    }

    /// The bound address. When bound to port `0`, this is the actual
    /// OS-assigned port after [`Server::start`].
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Set the database path (used in stats responses).
    pub fn set_db_path(&mut self, path: impl Into<String>) {
        self.db_path = path.into();
    }

    /// Set an authentication token. Clients must include this token in every
    /// request; connections without a valid token are rejected.
    pub fn set_auth_token(&mut self, token: impl Into<String>) {
        self.auth_token = Some(token.into());
    }

    /// Enable idle timeout monitoring. After `timeout` of inactivity (no client
    /// requests), the server shuts down gracefully.
    pub fn set_idle_timeout(&mut self, timeout: Duration) {
        let last_activity = self.last_activity.clone();
        let shutdown = self.shutdown.clone();
        let timeout_secs = timeout.as_secs();
        let handle = thread::Builder::new()
            .name("akar-server-idle".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    if shutdown.load(Ordering::SeqCst) {
                        return;
                    }
                    let last = last_activity.load(Ordering::Relaxed);
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if now.saturating_sub(last) >= timeout_secs {
                        tracing::info!("Idle timeout ({timeout_secs}s) reached — initiating shutdown");
                        shutdown.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            })
            .ok();
        self.idle_handle = handle;
    }

    /// Number of currently active client sessions.
    ///
    /// Finished client threads are pruned from the handle list, so the count
    /// reflects live sessions rather than the cumulative number ever accepted
    /// (and the backing `Vec` never grows unboundedly).
    pub fn num_clients(&self) -> usize {
        let mut handles = self.client_handles.lock().unwrap_or_else(|p| p.into_inner());
        handles.retain(|h| !h.is_finished());
        handles.len()
    }

    /// Total number of queries executed since the server started.
    pub fn total_queries(&self) -> u64 {
        self.total_queries.load(Ordering::Relaxed)
    }

    /// Stop the accept loop and wait for all client sessions to exit.
    ///
    /// Client threads observe the shutdown flag between frames and exit (the
    /// per-session read timeout bounds the wait when a client is idle). Once
    /// this returns, every client thread has finished, so the database can be
    /// reopened elsewhere immediately.
    pub fn shutdown(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(handle) = self.idle_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
        // Drain repeatedly: the accept loop may have pushed a handle in the
        // small window between the shutdown flag and the accept loop actually
        // stopping. Each join is bounded by the session's read timeout.
        loop {
            let finished = {
                let mut guard = self.client_handles.lock().unwrap_or_else(|p| p.into_inner());
                if guard.is_empty() {
                    break;
                }
                std::mem::take(&mut *guard)
            };
            for handle in finished {
                let _ = handle.join();
            }
        }
    }

    /// Start accepting client connections on a background thread.
    ///
    /// Non-blocking: returns as soon as the accept loop is running.
    pub fn start(&mut self) -> Result<(), String> {
        if self.accept_handle.is_some() {
            return Err("Server is already running".to_string());
        }
        let listener = self
            .listener
            .try_clone()
            .map_err(|e| format!("Failed to clone listener: {e}"))?;
        let _ = listener.set_nonblocking(true);
        let db = self.db.clone();
        let shutdown = self.shutdown.clone();
        let client_handles = self.client_handles.clone();
        let last_activity = self.last_activity.clone();
        let total_queries = self.total_queries.clone();
        let db_path = self.db_path.clone();
        let auth_token = self.auth_token.clone();
        let handle = thread::Builder::new()
            .name("akar-server-accept".into())
            .spawn(move || {
                accept_loop(
                    listener,
                    db,
                    shutdown,
                    client_handles,
                    last_activity,
                    total_queries,
                    db_path,
                    auth_token,
                )
            })
            .map_err(|e| format!("Failed to spawn accept thread: {e}"))?;
        self.accept_handle = Some(handle);
        tracing::info!("Akar server listening on {}", self.local_addr);
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(
    listener: TcpListener,
    db: Arc<Database>,
    shutdown: Arc<AtomicBool>,
    client_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    last_activity: Arc<AtomicU64>,
    total_queries: Arc<AtomicU64>,
    db_path: String,
    auth_token: Option<String>,
) {
    let dream = dream::DreamControl::new();
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let db = db.clone();
                let shutdown = shutdown.clone();
                let handles = client_handles.clone();
                let last_activity = last_activity.clone();
                let total_queries = total_queries.clone();
                let db_path = db_path.clone();
                let dream = dream.clone();
                let config = SessionConfig {
                    auth_token: auth_token.clone(),
                    last_activity,
                    total_queries,
                    db_path,
                    shutdown: shutdown.clone(),
                    dream,
                };
                match thread::Builder::new().name("akar-server-client".into()).spawn(move || {
                    tracing::debug!("Client connected: {peer}");
                    session::handle_client(stream, db, &config);
                    tracing::debug!("Client disconnected: {peer}");
                }) {
                    Ok(handle) => {
                        if let Ok(mut guard) = handles.lock() {
                            guard.push(handle);
                        }
                    }
                    Err(e) => tracing::warn!("Failed to spawn client thread: {e}"),
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                tracing::warn!("Accept error: {e}");
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
