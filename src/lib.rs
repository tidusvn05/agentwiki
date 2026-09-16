//! # AgentWiki
//!
//! CLI-agent-native rewrite of deepwiki-rs/Litho: generates C4-style
//! architecture documentation for an arbitrary repository, using an
//! already-authenticated agent CLI (`devin`, `claude`, `codex`) as the LLM
//! instead of a paid HTTP API.
//!
//! ## Pipeline
//!
//! `Preprocess → Research → Compose → Verify`
//!
//! - **Preprocess** ([`scanner`]): deterministic walk, importance scoring,
//!   per-file static extraction.
//! - **Research** ([`agent::registry`], [`agent::runner`]): fan-out
//!   `dir_summary` per directory, then typed research agents (system
//!   context, domain modules, architecture, workflow, key-module,
//!   boundary, database) with lenient-JSON validation.
//! - **Compose** ([`output`]): LLM editors + deterministic renderers →
//!   `1.Overview.md` … `6.Database-Overview.md`.
//! - **Verify** ([`output::verify`]): file integrity + mermaid checks +
//!   summary report.
//!
//! Quota (`calls.jsonl` + daily cap), content-hash caching, and env
//! sanitization live in [`quota`], [`cache`], and [`backend`].

pub mod agent;
pub mod backend;
pub mod cache;
pub mod cli;
pub mod config;
pub mod error;
pub mod output;
pub mod pipeline;
pub mod prompt;
pub mod quota;
pub mod scanner;

pub use config::{CliOverrides, Config, Mode, ModelTier, TargetLanguage};
pub use error::{Error, Result};
pub use pipeline::{PipelineCtx, RunStats, dry_run_report, run};
