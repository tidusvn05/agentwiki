//! `claude` CLI backend — prompt via stdin, plain-text output.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{AgentBackend, AgentRequest, AgentResult, BackendKind, sanitized_env, tail};
use crate::error::{Error, Result};

/// Runs prompts through `claude -p`.
pub struct ClaudeBackend;

impl ClaudeBackend {
    /// The full CLI invocation, kept in one place so flag changes are a
    /// single-point fix.
    fn build_cmd(model: Option<&str>, cwd: &std::path::Path) -> Command {
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("text")
            .arg("--no-session-persistence")
            .arg("--permission-mode")
            .arg("bypassPermissions")
            // Ignore user/project CLAUDE.md + settings so prompts stay clean.
            .arg("--setting-sources")
            .arg("local");
        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        sanitized_env(&mut cmd);
        cmd
    }
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Claude
    }

    fn supports_fs(&self) -> bool {
        true
    }

    async fn run(&self, req: AgentRequest) -> Result<AgentResult> {
        let started = Instant::now();
        let mut child = Self::build_cmd(req.model.as_deref(), &req.cwd)
            .spawn()
            .map_err(|e| Error::Backend {
                backend: "claude",
                message: format!("spawn failed: {e}"),
                stderr_tail: String::new(),
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(req.prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        let output = tokio::time::timeout(req.timeout, child.wait_with_output())
            .await
            .map_err(|_| Error::Timeout {
                agent: req.agent.clone(),
                secs: req.timeout,
            })?
            .map_err(|e| Error::Backend {
                backend: "claude",
                message: format!("wait failed: {e}"),
                stderr_tail: String::new(),
            })?;

        let stderr_tail = tail(&String::from_utf8_lossy(&output.stderr), 500);
        if !output.status.success() {
            return Err(Error::Backend {
                backend: "claude",
                message: format!("exit code {:?}", output.status.code()),
                stderr_tail,
            });
        }
        Ok(AgentResult {
            text: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            backend: BackendKind::Claude,
            model: req.model,
            duration: started.elapsed(),
            usage: None,
            stderr_tail,
        })
    }
}
