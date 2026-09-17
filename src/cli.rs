//! CLI argument parsing — thin layer over [`crate::config::CliOverrides`].

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{CliOverrides, TargetLanguage};

/// Generate C4-style architecture documentation for a repository using an
/// already-authenticated agent CLI (devin, claude, codex) as the LLM backend.
#[derive(Debug, Parser)]
#[command(name = "agentwiki", version, about)]
pub struct Args {
    /// Named profile from agentwiki.toml or ~/.config/agentwiki/config.toml
    /// (`[profiles.<name>]` section).
    #[arg(value_name = "PROFILE")]
    pub profile: Option<String>,

    /// Subcommands (`doctor`). Any other first token is read as PROFILE.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Repository path to document.
    #[arg(short = 'p', long)]
    pub project_path: Option<PathBuf>,

    /// Documentation output directory.
    #[arg(short = 'o', long)]
    pub output_path: Option<PathBuf>,

    /// Path to agentwiki.toml (defaults to <project>/agentwiki.toml).
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,

    /// Documentation language.
    #[arg(long, value_enum)]
    pub target_language: Option<Lang>,

    /// `"<backend>:<model>"` for routine tasks.
    #[arg(long)]
    pub model_efficient: Option<String>,

    /// `"<backend>:<model>"` for heavy reasoning / retry fallback.
    #[arg(long)]
    pub model_powerful: Option<String>,

    /// Max concurrent CLI calls.
    #[arg(long)]
    pub max_parallels: Option<usize>,

    /// Let the agent read the repo itself instead of embedding code.
    #[arg(long)]
    pub agentic: bool,

    /// Do not read or write the on-disk cache.
    #[arg(long)]
    pub no_cache: bool,

    /// Ignore cached results; recompute and overwrite them.
    #[arg(long)]
    pub force_regenerate: bool,

    /// Skip the research phase; load `.agentwiki/research.json`.
    #[arg(long)]
    pub skip_research: bool,

    /// Stop after the research phase (no documentation written).
    #[arg(long)]
    pub skip_documentation: bool,

    /// Print the resolved config and task DAG, then exit.
    #[arg(long)]
    pub dry_run: bool,

    /// Verbose logging (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

/// Subcommands. `doctor` shadows a profile of the same name.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Health-check the environment: agent CLIs on PATH, running or
    /// orphaned agent processes, `.agentwiki/` state, config sanity.
    Doctor(DoctorArgs),
}

/// `agentwiki doctor` options.
#[derive(Debug, Clone, clap::Args)]
pub struct DoctorArgs {
    /// Repository path to inspect (defaults to top-level `-p` or cwd).
    #[arg(short = 'p', long)]
    pub project_path: Option<PathBuf>,

    /// Path to agentwiki.toml (defaults to top-level `-c`).
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,

    /// Also clean up: remove a stale run.lock and leftover tempfiles.
    #[arg(long)]
    pub fix: bool,
}

/// clap-side language enum (converted to [`TargetLanguage`]).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Lang {
    /// Chinese.
    Zh,
    /// English.
    En,
    /// Japanese.
    Ja,
    /// Korean.
    Ko,
    /// German.
    De,
    /// French.
    Fr,
    /// Russian.
    Ru,
    /// Vietnamese.
    Vi,
}

impl From<Lang> for TargetLanguage {
    fn from(l: Lang) -> Self {
        match l {
            Lang::Zh => TargetLanguage::Zh,
            Lang::En => TargetLanguage::En,
            Lang::Ja => TargetLanguage::Ja,
            Lang::Ko => TargetLanguage::Ko,
            Lang::De => TargetLanguage::De,
            Lang::Fr => TargetLanguage::Fr,
            Lang::Ru => TargetLanguage::Ru,
            Lang::Vi => TargetLanguage::Vi,
        }
    }
}

impl From<&Args> for CliOverrides {
    fn from(a: &Args) -> Self {
        CliOverrides {
            profile: a.profile.clone(),
            project_path: a.project_path.clone(),
            output_path: a.output_path.clone(),
            target_language: a.target_language.map(TargetLanguage::from),
            model_efficient: a.model_efficient.clone(),
            model_powerful: a.model_powerful.clone(),
            max_parallels: a.max_parallels,
            agentic: a.agentic,
            no_cache: a.no_cache,
            force_regenerate: a.force_regenerate,
            skip_research: a.skip_research,
            skip_documentation: a.skip_documentation,
        }
    }
}
