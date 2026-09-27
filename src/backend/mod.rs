//! `AgentBackend` abstraction: one CLI agent == one backend.
//!
//! Model strings use `<backend>:<model>` and are delegated to `agent-core`.

pub mod cli;
pub mod mock;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::{Error, Result};

/// Which CLI backs an [`AgentBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    Claude,
    Codex,
    Devin,
    /// OpenCode v2 CLI.
    OpenCode,
    /// In-process mock for tests (`mock:<model>`).
    Mock,
}

impl BackendKind {
    /// Split `"<backend>:<model>"` → `(kind, Some(model))`; bare `"<backend>"`
    /// → `(kind, None)`.
    pub fn parse(model_string: &str) -> Result<(BackendKind, Option<String>)> {
        let s = model_string.trim();
        let (name, model) = match s.split_once(':') {
            Some((b, m)) => (
                b,
                if m.is_empty() {
                    None
                } else {
                    Some(m.to_string())
                },
            ),
            None => (s, None),
        };
        let kind = match name.to_lowercase().as_str() {
            "claude" => BackendKind::Claude,
            "codex" => BackendKind::Codex,
            "devin" => BackendKind::Devin,
            "opencode" => BackendKind::OpenCode,
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
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
            BackendKind::Devin => "devin",
            BackendKind::OpenCode => "opencode",
            BackendKind::Mock => "mock",
        }
    }

    /// Default model strings for (efficient, powerful).
    pub fn default_models(&self) -> (String, String) {
        let (e, p) = match self {
            BackendKind::Claude => ("claude:sonnet@low", "claude:sonnet@high"),
            BackendKind::Codex => ("codex:gpt-5.6-sol@low", "codex:gpt-5.6-sol@high"),
            BackendKind::Devin => ("devin:swe-2-medium", "devin:swe-2-medium"),
            BackendKind::OpenCode => ("opencode", "opencode"),
            BackendKind::Mock => ("mock:test", "mock:test"),
        };
        (e.to_string(), p.to_string())
    }

    /// First installed agent CLI, preferring OpenCode v2.
    pub fn detect() -> Option<BackendKind> {
        [Self::OpenCode, Self::Codex, Self::Claude, Self::Devin]
            .into_iter()
            .find(|kind| crate::sys::find_on_path(kind.as_str()).is_some())
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
    /// JSON Schema for structured agents. Claude and Codex enforce it;
    /// agentwiki also includes it in the prompt and validates the response.
    pub json_schema: Option<serde_json::Value>,
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
        BackendKind::Claude | BackendKind::Codex | BackendKind::Devin | BackendKind::OpenCode => {
            Arc::new(cli::CliBackend { kind })
        }
        BackendKind::Mock => Arc::new(mock::MockBackend::canned(&[])),
    }
}

/// Env vars that could change CLI provider billing. CLIs use saved
/// authentication instead of inheriting these variables from agentwiki.
const BANNED_PREFIXES: &[&str] = &[
    "ANTHROPIC_",
    "OPENAI_",
    "CLAUDE_API",
    "CODEX_API",
    "DEVIN_API",
    "OPENHANDS_",
];

/// Names to remove from the OpenCode child environment.
pub fn billing_env_keys() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .filter_map(|(key, _)| {
            let k = key.to_string_lossy();
            if BANNED_PREFIXES.iter().any(|p| k.starts_with(p)) || k.ends_with("_API_KEY") {
                Some(key)
            } else {
                None
            }
        })
        .collect()
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
        let (k, m) = BackendKind::parse("opencode:openai/gpt-5#high").unwrap();
        assert_eq!(k, BackendKind::OpenCode);
        assert_eq!(m.as_deref(), Some("openai/gpt-5#high"));

        let (k, m) = BackendKind::parse("opencode").unwrap();
        assert_eq!(k, BackendKind::OpenCode);
        assert_eq!(m, None);

        assert_eq!(
            BackendKind::parse("claude:sonnet").unwrap().0,
            BackendKind::Claude
        );
        assert_eq!(
            BackendKind::parse("codex:gpt-5").unwrap().0,
            BackendKind::Codex
        );
        assert_eq!(
            BackendKind::parse("devin:swe-2-medium").unwrap().0,
            BackendKind::Devin
        );
    }
}
