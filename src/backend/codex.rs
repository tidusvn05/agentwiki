//! `codex` CLI backend — `codex exec`, prompt via stdin, last message via `-o`.
//!
//! Model string extension: `codex:<model>@<effort>` sets
//! `model_reasoning_effort` for that call (e.g. `codex:gpt-5.6-sol@high`).
//! Bare `codex:<model>` inherits the user's configured default.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{
    AgentBackend, AgentRequest, AgentResult, BackendKind, TokenUsage, sanitized_env, tail,
};
use crate::error::{Error, Result};

/// Runs prompts through `codex exec`.
pub struct CodexBackend;

/// Reasoning efforts accepted after `model@` in the model string.
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

/// Split `"<model>@<effort>"` → `(model, Some(effort))`; bare `"<model>"` →
/// `(model, None)`. Effort is validated here so a typo fails before spawn
/// rather than inside the CLI.
fn parse_model(model: &str) -> Result<(String, Option<String>)> {
    match model.rsplit_once('@') {
        None => Ok((model.to_string(), None)),
        Some((m, e)) if EFFORTS.contains(&e) => Ok((m.to_string(), Some(e.to_string()))),
        Some((_, e)) => Err(Error::Backend {
            backend: "codex",
            message: format!(
                "unknown effort '{e}' in 'codex:{model}' (valid: {})",
                EFFORTS.join(", ")
            ),
            stderr_tail: String::new(),
        }),
    }
}

/// Parse `codex exec --json` stdout (one JSON event per line): the last
/// `agent_message` item is the response-text fallback, `turn.completed`
/// carries token usage, `turn.failed`/`error` carry failure detail (exec
/// exits 0 on a failed turn — it must be surfaced explicitly).
fn parse_events(stdout: &str) -> (Option<String>, Option<TokenUsage>, Option<String>) {
    let mut text = None;
    let mut usage = None;
    let mut error = None;
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"].as_str() == Some("agent_message") => {
                text = v["item"]["text"].as_str().map(str::to_string);
            }
            Some("turn.completed") => {
                let u = &v["usage"];
                usage = Some(TokenUsage {
                    input: u["input_tokens"].as_u64().unwrap_or(0),
                    output: u["output_tokens"].as_u64().unwrap_or(0)
                        + u["reasoning_output_tokens"].as_u64().unwrap_or(0),
                });
            }
            Some("turn.failed") | Some("error") => {
                error = v["error"]["message"]
                    .as_str()
                    .or_else(|| v["message"].as_str())
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    (text, usage, error)
}

/// Rewrite a schemars-produced schema into the strict subset codex's
/// `--output-schema` accepts: `oneOf` → `enum`/`anyOf`, every object gets
/// `additionalProperties: false` and a `required` listing all properties
/// (originally-optional fields become nullable `anyOf`).
fn normalize_schema(v: &mut serde_json::Value) {
    use serde_json::Value;
    let map = match v {
        Value::Object(m) => m,
        Value::Array(a) => {
            for x in a {
                normalize_schema(x);
            }
            return;
        }
        _ => return,
    };

    map.remove("$schema");

    // `$ref` must stand alone (`allOf` is also banned) — sibling keywords
    // are dropped; descriptions survive in the prompt's schema block.
    if map.contains_key("$ref") && map.len() > 1 {
        let ref_val = map.remove("$ref").unwrap();
        map.clear();
        map.insert("$ref".to_string(), ref_val);
    }

    // `oneOf` is rejected: collapse const-variants into a plain `enum`,
    // otherwise downgrade to `anyOf`.
    if let Some(Value::Array(variants)) = map.remove("oneOf") {
        let consts: Option<Vec<Value>> = variants.iter().map(|v| v.get("const").cloned()).collect();
        match consts {
            Some(vals) => {
                let mut tys: Vec<&str> = variants
                    .iter()
                    .filter_map(|v| v.get("type").and_then(|t| t.as_str()))
                    .collect();
                tys.sort_unstable();
                tys.dedup();
                if let [ty] = tys.as_slice() {
                    map.insert("type".to_string(), Value::String((*ty).to_string()));
                }
                map.insert("enum".to_string(), Value::Array(vals));
            }
            None => {
                map.insert("anyOf".to_string(), Value::Array(variants));
            }
        }
    }

    // Normalize children first (properties/items/anyOf/$defs/…).
    for child in map.values_mut() {
        normalize_schema(child);
    }

    // Objects must be closed, and every property must appear in `required`.
    // Fields schemars left optional stay optional via a nullable `anyOf`.
    if map.get("type").and_then(|t| t.as_str()) == Some("object") || map.contains_key("properties")
    {
        let prev_required: std::collections::HashSet<String> = map
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(Value::Object(props)) = map.get_mut("properties") {
            let all: Vec<Value> = props.keys().cloned().map(Value::String).collect();
            for (name, pspec) in props.iter_mut() {
                if !prev_required.contains(name) && !is_nullable(pspec) {
                    let orig = pspec.take();
                    *pspec = serde_json::json!({"anyOf": [orig, {"type": "null"}]});
                }
            }
            map.insert("required".to_string(), Value::Array(all));
        }
        map.insert("additionalProperties".to_string(), Value::Bool(false));
    }
}

