//! Error types for the agentwiki library.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// Library-wide error type. Binary entry points wrap this in `anyhow`.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration file could not be read or parsed.
    #[error("config error: {0}")]
    Config(String),

    /// Filesystem operation failed.
    #[error("io error on {path}: {source}")]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying IO error.
        source: std::io::Error,
    },

    /// An agent CLI exited non-zero or could not be spawned.
    #[error("backend {backend} failed: {message}\nstderr tail: {stderr_tail}")]
    Backend {
        /// Backend kind (devin, claude, codex).
        backend: &'static str,
        /// What went wrong.
        message: String,
        /// Last bytes of stderr for debugging.
        stderr_tail: String,
    },

    /// A backend is known but not usable in this environment.
    #[error("backend not available: {0}")]
    BackendNotAvailable(String),

    /// Agent output could not be parsed into the expected shape.
    #[error("agent {agent} output parse failed: {message}")]
    Parse {
        /// Agent that produced the output.
        agent: String,
        /// Parser diagnostic.
        message: String,
    },

    /// Agent output parsed but failed schema/semantic validation.
    #[error("agent {agent} output validation failed: {message}")]
    Validation {
        /// Agent that produced the output.
        agent: String,
        /// Validation diagnostic.
        message: String,
    },

    /// Daily CLI-call cap reached; refuse to spend more.
    #[error("daily quota exceeded ({cap} calls/day)")]
    QuotaExceeded {
        /// Configured cap.
        cap: u32,
    },

    /// A backend call exceeded its timeout.
    #[error("agent {agent} timed out after {secs:?}")]
    Timeout {
        /// Agent being run.
        agent: String,
        /// Timeout that elapsed.
        secs: Duration,
    },

    /// Prompt template could not be loaded or rendered.
    #[error("prompt template {name}: {message}")]
    Prompt {
        /// Template file name.
        name: String,
        /// What went wrong.
        message: String,
    },

    /// An agent in the DAG depends on a node that failed.
    #[error("agent {agent} skipped: dependency {dep} failed")]
    DepFailed {
        /// Dependent agent.
        agent: String,
        /// Failed dependency.
        dep: String,
    },

    /// Catch-all for pipeline-level failures.
    #[error("{0}")]
    Pipeline(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Wrap an IO error with the path it occurred on.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
