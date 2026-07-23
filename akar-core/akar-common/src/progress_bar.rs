use indicatif::{ProgressBar as IndiProgressBar, ProgressStyle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A thin wrapper around `indicatif::ProgressBar` for Akar operations.
///
/// Provides factory methods for common progress styles (spinner, count-based,
/// bytes-transferred) used during bulk ingest, export, and long-running queries.
///
/// # Example
///
/// ```no_run
/// use akar_common::progress_bar::AkarProgress;
///
/// let pb = AkarProgress::new("Loading data…", Some(1000));
/// for i in 0..1000 {
///     pb.inc();
///     std::thread::sleep(std::time::Duration::from_millis(1));
/// }
/// pb.finish("Done.");
/// ```
pub struct AkarProgress {
    inner: Option<IndiProgressBar>,
    cancelled: Arc<AtomicBool>,
}

impl AkarProgress {
    /// Create a new progress bar with an optional total count.
    ///
    /// When `total` is `None`, a spinner is used (indeterminate progress).
    /// When `total` is `Some(n)`, a count-based bar is shown.
    pub fn new(msg: &str, total: Option<u64>) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let inner = match total {
            Some(n) => {
                let pb = IndiProgressBar::new(n);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{msg} [{bar:40}] {pos}/{len} ({eta})")
                        .unwrap()
                        .progress_chars("=> "),
                );
                pb.set_message(msg.to_owned());
                Some(pb)
            }
            None => {
                let pb = IndiProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.green} {msg}")
                        .unwrap(),
                );
                pb.set_message(msg.to_owned());
                pb.enable_steady_tick(Duration::from_millis(100));
                Some(pb)
            }
        };
        Self { inner, cancelled }
    }

    /// Advance the progress bar by one step.
    pub fn inc(&self) {
        if let Some(ref pb) = self.inner {
            pb.inc(1);
        }
    }

    /// Advance by `delta` steps.
    pub fn inc_by(&self, delta: u64) {
        if let Some(ref pb) = self.inner {
            pb.inc(delta);
        }
    }

    /// Set the current position.
    pub fn set_pos(&self, pos: u64) {
        if let Some(ref pb) = self.inner {
            pb.set_position(pos);
        }
    }

    /// Update the message displayed alongside the bar.
    pub fn set_message(&self, msg: &str) {
        if let Some(ref pb) = self.inner {
            pb.set_message(msg.to_owned());
        }
    }

    /// Mark as finished with a final message.
    pub fn finish(&self, msg: &str) {
        if let Some(ref pb) = self.inner {
            pb.finish_with_message(msg.to_owned());
        }
    }

    /// Abort and clear the progress display.
    pub fn abort(&self) {
        if let Some(ref pb) = self.inner {
            pb.abandon();
        }
    }

    /// Mark as cancelled. Callers should check [`Self::is_cancelled`] in their
    /// work loop to abort early.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.finish("Cancelled.");
    }

    /// Whether a cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Get a reference to the cancellation flag for sharing across threads.
    pub fn cancelled_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }
}

impl Drop for AkarProgress {
    fn drop(&mut self) {
        // If the progress bar was not explicitly finished, clear it.
        if let Some(ref pb) = self.inner {
            pb.finish_and_clear();
        }
    }
}
