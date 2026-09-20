//! Connection pooling over a shared `Arc<Database>` (P123.1).
//!
//! Embedding Akar in a multi-threaded host — a Tokio daemon such as
//! `sulur-server`, a worker pool, an HTTP handler — needs two things the base
//! API deliberately leaves to the caller.
//!
//! 1. **One `Database` behind an `Arc`.** [`Database::new`] takes the file lock
//!    for its path, so a process must hold exactly one. Everything else shares
//!    it.
//! 2. **A pool of [`Connection`]s.** A `Connection` is not just a handle: it
//!    owns a statement cache, a plan cache and a transaction context, and it is
//!    not meant to be driven by two threads at once. Creating one per request
//!    throws away the plan cache that makes repeated Cypher cheap.
//!
//! [`ConnectionPool`] supplies (2) on top of (1): it hands out checked-out
//! connections and takes them back on drop, so a caller can use one for the
//! duration of a query or a transaction and give it back.
//!
//! # Using this from async code
//!
//! Akar is a synchronous engine — deliberately so, and this crate does not
//! depend on an async runtime (a database library has no business choosing one
//! for its host). In a Tokio host, hand the pool to
//! `tokio::task::spawn_blocking` rather than awaiting a query directly:
//!
//! ```text
//! let pool = Arc::new(ConnectionPool::new(db));
//! let pool_for_task = pool.clone();
//! let result = tokio::task::spawn_blocking(move || {
//!     let conn = pool_for_task.get();
//!     conn.query("MATCH (m:Memory) RETURN count(m) AS n")
//! })
//! .await?;
//! ```
//!
//! The pool is `Send + Sync`, so cloning the `Arc` into each closure is the
//! whole async story; the `async fn` wrapper belongs in the host, where the
//! runtime already lives.
//!
//! A synchronous host uses the same pool without any of that:
//!
//! ```
//! # use std::sync::Arc;
//! # use akar_main::{ConnectionPool, Database, SystemConfig};
//! let db = Arc::new(Database::new("/tmp/db", SystemConfig::default())?);
//! let pool = ConnectionPool::new(db);
//!
//! let result = pool.run(|conn| conn.query("RETURN 1 AS one"))?;
//! assert!(result.success);
//! assert_eq!(pool.stats().idle, 1, "the connection came back");
//! # Ok::<(), String>(())
//! ```
//!
//! # Abandoned transactions
//!
//! A pooled connection that is returned while an explicit transaction is still
//! open would poison the pool: `BEGIN` is connection-scoped, and DDL inside a
//! transaction cannot be rolled back (see `Connection`'s `Drop`). Such a
//! connection is therefore **discarded** rather than pooled, which runs the
//! normal abandoned-transaction rollback in `Drop`.

use crate::connection::Connection;
use crate::database::Database;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Tunables for a [`ConnectionPool`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// How many idle connections to keep for reuse.
    ///
    /// Connections beyond this are closed when returned instead of parked, so
    /// the pool tracks a burst of concurrent work without pinning every
    /// connection's caches forever. Defaults to 8.
    pub max_idle: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self { max_idle: 8 }
    }
}

/// Counters describing a pool's activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// Connections currently parked and available for reuse.
    pub idle: usize,
    /// Connections created over the pool's lifetime.
    pub created: u64,
    /// How many times a connection has been handed out.
    pub checked_out: u64,
    /// Connections discarded on return because they were poisoned (see the
    /// module docs on abandoned transactions) or because the idle cap was hit.
    pub discarded: u64,
}

/// A pool of reusable [`Connection`]s over one shared `Arc<Database>`.
///
/// Cheap to share behind an `Arc`; every method takes `&self`.
pub struct ConnectionPool {
    database: Arc<Database>,
    config: PoolConfig,
    idle: Mutex<Vec<Connection>>,
    created: AtomicU64,
    checked_out: AtomicU64,
    discarded: AtomicU64,
}

impl std::fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("ConnectionPool")
            .field("max_idle", &self.config.max_idle)
            .field("idle", &stats.idle)
            .field("created", &stats.created)
            .field("checked_out", &stats.checked_out)
            .field("discarded", &stats.discarded)
            .finish()
    }
}

impl ConnectionPool {
    /// Create a pool over `database` with [`PoolConfig::default`].
    pub fn new(database: Arc<Database>) -> Self {
        Self::with_config(database, PoolConfig::default())
    }

