//! Daily call cap + `calls.jsonl` audit log under `.agentwiki/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::Mutex;

use crate::error::{Error, Result};

#[derive(Debug, Serialize, Deserialize)]
struct DayState {
    date: String,
    count: u32,
}

/// One audit record per real CLI call.
#[derive(Debug, Serialize)]
pub struct CallRecord<'a> {
    /// RFC3339 timestamp.
    pub ts: String,
    /// Agent name (e.g. `dir_summary@src`).
    pub agent: &'a str,
    /// Backend kind.
    pub backend: &'a str,
    /// Model string.
    pub model: &'a str,
    /// Prompt size.
    pub prompt_chars: usize,
    /// Wall seconds.
    pub secs: f64,
    /// ok | error | timeout.
    pub status: &'a str,
}

/// Enforces `daily_cap` and appends to `calls.jsonl`.
pub struct Quota {
    state_path: PathBuf,
    calls_log: PathBuf,
    cap: u32,
    lock: Mutex<()>,
}

impl Quota {
    /// Root the quota files at `<internal>`.
    pub fn new(internal_dir: &Path, cap: u32) -> Self {
        Self {
            state_path: internal_dir.join("state.json"),
            calls_log: internal_dir.join("calls.jsonl"),
            cap,
            lock: Mutex::new(()),
        }
    }

    /// Reserve one call slot; `Err(QuotaExceeded)` when the day is spent.
    /// Check+increment happens under the mutex so parallel agents can't
    /// overrun the cap.
    pub async fn consume(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        let today = today();
        let mut state = self.read_state();
        if state.date != today {
            state = DayState {
                date: today,
                count: 0,
            };
        }
        if state.count >= self.cap {
            return Err(Error::QuotaExceeded { cap: self.cap });
        }
        state.count += 1;
        let body = serde_json::to_string(&state)
            .map_err(|e| Error::Pipeline(format!("quota serialize: {e}")))?;
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&self.state_path, body).map_err(|e| Error::io(&self.state_path, e))
    }

    /// Append one audit line to `calls.jsonl`. Best-effort — a logging
    /// failure must not fail the pipeline.
    pub async fn record(&self, rec: CallRecord<'_>) {
        let _guard = self.lock.lock().await;
        if let Some(parent) = self.calls_log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut line) = serde_json::to_string(&rec) {
            line.push('\n');
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.calls_log)
                .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        }
    }

    /// Current count for today (for the summary report).
    pub async fn today_count(&self) -> u32 {
        let _guard = self.lock.lock().await;
        let s = self.read_state();
        if s.date == today() { s.count } else { 0 }
    }

    fn read_state(&self) -> DayState {
        std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(DayState {
                date: today(),
                count: 0,
            })
    }
}

/// Today's date in UTC, `YYYY-MM-DD`.
pub fn today() -> String {
    OffsetDateTime::now_utc().date().to_string()
}

/// Current UTC time as RFC3339.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
