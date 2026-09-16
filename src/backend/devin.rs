//! `devin` CLI backend — the v1 reference implementation.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::process::Command;

use super::{AgentBackend, AgentRequest, AgentResult, BackendKind, sanitized_env, tail};
use crate::error::{Error, Result};

/// Runs prompts through `devin -p`.
pub struct DevinBackend;

impl DevinBackend {
    /// The full CLI invocation, kept in one place so flag changes are a
    /// single-point fix. The prompt travels via a temp file to stay well
    /// under argv limits.
    fn build_cmd(prompt_path: &std::path::Path, model: Option<&str>, cwd: &std::path::Path) -> Command {
        let mut cmd = Command::new("devin");
        cmd.arg("-p")
            .arg("--prompt-file")
            .arg(prompt_path)
            .arg("--respect-workspace-trust")
            .arg("false")
            // Read-only tools auto-approve; edits would still prompt — and a
            // print-mode prompt failure surfaces as exit!=0, which is what we
            // want rather than a silent hang.
            .arg("--permission-mode")
            .arg("auto");
        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }
        cmd.current_dir(cwd)
            .stdin(Stdio::null())
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
impl AgentBackend for DevinBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Devin
    }

    fn supports_fs(&self) -> bool {
        true
    }

    async fn run(&self, req: AgentRequest) -> Result<AgentResult> {
        let prompt_file = tempfile::NamedTempFile::with_prefix("agentwiki-devin-")
            .map_err(|e| Error::io("tempfile", e))?;
        std::fs::write(prompt_file.path(), &req.prompt)
            .map_err(|e| Error::io(prompt_file.path(), e))?;

        let started = Instant::now();
        let mut cmd = Self::build_cmd(prompt_file.path(), req.model.as_deref(), &req.cwd);
        let output = tokio::time::timeout(req.timeout, cmd.output())
            .await
            .map_err(|_| Error::Timeout {
                agent: req.agent.clone(),
                secs: req.timeout,
            })?
            .map_err(|e| Error::Backend {
                backend: "devin",
                message: format!("spawn failed: {e}"),
                stderr_tail: String::new(),
            })?;

        let stderr_tail = tail(&String::from_utf8_lossy(&output.stderr), 500);
        if !output.status.success() {
            return Err(Error::Backend {
                backend: "devin",
                message: format!("exit code {:?}", output.status.code()),
                stderr_tail,
            });
        }
        Ok(AgentResult {
            text: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            backend: BackendKind::Devin,
            model: req.model,
            duration: started.elapsed(),
            usage: None,
            stderr_tail,
        })
    }
}