    /// Create a pool over `database` with an explicit configuration.
    pub fn with_config(database: Arc<Database>, config: PoolConfig) -> Self {
        Self {
            database,
            config,
            idle: Mutex::new(Vec::new()),
            created: AtomicU64::new(0),
            checked_out: AtomicU64::new(0),
            discarded: AtomicU64::new(0),
        }
    }

    /// The shared database every connection in this pool attaches to.
    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    /// A clone of the shared database handle, for handing to another thread.
    pub fn database_handle(&self) -> Arc<Database> {
        Arc::clone(&self.database)
    }

    /// Check out a connection.
    ///
    /// Reuses an idle connection when one is available, otherwise creates a new
    /// one. The connection returns to the pool when the guard is dropped.
    ///
    /// A poisoned lock (a panic while another thread held it) degrades to a
    /// fresh connection rather than propagating the panic — the pool is a
    /// convenience, never a correctness dependency of the query path.
    pub fn get(&self) -> PooledConnection<'_> {
        let reused = match self.idle.lock() {
            Ok(mut idle) => idle.pop(),
            Err(poisoned) => poisoned.into_inner().pop(),
        };
        self.checked_out.fetch_add(1, Ordering::Relaxed);

        let connection = match reused {
            Some(connection) => connection,
            None => {
                self.created.fetch_add(1, Ordering::Relaxed);
                Connection::new(&self.database)
            }
        };

        PooledConnection {
            pool: self,
            connection: Some(connection),
        }
    }

    /// Run `f` with a pooled connection and return its result.
    ///
    /// Sugar for [`Self::get`] that keeps the check-out scoped to one call,
    /// which is the common shape in a request handler.
    pub fn run<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        let connection = self.get();
        f(&connection)
    }

    /// Number of connections currently parked for reuse.
    pub fn idle_count(&self) -> usize {
        match self.idle.lock() {
            Ok(idle) => idle.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// A snapshot of the pool's counters.
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            idle: self.idle_count(),
            created: self.created.load(Ordering::Relaxed),
            checked_out: self.checked_out.load(Ordering::Relaxed),
            discarded: self.discarded.load(Ordering::Relaxed),
        }
    }

    /// Return `connection` to the pool, or drop it when it must not be reused.
    fn release(&self, connection: Connection) {
        // An explicit transaction left open makes the connection unusable for
        // the next borrower. Dropping it here triggers `Connection::drop`,
        // which rolls the transaction back and frees its table locks.
        if connection.explicit_txn_active.load(Ordering::Acquire) {
            self.discarded.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let mut idle = match self.idle.lock() {
            Ok(idle) => idle,
            Err(poisoned) => poisoned.into_inner(),
        };
        if idle.len() >= self.config.max_idle {
            drop(idle);
            self.discarded.fetch_add(1, Ordering::Relaxed);
            return;
        }
        idle.push(connection);
    }
}

/// A checked-out [`Connection`] that returns itself to its pool on drop.
///
/// Dereferences to `Connection`, so it can be used wherever a `&Connection` is
/// expected.
pub struct PooledConnection<'a> {
    pool: &'a ConnectionPool,
    connection: Option<Connection>,
}

impl PooledConnection<'_> {
    /// The shared database this connection attaches to.
    pub fn database(&self) -> &Arc<Database> {
        self.pool.database()
    }
}

impl Deref for PooledConnection<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        // `connection` is only taken in `Drop`, so it is present for the whole
        // lifetime of a live guard.
        self.connection
            .as_ref()
            .expect("pooled connection is present until dropped")
    }
}

impl DerefMut for PooledConnection<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("pooled connection is present until dropped")
    }
}

