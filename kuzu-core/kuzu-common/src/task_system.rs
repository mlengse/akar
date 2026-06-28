//! Task system / thread pool for parallel query execution.
//!
//! Uses `rayon` under the hood for work-stealing parallelism.

use rayon::ThreadPool;
use std::sync::Arc;

/// A handle to Kuzu's task execution system.
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
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(num)
                .thread_name(|i| format!("kuzu-worker-{i}"))
                .build()
                .expect("Failed to build rayon thread pool"),
        );
        Self {
            pool,
            num_threads: num,
        }
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
