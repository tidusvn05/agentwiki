//! Bridges agentwiki's backend trait to the four `agent-core` CLI adapters.

use async_trait::async_trait;

use super::{AgentBackend, AgentRequest, AgentResult, BackendKind, TokenUsage};
use crate::error::{Error, Result};

pub struct CliBackend {
    pub kind: BackendKind,
}

#[async_trait]
impl AgentBackend for CliBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn supports_fs(&self) -> bool {
        true
    }

    async fn run(&self, req: AgentRequest) -> Result<AgentResult> {
        let provider = match self.kind {
            BackendKind::Claude => agent_core::Provider::Claude,
            BackendKind::Codex => agent_core::Provider::Codex,
            BackendKind::Devin => agent_core::Provider::Devin,
            BackendKind::OpenCode => agent_core::Provider::OpenCodeV2,
            BackendKind::Mock => unreachable!("mock uses its own backend"),
        };
        let core = agent_core::AgentCli::discover(provider).map_err(|e| map_error(self.kind, e))?;
        let result = core
            .run(agent_core::RunRequest {
                prompt: req.prompt,
                cwd: req.cwd,
                model: req.model.clone(),
                timeout: req.timeout,
                env_remove: super::billing_env_keys(),
                json_schema: req.json_schema,
            })
            .await
            .map_err(|e| match e {
                agent_core::Error::Timeout(secs) => Error::Timeout {
                    agent: req.agent,
                    secs,
                },
                other => map_error(self.kind, other),
            })?;
        Ok(AgentResult {
            text: result.text,
            backend: self.kind,
            model: req.model,
            duration: result.duration,
            usage: result.usage.map(|u| TokenUsage {
                input: u.input + u.cache_read + u.cache_write,
                output: u.output + u.reasoning,
            }),
            stderr_tail: result.stderr_tail,
        })
    }
}

fn map_error(kind: BackendKind, error: agent_core::Error) -> Error {
    Error::Backend {
        backend: kind.as_str(),
        message: error.to_string(),
        stderr_tail: String::new(),
    }
}
