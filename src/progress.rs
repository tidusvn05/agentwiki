//! Terminal progress display — a single spinner bar tracking agent
//! instances, with the in-flight instance keys as its message.
//!
//! Hidden automatically when stderr is not a TTY (piped output, CI,
//! tests), so callers never need to check.

use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Pipeline progress bar. `len` grows as fan-out targets are discovered;
/// `pos` counts finished instances.
pub struct Progress {
    bar: ProgressBar,
    running: Mutex<BTreeSet<String>>,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    /// Visible spinner on a TTY, hidden otherwise.
    pub fn new() -> Self {
        let target = if std::io::stderr().is_terminal() {
            ProgressDrawTarget::stderr()
        } else {
            ProgressDrawTarget::hidden()
        };
        let bar = ProgressBar::with_draw_target(Some(0), target);
        if let Ok(style) = ProgressStyle::with_template(
            "{spinner:.cyan} [{pos:>3}/{len:3}] {elapsed_precise} {msg}",
        ) {
            bar.set_style(style);
        }
        bar.enable_steady_tick(Duration::from_millis(120));
        Self {
            bar,
            running: Mutex::new(BTreeSet::new()),
        }
    }

    /// Grow the total by `n` newly discovered instances.
    pub fn add_total(&self, n: u64) {
        self.bar.inc_length(n);
    }

    /// Mark an instance as in-flight.
    pub fn start(&self, key: &str) {
        if let Ok(mut r) = self.running.lock() {
            r.insert(key.to_string());
        }
        self.sync_message();
    }

    /// Mark an instance finished (success, cache hit, or final failure).
    pub fn finish(&self, key: &str) {
        if let Ok(mut r) = self.running.lock() {
            r.remove(key);
        }
        self.bar.inc(1);
        self.sync_message();
    }

    /// Pipeline finished successfully.
    pub fn done(&self) {
        self.bar.finish_with_message("done");
    }

    /// Pipeline aborted — leave the last state visible with the error.
    pub fn fail(&self, err: &crate::error::Error) {
        self.bar.abandon_with_message(format!("failed: {err}"));
    }

    /// User-requested shutdown.
    pub fn cancel(&self) {
        self.bar
            .abandon_with_message("cancelled — rerun resumes from cache");
    }

    /// Show the in-flight set, truncated to one line.
    fn sync_message(&self) {
        let Ok(running) = self.running.lock() else {
            return;
        };
        let mut msg = String::new();
        for (i, key) in running.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            if msg.len() + sep.len() + key.len() > 72 {
                msg.push_str(&format!("… +{}", running.len() - i));
                break;
            }
            msg.push_str(sep);
            msg.push_str(key);
        }
        drop(running);
        self.bar.set_message(msg);
    }
}
