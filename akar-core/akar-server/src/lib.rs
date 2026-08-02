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

pub mod session;

use akar_main::database::Database;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
}

impl Server {
    /// Bind the server to `addr` without starting it yet.
    pub fn bind<A: ToSocketAddrs>(addr: A, db: Arc<Database>) -> Result<Self, String> {
        let listener =
            TcpListener::bind(addr).map_err(|e| format!("Failed to bind Akar server: {e}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read local address: {e}"))?;
        Ok(Self {
            db,
            listener,
            local_addr,
            shutdown: Arc::new(AtomicBool::new(false)),
            accept_handle: None,
            client_handles: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The bound address. When bound to port `0`, this is the actual
    /// OS-assigned port after [`Server::start`].
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
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
        let handle = thread::Builder::new()
            .name("akar-server-accept".into())
            .spawn(move || accept_loop(listener, db, shutdown, client_handles))
            .map_err(|e| format!("Failed to spawn accept thread: {e}"))?;
        self.accept_handle = Some(handle);
        tracing::info!("Akar server listening on {}", self.local_addr);
        Ok(())
    }

    /// Number of currently accepted client sessions.
    pub fn num_clients(&self) -> usize {
        self.client_handles.lock().map(|h| h.len()).unwrap_or(0)
    }

    /// Stop the accept loop.
    ///
    /// Active client sessions keep running — they observe the shutdown flag
    /// between frames and exit when the peer disconnects. Active connections
    /// must be closed by their clients (or dropped) before the database can be
    /// reopened elsewhere.
    pub fn shutdown(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
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
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let db = db.clone();
                let shutdown = shutdown.clone();
                let handles = client_handles.clone();
                match thread::Builder::new()
                    .name("akar-server-client".into())
                    .spawn(move || {
                        tracing::debug!("Client connected: {peer}");
                        session::handle_client(stream, db, &shutdown);
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
