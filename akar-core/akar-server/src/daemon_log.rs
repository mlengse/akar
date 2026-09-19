//! Daemon log hardening (P114.3 — FINDINGS F9 item 2).
//!
//! On the Sulur host several `akar-server` spawns terminated without leaving
//! any panic/abort/`memory allocation … failed` trace, so the reason for
//! termination could not be determined. Two hooks make abnormal termination
//! observable in the supervisor's captured stderr:
//!
//! - A **panic hook** that records a timestamped, pid-tagged line (with
//!   location and thread) and flushes stdout + stderr before delegating to the
//!   default hook.
//! - A **global allocator wrapper** that records allocation failure — the
//!   `memory allocation of N bytes failed` path that previously only reached
//!   the default OOM handler, whose output may be lost if the process is torn
//!   down immediately afterwards.
//!
//! Both write straight to stderr (unbuffered in Rust) with an explicit flush,
//! and the allocator path formats directly into the writer so logging an OOM
//! never allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{self, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Epoch milliseconds, or 0 if the clock is set before the Unix epoch.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Write one line to stderr and flush it immediately. Used for lifecycle
/// markers (`START`/`EXIT`) so a spawn that never logs `EXIT` is visibly
/// abnormal.
pub fn log_line(tag: &str, message: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(
        err,
        "[akar-server] t={} pid={} {tag}: {message}",
        now_ms(),
        std::process::id()
    );
    let _ = err.flush();
}

/// Install a panic hook that records the panic on stderr before the default
/// hook runs (which preserves the standard `thread '…' panicked at …` line).
pub fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        log_line("PANIC", &format!("{payload} at {location} (thread '{thread_name}')"));
        // tracing_subscriber's default fmt layer writes to stdout; flush it so
        // buffered prior lines are not lost when the process aborts.
        let _ = io::stdout().flush();
        default(info);
    }));
}

/// Set once the first allocation failure has been logged, so a failure that
/// occurs *while* logging cannot recurse.
static OOM_LOGGED: AtomicBool = AtomicBool::new(false);

/// Format directly into the unbuffered stderr writer (no heap allocation).
fn log_oom(op: &str, size: usize) {
    if OOM_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut err = io::stderr().lock();
    let _ = writeln!(
        err,
        "[akar-server] t={} pid={} OOM: {op} failed (size={size} bytes); aborting",
        now_ms(),
        std::process::id()
    );
    let _ = err.flush();
}

/// Global allocator wrapper that logs allocation failure before returning null
/// to the caller (which then aborts via the default OOM handler).
pub struct LoggingAllocator;

// SAFETY: every method forwards to `System` unchanged; the only added work is
// a non-allocating stderr write when the underlying allocator returns null.
unsafe impl GlobalAlloc for LoggingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if ptr.is_null() {
            log_oom("alloc", layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if ptr.is_null() {
            log_oom("alloc_zeroed", layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if new_ptr.is_null() {
            log_oom("realloc", new_size);
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_positive() {
        assert!(now_ms() > 0, "epoch milliseconds must be positive");
    }

    /// The wrapper must forward every operation to `System` unchanged — only a
    /// null result triggers the extra (non-allocating) log write.
    #[test]
    fn logging_allocator_delegates_to_system() {
        let alloc = LoggingAllocator;
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { alloc.alloc(layout) };
        assert!(!ptr.is_null(), "allocation must succeed");
        let grown = unsafe { alloc.realloc(ptr, layout, 128) };
        assert!(!grown.is_null(), "realloc must succeed");
        unsafe { alloc.dealloc(grown, Layout::from_size_align(128, 8).unwrap()) };

        let layout = Layout::from_size_align(32, 8).unwrap();
        let zeroed = unsafe { alloc.alloc_zeroed(layout) };
        assert!(!zeroed.is_null(), "alloc_zeroed must succeed");
        unsafe { alloc.dealloc(zeroed, layout) };
    }
}