impl Drop for PooledConnection<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.pool.release(connection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Arc<Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db =
            Arc::new(Database::new(dir.path().to_str().unwrap(), crate::SystemConfig::default()).expect("open db"));
        (db, dir)
    }

    #[test]
    fn the_pool_and_its_guards_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ConnectionPool>();
        assert_send_sync::<Connection>();
        assert_send_sync::<PoolStats>();
    }

    #[test]
    fn a_connection_is_reused_after_the_guard_drops() {
        let (db, _dir) = temp_db();
        let pool = ConnectionPool::new(db);

        assert_eq!(pool.stats().created, 0);
        {
            let _conn = pool.get();
            assert_eq!(pool.stats().created, 1);
            assert_eq!(pool.idle_count(), 0, "a checked-out connection is not idle");
        }
        assert_eq!(pool.idle_count(), 1, "the guard returned its connection");

        {
            let _conn = pool.get();
            assert_eq!(pool.stats().created, 1, "the second check-out must reuse, not create");
        }
        assert_eq!(pool.stats().checked_out, 2);
    }

    #[test]
    fn the_idle_cap_bounds_reuse_instead_of_the_pool() {
        let (db, _dir) = temp_db();
        let pool = ConnectionPool::with_config(db, PoolConfig { max_idle: 1 });

        let first = pool.get();
        let second = pool.get();
        drop(first);
        drop(second);

        assert_eq!(pool.idle_count(), 1, "only one connection may be parked");
        assert_eq!(pool.stats().created, 2);
        assert_eq!(pool.stats().discarded, 1, "the connection over the cap is closed");
        // The pool stays usable after discarding.
        assert!(pool.get().query("RETURN 1 AS one").is_ok());
    }

    #[test]
    fn run_scopes_the_check_out_to_the_call() {
        let (db, _dir) = temp_db();
        let pool = ConnectionPool::new(db);

        let result = pool.run(|conn| conn.query("RETURN 1 AS one"));
        assert!(result.expect("query runs").success);
        assert_eq!(pool.idle_count(), 1, "run() hands the connection back");
        assert_eq!(pool.stats().checked_out, 1);
    }

    #[test]
    fn a_pooled_connection_keeps_its_plan_cache_across_borrows() {
        // The point of pooling: the second borrow of the same connection finds
        // the plan already cached instead of re-planning from scratch.
        let (db, _dir) = temp_db();
        let pool = ConnectionPool::new(db);
        let sql = "RETURN 1 AS one";

        let first_size = {
            let conn = pool.get();
            conn.query(sql).expect("first query");
            conn.plan_cache_size()
        };
        assert!(first_size > 0, "the first run populates the plan cache");

        let conn = pool.get();
        assert_eq!(
            conn.plan_cache_size(),
            first_size,
            "the reused connection still holds its plan cache"
        );
        conn.query(sql).expect("second query");
    }

    #[test]
    fn a_connection_abandoned_mid_transaction_is_not_pooled() {
        let (db, _dir) = temp_db();
        let pool = ConnectionPool::new(db);
        pool.run(|conn| {
            conn.query("CREATE NODE TABLE PoolCheck (id INT64, PRIMARY KEY (id))")
                .expect("table")
        });

        {
            let conn = pool.get();
            conn.query("BEGIN TRANSACTION").expect("begin");
            // The guard drops with the transaction still open.
        }

        assert_eq!(pool.idle_count(), 0, "a poisoned connection must not be reused");
        assert_eq!(pool.stats().discarded, 1);

        // The discard performed the rollback, so the database is usable again.
        let conn = pool.get();
        let result = conn.query("CREATE (:PoolCheck {id: 1})").expect("write after rollback");
        assert!(result.success);
    }

    #[test]
    fn concurrent_borrows_share_the_database() {
        // std::thread rather than Tokio: the pool's contract is that it is
        // Send + Sync, and this is the property an async host relies on.
        let (db, _dir) = temp_db();
        let pool = Arc::new(ConnectionPool::new(db));
        pool.run(|conn| {
            conn.query("CREATE NODE TABLE PoolCheck (id INT64, PRIMARY KEY (id))")
                .expect("table")
        });
        pool.run(|conn| conn.query("CREATE (:PoolCheck {id: 1})").expect("seed"));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let pool = Arc::clone(&pool);
                std::thread::spawn(move || {
                    let conn = pool.get();
                    let result = conn.query("MATCH (p:PoolCheck) RETURN count(p) AS n").expect("count");
                    result.success
                })
            })
            .collect();

        for handle in handles {
            assert!(handle.join().expect("thread joins"));
        }

        let stats = pool.stats();
        assert!(
            stats.created <= 4,
            "the pool created no more connections than borrowers: {stats:?}"
        );
        assert_eq!(stats.checked_out, 6, "two setup check-outs plus four concurrent ones");
    }
}
