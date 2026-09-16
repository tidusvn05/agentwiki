//! `AgentSpec` — one node in the task DAG.

use serde::Serialize;
use serde::de::DeserializeOwned;
use schemars::JsonSchema;

use crate::config::ModelTier;
use crate::error::{Error, Result};

/// JSON-schema hook for structured agents: one function to inject into the
/// prompt, one to validate/normalize the model's JSON.
#[derive(Clone, Copy)]
pub struct SchemaSpec {
    /// `schema_for!(T)` as a JSON value, for prompt injection.
    pub json_schema: fn() -> serde_json::Value,
    /// `from_value::<T>` then re-serialize — canonical form stored in ctx.
    pub validate: fn(serde_json::Value, &str) -> Result<serde_json::Value>,
}

/// Build a [`SchemaSpec`] for report type `T`.
pub fn schema_spec<T>() -> SchemaSpec
where
    T: JsonSchema + DeserializeOwned + Serialize,
{
    SchemaSpec {
        json_schema: || serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default(),
        validate: |v, agent| {
            // Models sometimes wrap a single object in an array — try the
            // sole element as a fallback before failing.
            let cands: Vec<&serde_json::Value> = match &v {
                serde_json::Value::Array(a) if a.len() == 1 => vec![&v, &a[0]],
                _ => vec![&v],
            };
            let mut last = String::new();
            for cand in cands {
                match serde_json::from_value::<T>(cand.clone()) {
                    Ok(typed) => {
                        return serde_json::to_value(&typed).map_err(|e| {
                            Error::Validation {
                                agent: agent.to_string(),
                                message: e.to_string(),
                            }
                        });
                    }
                    Err(e) => last = e.to_string(),
                }
            }
            Err(Error::Validation {
                agent: agent.to_string(),
                message: format!("{last}; raw: {}", crate::backend::tail(&v.to_string(), 300)),
            })
        },
    }
}

/// Fan-out axis: one instance per X.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanOut {
    /// One instance per scanned directory (`dir_summary`).
    PerDir,
    /// One instance per domain from `domain_modules` (`key_module`, `deep_dive`).
    PerDomain,
}

/// Which pipeline phase a spec belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Phase 1 — produces structured/markdown research.
    Research,
    /// Phase 2 — produces final markdown documents.
    Compose,
}

/// Scan- or context-derived material injected into `{{materials}}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    /// File/directory tree.
    ProjectStructure,
    /// Top-N file insights across directory dossiers.
    CodeInsights,
    /// `relationships` agent output, formatted as edges.
    Relationships,
    /// README content (truncated).
    Readme,
    /// Per-spec custom block (dir file list, domain detail, filtered
    /// insights) built by the runner.
    Custom,
}

/// How an agent produces its result.
#[derive(Clone, Copy)]
pub enum ExecKind {
    /// Render prompt → backend → parse.
    Llm,
    /// Pure function over the context — no CLI call (boundary_doc,
    /// database_doc are deterministic renderers, like deepwiki-rs).
    Deterministic(DetFn),
}

/// Deterministic renderer signature: `(scan, config, dep_result)` → markdown.
pub type DetFn = fn(&crate::scanner::ScanData, &crate::config::Config, &serde_json::Value)
    -> Result<String>;

/// A node in the task DAG.
#[derive(Clone)]
pub struct AgentSpec {
    /// Unique node name (`dir_summary`, `system_context`, …).
    pub name: &'static str,
    /// Template file under `prompts/`.
    pub prompt_tmpl: &'static str,
    /// Structured output contract; `None` → raw markdown/text result.
    pub schema: Option<SchemaSpec>,
    /// Model tier.
    pub tier: ModelTier,
    /// Nodes that must complete first; their results are injected as
    /// `#### <dep>` material blocks automatically.
    pub deps: &'static [&'static str],
    /// Fan-out axis, if any.
    pub fan_out: Option<FanOut>,
    /// Extra scan/context materials for `{{materials}}`.
    pub materials: &'static [Material],
    /// Pipeline phase.
    pub phase: Phase,
    /// LLM call or deterministic render.
    pub exec: ExecKind,
}

impl AgentSpec {
    /// Instance key for a fan-out target (`name@target`) or plain name.
    pub fn instance_key(&self, target: Option<&str>) -> String {
        match target {
            Some(t) => format!("{}@{}", self.name, t),
            None => self.name.to_string(),
        }
    }
}

/// One fan-out target: a directory or a domain module.
#[derive(Debug, Clone)]
pub struct FanTarget {
    /// Cache/instance key (`src`, `Auth`, …).
    pub key: String,
    /// Set for `FanOut::PerDir`.
    pub dir: Option<crate::scanner::DirectoryInfo>,
    /// Set for `FanOut::PerDomain`.
    pub domain: Option<crate::agent::reports::DomainModule>,
}

/// Expand a fan-out axis into concrete targets.
pub async fn expand(
    fan: FanOut,
    _spec: &AgentSpec,
    pctx: &crate::pipeline::PipelineCtx,
) -> Vec<FanTarget> {
    match fan {
        FanOut::PerDir => pctx
            .scan
            .directories
            .iter()
            .cloned()
            .map(|d| {
                let rel = d.rel_path.to_string_lossy().to_string();
                FanTarget {
                    key: if rel.is_empty() { ".".to_string() } else { rel },
                    dir: Some(d),
                    domain: None,
                }
            })
            .collect(),
        FanOut::PerDomain => pctx
            .ctx
            .get_typed::<crate::agent::reports::DomainModulesReport>("domain_modules")
            .await
            .map(|r| {
                r.domain_modules
                    .into_iter()
                    .map(|d| FanTarget {
                        key: d.name.clone(),
                        dir: None,
                        domain: Some(d),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}
