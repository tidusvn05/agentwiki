//! `AgentBackend` abstraction: one CLI agent == one backend.
//!
//! Adding a backend = one file + one arm in [`for_kind`]. Model strings follow
//! `"<backend>:<model>"`; the part after `:` is passed through to the CLI.

pub mod claude;
pub mod codex;
pub mod devin;
pub mod mock;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::error::{Error, Result};

/// Which CLI backs an [`AgentBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// `devin` CLI.
    Devin,
    /// `claude` CLI.
    Claude,
    /// `codex` CLI.
    Codex,
    /// In-process mock for tests (`mock:<model>`).
    Mock,
}

impl BackendKind {
    /// Split `"<backend>:<model>"` → `(kind, Some(model))`; bare `"<backend>"`
    /// → `(kind, None)`.
    pub fn parse(model_string: &str) -> Result<(BackendKind, Option<String>)> {
        let s = model_string.trim();
        let (name, model) = match s.split_once(':') {
            Some((b, m)) => (b, if m.is_empty() { None } else { Some(m.to_string()) }),
            None => (s, None),
        };
        let kind = match name.to_lowercase().as_str() {
            "devin" => BackendKind::Devin,
            "claude" => BackendKind::Claude,
            "codex" => BackendKind::Codex,
            "mock" | "test" => BackendKind::Mock,
            other => {
                return Err(Error::BackendNotAvailable(format!(
                    "unknown backend '{other}' in model string '{model_string}'"
                )));
            }
        };
        Ok((kind, model))
    }

    /// Stable lowercase id used in logs, cache keys and audit lines.
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Devin => "devin",
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
        BackendKind::Mock => "mock",
        }
    }
}

/// Token accounting — `None` when the CLI does not expose usage.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
}

/// One backend invocation.
pub struct AgentRequest {
    /// Fully-rendered prompt (system + user merged).
    pub prompt: String,
    /// Model id passed through to the CLI, if any.
    pub model: Option<String>,
    /// Working directory: empty-cwd (embedded mode) or project root (agentic).
    pub cwd: PathBuf,
    /// Per-call timeout.
    pub timeout: Duration,
    /// Agent name, for span/log correlation.
    pub agent: String,
}

/// What the CLI returned.
#[derive(Debug)]
pub struct AgentResult {
    /// Final message / stdout text.
    pub text: String,
    /// Backend that produced it.
    pub backend: BackendKind,
    /// Model actually used, if known.
    pub model: Option<String>,
    /// Wall-clock duration of the call.
    pub duration: Duration,
    /// Token usage if the CLI reports it.
    pub usage: Option<TokenUsage>,
    /// Last bytes of stderr (kept for diagnostics even on success).
    pub stderr_tail: String,
}

/// An authenticated CLI agent usable as an LLM.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Which CLI this is.
    fn kind(&self) -> BackendKind;
    /// Whether the agent can read files inside `cwd` (agentic mode).
    fn supports_fs(&self) -> bool;
    /// Run one prompt, return the final text.
    async fn run(&self, req: AgentRequest) -> Result<AgentResult>;
}

/// Construct a backend by kind — the only place `match` on kinds lives.
pub fn for_kind(kind: BackendKind) -> Arc<dyn AgentBackend> {
    match kind {
        BackendKind::Devin => Arc::new(devin::DevinBackend),
        BackendKind::Claude => Arc::new(claude::ClaudeBackend),
        BackendKind::Codex => Arc::new(codex::CodexBackend),
        BackendKind::Mock => Arc::new(mock::MockBackend::canned(&[])),
    }
}

/// Env vars that would flip a CLI onto metered API billing or confuse session
/// state — never inherited by spawned agents. Defined once, used by every
/// backend.
const BANNED_PREFIXES: &[&str] = &[
    "ANTHROPIC_",
    "OPENAI_",
    "CLAUDE_API",
    "CODEX_API",
    "DEVIN_API",
    "OPENHANDS_",
];

/// Strip billing-related env vars from a child command.
pub fn sanitized_env(cmd: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if BANNED_PREFIXES.iter().any(|p| k.starts_with(p)) {
            cmd.env_remove(&key);
        }
    }
}

/// Last `n` chars of a string for error messages.
pub fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars.iter().skip(chars.len().saturating_sub(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_model_strings() {
        let (k, m) = BackendKind::parse("devin:swe-2-medium").unwrap();
        assert_eq!(k, BackendKind::Devin);
        assert_eq!(m.as_deref(), Some("swe-2-medium"));

        let (k, m) = BackendKind::parse("claude").unwrap();
        assert_eq!(k, BackendKind::Claude);
        assert_eq!(m, None);

        assert!(BackendKind::parse("openai:gpt-5").is_err());
    }
}
