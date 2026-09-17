//! `claude` CLI backend — `claude -p`, prompt via stdin, JSON envelope output.
//!
//! Model string extension: `claude:<model>@<effort>` sets `--effort` for that
//! call (e.g. `claude:sonnet@high`). Bare `claude:<model>` inherits the CLI's
//! configured default.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{
    AgentBackend, AgentRequest, AgentResult, BackendKind, TokenUsage, sanitized_env, tail,
};
use crate::error::{Error, Result};

/// Runs prompts through `claude -p`.
pub struct ClaudeBackend;

/// Effort levels accepted after `model@` in the model string (`--effort`).
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Split `"<model>@<effort>"` → `(model, Some(effort))`; bare `"<model>"` →
/// `(model, None)`. Effort is validated here so a typo fails before spawn
/// rather than inside the CLI.
fn parse_model(model: &str) -> Result<(String, Option<String>)> {
    match model.rsplit_once('@') {
        None => Ok((model.to_string(), None)),
        Some((m, e)) if EFFORTS.contains(&e) => Ok((m.to_string(), Some(e.to_string()))),
        Some((_, e)) => Err(Error::Backend {
            backend: "claude",
            message: format!(
                "unknown effort '{e}' in 'claude:{model}' (valid: {})",
                EFFORTS.join(", ")
            ),
            stderr_tail: String::new(),
        }),
    }
}

/// Strip `$schema` keys — `claude --json-schema` rejects the draft URI
/// schemars emits at the root (`no schema with key or ref
/// "…/draft/2020-12/schema"`).
fn sanitize_schema(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("$schema");
            for child in m.values_mut() {
                sanitize_schema(child);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a {
                sanitize_schema(x);
            }
        }
        _ => {}
    }
}