/// True when the schema already permits `null` (`type` union or `anyOf`).
fn is_nullable(v: &serde_json::Value) -> bool {
    let is_null_type =
        |x: &serde_json::Value| x.get("type").and_then(|t| t.as_str()) == Some("null");
    match v.get("type") {
        Some(t) if t.as_str() == Some("null") => return true,
        Some(serde_json::Value::Array(a)) if a.iter().any(|x| x.as_str() == Some("null")) => {
            return true;
        }
        _ => {}
    }
    v.get("anyOf")
        .and_then(|a| a.as_array())
        .is_some_and(|a| a.iter().any(is_null_type))
}

impl CodexBackend {
    /// The full CLI invocation, kept in one place so flag changes are a
    /// single-point fix. `-` reads the prompt from stdin; `-o` captures the
    /// agent's last message into a file.
    fn build_cmd(
        out_path: &std::path::Path,
        model: Option<&str>,
        effort: Option<&str>,
        schema_path: Option<&std::path::Path>,
        cwd: &std::path::Path,
    ) -> Command {
        let mut cmd = Command::new("codex");
        cmd.arg("exec")
            .arg("--skip-git-repo-check")
            .arg("-s")
            .arg("read-only")
            .arg("--color")
            .arg("never")
            // One subprocess per call — leave no session files behind.
            .arg("--ephemeral")
            // User hooks (e.g. SessionStart) would fire once per call.
            .arg("--disable")
            .arg("hooks")
            // Keep project AGENTS.md out of the prompt — materials are
            // agentwiki's job.
            .arg("-c")
            .arg("project_doc_max_bytes=0")
            .arg("--json")
            .arg("-o")
            .arg(out_path);
        if let Some(m) = model {
            cmd.arg("-m").arg(m);
        }
        if let Some(e) = effort {
            cmd.arg("-c").arg(format!("model_reasoning_effort=\"{e}\""));
        }
        if let Some(s) = schema_path {
            cmd.arg("--output-schema").arg(s);
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

        // `--output-schema` reads a file; it must outlive the child.
        let schema_file = match &req.json_schema {
            Some(schema) => {
                let mut schema = schema.clone();
                normalize_schema(&mut schema);
                let f = tempfile::NamedTempFile::with_prefix("agentwiki-codex-schema-")
                    .map_err(|e| Error::io("tempfile", e))?;
                std::fs::write(f.path(), schema.to_string()).map_err(|e| Error::io(f.path(), e))?;
                Some(f)
            }
            None => None,
        };

        let (model, effort) = match req.model.as_deref() {
            Some(m) => {
                let (m, e) = parse_model(m)?;
                (Some(m), e)
            }
            None => (None, None),
        };

        let started = Instant::now();
        let mut child = Self::build_cmd(
            &out_path,
            model.as_deref(),
            effort.as_deref(),
            schema_file.as_ref().map(|f| f.path()),
            &req.cwd,
        )
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

        let (event_text, usage, event_error) =
            parse_events(&String::from_utf8_lossy(&output.stdout));
        if let Some(msg) = event_error {
            return Err(Error::Backend {
                backend: "codex",
                message: tail(&msg, 500),
                stderr_tail,
            });
        }

        // Prefer the `-o` file (agent's final message); the event stream's
        // last agent_message is the fallback.
        let from_file = std::fs::read_to_string(&out_path).unwrap_or_default();
        let text = if from_file.trim().is_empty() {
            event_text.unwrap_or_default().trim().to_string()
        } else {
            from_file.trim().to_string()
        };

        Ok(AgentResult {
            text,
            backend: BackendKind::Codex,
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
        let (m, e) = parse_model("gpt-5.6-sol").unwrap();
        assert_eq!(m, "gpt-5.6-sol");
        assert_eq!(e, None);
    }

    #[test]
    fn parse_model_with_effort() {
        let (m, e) = parse_model("gpt-5.6-sol@high").unwrap();
        assert_eq!(m, "gpt-5.6-sol");
        assert_eq!(e.as_deref(), Some("high"));
    }

    #[test]
    fn parse_model_rejects_bad_effort() {
        assert!(parse_model("gpt-5.6-sol@turbo").is_err());
        assert!(parse_model("gpt-5.6-sol@").is_err());
    }

    #[test]
    fn parse_events_extracts_text_and_usage() {
        let stdout = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"t1\"}\n",
            "{\"type\":\"turn.started\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"i0\",\"type\":\"agent_message\",\"text\":\"OK\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":50,\"output_tokens\":5,\"reasoning_output_tokens\":2}}\n"
        );
        let (text, usage, error) = parse_events(stdout);
        assert_eq!(text.as_deref(), Some("OK"));
        let u = usage.unwrap();
        assert_eq!(u.input, 100);
        assert_eq!(u.output, 7);
        assert_eq!(error, None);
    }

    #[test]
    fn parse_events_surfaces_turn_failure() {
        let stdout =
            "{\"type\":\"turn.failed\",\"error\":{\"message\":\"invalid_json_schema: nope\"}}\n";
        let (_, _, error) = parse_events(stdout);
        assert_eq!(error.as_deref(), Some("invalid_json_schema: nope"));
    }

    #[test]
    fn parse_events_ignores_garbage() {
        let (text, usage, error) = parse_events("not json\n{}\n");
        assert_eq!(text, None);
        assert!(usage.is_none());
        assert_eq!(error, None);
    }

    #[test]
    fn normalize_collapses_oneof_consts_to_enum() {
        let mut v = serde_json::json!({
            "oneOf": [
                {"const": "a", "type": "string"},
                {"const": "b", "type": "string"}
            ]
        });
        normalize_schema(&mut v);
        assert_eq!(v["enum"], serde_json::json!(["a", "b"]));
        assert_eq!(v["type"], "string");
        assert!(v.get("oneOf").is_none());
    }

    #[test]
    fn normalize_closes_objects_and_makes_optional_nullable() {
        let mut v = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "maybe": {"type": "string"}
            },
            "required": ["name"]
        });
        normalize_schema(&mut v);
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(v["required"], serde_json::json!(["maybe", "name"]));
        assert_eq!(v["properties"]["name"]["type"], "string");
        let maybe = &v["properties"]["maybe"]["anyOf"];
        assert_eq!(maybe[1], serde_json::json!({"type": "null"}));
    }

    #[test]
    fn normalize_recurses_into_defs() {
        let mut v = serde_json::json!({
            "$defs": {
                "Inner": {"type": "object", "properties": {"x": {"type": "integer"}}}
            },
            "type": "object",
            "properties": {"inner": {"$ref": "#/$defs/Inner"}}
        });
        normalize_schema(&mut v);
        let inner = &v["$defs"]["Inner"];
        assert_eq!(inner["additionalProperties"], false);
        assert_eq!(inner["required"], serde_json::json!(["x"]));
    }
}
