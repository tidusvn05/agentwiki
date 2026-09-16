//! `codex` CLI backend — `codex exec`, prompt via stdin, last message via `-o`.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{AgentBackend, AgentRequest, AgentResult, BackendKind, sanitized_env, tail};
use crate::error::{Error, Result};

/// Runs prompts through `codex exec`.
pub struct CodexBackend;

impl CodexBackend {
    /// The full CLI invocation, kept in one place so flag changes are a
    /// single-point fix. `-` reads the prompt from stdin; `-o` captures the
    /// agent's last message into a file.
    fn build_cmd(out_path: &std::path::Path, model: Option<&str>, cwd: &std::path::Path) -> Command {
        let mut cmd = Command::new("codex");
        cmd.arg("exec")
            .arg("--skip-git-repo-check")
            .arg("-s")
            .arg("read-only")
            .arg("--color")
            .arg("never")
            .arg("-o")
            .arg(out_path);
        if let Some(m) = model {
            cmd.arg("-m").arg(m);
        }
        cmd.arg("-");
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Drop of the Child (cancel, timeout) must kill the CLI — an orphan
        // would keep spending calls with nobody to cache the result.
        cmd.kill_on_drop(true);
        sanitized_env(&mut cmd);
        cmd
    }
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Codex
    }

    fn supports_fs(&self) -> bool {
        true
    }

    async fn run(&self, req: AgentRequest) -> Result<AgentResult> {
        let out_file = tempfile::NamedTempFile::with_prefix("agentwiki-codex-")
            .map_err(|e| Error::io("tempfile", e))?;
        let out_path = out_file.path().to_path_buf();

        let started = Instant::now();
        let mut child = Self::build_cmd(&out_path, req.model.as_deref(), &req.cwd)
            .spawn()
            .map_err(|e| Error::Backend {
                backend: "codex",
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
                backend: "codex",
                message: format!("wait failed: {e}"),
                stderr_tail: String::new(),
            })?;

        let stderr_tail = tail(&String::from_utf8_lossy(&output.stderr), 500);
        if !output.status.success() {
            return Err(Error::Backend {
                backend: "codex",
                message: format!("exit code {:?}", output.status.code()),
                stderr_tail,
            });
        }

        // Prefer the `-o` file (agent's final message); stdout carries the
        // session transcript as fallback.
        let from_file = std::fs::read_to_string(&out_path).unwrap_or_default();
        let text = if from_file.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            from_file.trim().to_string()
        };

        Ok(AgentResult {
            text,
            backend: BackendKind::Codex,
            model: req.model,
            duration: started.elapsed(),
            usage: None,
            stderr_tail,
        })
    }
}