/// Parse the `--output-format json` envelope: `structured_output` (present
/// when `--json-schema` was given) or `result` carries the response text,
/// `usage` carries token counts, `is_error` + `result`/`errors` carry failure
/// detail — claude may exit 0 on a failed turn, so `is_error` must be
/// surfaced explicitly.
fn parse_envelope(stdout: &str) -> (Option<String>, Option<TokenUsage>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return (None, None, None);
    };
    let text = v
        .get("structured_output")
        .filter(|s| !s.is_null())
        .map(|s| s.to_string())
        .or_else(|| v["result"].as_str().map(str::to_string));
    let u = &v["usage"];
    let usage = u.is_object().then(|| TokenUsage {
        // claude reports non-cached input separately; include cache tokens so
        // `input` means "total prompt tokens" like it does for codex.
        input: u["input_tokens"].as_u64().unwrap_or(0)
            + u["cache_creation_input_tokens"].as_u64().unwrap_or(0)
            + u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        output: u["output_tokens"].as_u64().unwrap_or(0),
    });
    let error = if v["is_error"].as_bool().unwrap_or(false) {
        let detail = v["errors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|s| !s.is_empty())
            .or_else(|| v["result"].as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown error".to_string());
        Some(match v["subtype"].as_str() {
            Some("") | Some("success") | None => detail,
            Some(subtype) => format!("{subtype}: {detail}"),
        })
    } else {
        None
    };
    (text, usage, error)
}

impl ClaudeBackend {
    /// The full CLI invocation, kept in one place so flag changes are a
    /// single-point fix.
    fn build_cmd(
        model: Option<&str>,
        effort: Option<&str>,
        schema: Option<&str>,
        cwd: &std::path::Path,
    ) -> Command {
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--no-session-persistence")
            .arg("--permission-mode")
            .arg("bypassPermissions")
            // Ignore user/project CLAUDE.md + settings so prompts stay clean.
            .arg("--setting-sources")
            .arg("local");
        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }
        if let Some(e) = effort {
            cmd.arg("--effort").arg(e);
        }
        if let Some(s) = schema {
            // `--json-schema` takes the schema inline (not a file).
            cmd.arg("--json-schema").arg(s);
        }
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
impl AgentBackend for ClaudeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Claude
    }

    fn supports_fs(&self) -> bool {
        true
    }

    async fn run(&self, req: AgentRequest) -> Result<AgentResult> {
        let (model, effort) = match req.model.as_deref() {
            Some(m) => {
                let (m, e) = parse_model(m)?;
                (Some(m), e)
            }
            None => (None, None),
        };
        let schema = req.json_schema.as_ref().map(|s| {
            let mut s = s.clone();
            sanitize_schema(&mut s);
            serde_json::to_string(&s).unwrap_or_default()
        });

        let started = Instant::now();
        let mut child = Self::build_cmd(
            model.as_deref(),
            effort.as_deref(),
            schema.as_deref(),
            &req.cwd,
        )
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        let (text, usage, error) = parse_envelope(&stdout);

        if !output.status.success() {
            return Err(Error::Backend {
                backend: "claude",
                // The envelope's error text beats a bare exit code.
                message: error.unwrap_or_else(|| format!("exit code {:?}", output.status.code())),
                stderr_tail,
            });
        }
        if let Some(msg) = error {
            return Err(Error::Backend {
                backend: "claude",
                message: tail(&msg, 500),
                stderr_tail,
            });
        }
        // Envelope unparseable → fall back to raw stdout (plain-text output
        // stays usable for downstream extraction).
        let text = text.unwrap_or_else(|| stdout.trim().to_string());
        Ok(AgentResult {
            text: text.trim().to_string(),
            backend: BackendKind::Claude,
            model: req.model,
            duration: started.elapsed(),
            usage,
            stderr_tail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_model_plain() {
        let (m, e) = parse_model("sonnet").unwrap();
        assert_eq!(m, "sonnet");
        assert_eq!(e, None);
    }

    #[test]
    fn parse_model_with_effort() {
        let (m, e) = parse_model("sonnet@high").unwrap();
        assert_eq!(m, "sonnet");
        assert_eq!(e.as_deref(), Some("high"));
    }

    #[test]
    fn parse_model_rejects_bad_effort() {
        assert!(parse_model("sonnet@ultra").is_err());
        assert!(parse_model("sonnet@").is_err());
    }

    #[test]
    fn parse_envelope_extracts_result_and_usage() {
        let stdout = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": "the answer",
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": 100,
                "cache_read_input_tokens": 50,
                "output_tokens": 7
            }
        })
        .to_string();
        let (text, usage, error) = parse_envelope(&stdout);
        assert_eq!(text.as_deref(), Some("the answer"));
        let u = usage.unwrap();
        assert_eq!(u.input, 160);
        assert_eq!(u.output, 7);
        assert_eq!(error, None);
    }

    #[test]
    fn parse_envelope_prefers_structured_output() {
        let stdout = serde_json::json!({
            "is_error": false,
            "result": "{\"answer\": 4}",
            "structured_output": {"answer": 4}
        })
        .to_string();
        let (text, _, _) = parse_envelope(&stdout);
        assert_eq!(text.as_deref(), Some("{\"answer\":4}"));
    }

    #[test]
    fn parse_envelope_surfaces_is_error() {
        let stdout = serde_json::json!({
            "subtype": "success",
            "is_error": true,
            "result": "There's an issue with the selected model (nope)."
        })
        .to_string();
        let (_, _, error) = parse_envelope(&stdout);
        assert_eq!(
            error.as_deref(),
            Some("There's an issue with the selected model (nope).")
        );
    }

    #[test]
    fn parse_envelope_prepends_non_success_subtype() {
        let stdout = serde_json::json!({
            "subtype": "error_max_turns",
            "is_error": true,
            "result": "hit the turn limit"
        })
        .to_string();
        let (_, _, error) = parse_envelope(&stdout);
        assert_eq!(
            error.as_deref(),
            Some("error_max_turns: hit the turn limit")
        );
    }

    #[test]
    fn sanitize_strips_draft_key_recursively() {
        let mut v = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "x": {"$schema": "nested", "type": "string"},
                "list": {"type": "array", "items": [{"$schema": "in-array"}]}
            },
            "$defs": {"D": {"type": "integer"}}
        });
        sanitize_schema(&mut v);
        assert!(v.get("$schema").is_none());
        assert!(v["properties"]["x"].get("$schema").is_none());
        assert!(v["properties"]["list"]["items"][0].get("$schema").is_none());
        assert_eq!(v["type"], "object");
        assert_eq!(v["$defs"]["D"]["type"], "integer");
    }

    #[test]
    fn parse_envelope_ignores_garbage() {
        let (text, usage, error) = parse_envelope("not json\n");
        assert_eq!(text, None);
        assert!(usage.is_none());
        assert_eq!(error, None);
    }
}
