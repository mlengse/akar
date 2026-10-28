//! Task system / thread pool for parallel query execution.
//!
//! Uses `rayon` under the hood for work-stealing parallelism.

use rayon::ThreadPool;
use std::sync::Arc;

/// A handle to Akar's task execution system.
#[derive(Clone)]
pub struct TaskSystem {
    pool: Arc<ThreadPool>,
    num_threads: usize,
}

impl TaskSystem {
    /// Create a new task system with the given number of threads.
    /// If `num_threads` is 0, uses rayon's default (logical CPU count).
    pub fn new(num_threads: usize) -> Self {
        let num = if num_threads == 0 {
            rayon::current_num_threads()
        } else {
            num_threads
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num)
            .thread_name(|i| format!("akar-worker-{i}"))
            .build()
            .or_else(|_| rayon::ThreadPoolBuilder::new().num_threads(1).build())
            .unwrap_or_else(|e| {
                tracing::error!("Failed to build rayon thread pool: {e}");
                match rayon::ThreadPoolBuilder::new().num_threads(1).build() {
                    Ok(p) => p,
                    Err(_) => unreachable!("Failed to build 1-thread rayon pool"),
                }
            });
        let pool = Arc::new(pool);
        Self { pool, num_threads: num }
    }

    pub fn num_threads(&self) -> usize {
        self.num_threads
    }

    /// Execute a parallel operation across the thread pool.
    pub fn install<F, R>(&self, op: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        self.pool.install(op)
    }
}

impl Default for TaskSystem {
    fn default() -> Self {
        Self::new(0)
    }
}
