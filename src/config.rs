//! `agentwiki.toml` loading and CLI/TOML/default merge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::BackendKind;
use crate::error::{Error, Result};

/// Context mode: embed code in prompts vs. let the agent read files itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Prompts carry code/materials; agent runs in an empty cwd (default).
    #[default]
    Embedded,
    /// Agent runs at the project root and reads files itself.
    Agentic,
}

/// Documentation prose language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TargetLanguage {
    /// Chinese.
    Zh,
    /// English (default).
    #[default]
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

impl TargetLanguage {
    /// Instruction appended to every prompt.
    pub fn instruction(&self) -> &'static str {
        match self {
            Self::Zh => "Write all prose in Simplified Chinese. JSON keys stay in English.",
            Self::En => "Write all prose in English. JSON keys stay in English.",
            Self::Ja => "Write all prose in Japanese. JSON keys stay in English.",
            Self::Ko => "Write all prose in Korean. JSON keys stay in English.",
            Self::De => "Write all prose in German. JSON keys stay in English.",
            Self::Fr => "Write all prose in French. JSON keys stay in English.",
            Self::Ru => "Write all prose in Russian. JSON keys stay in English.",
            Self::Vi => "Write all prose in Vietnamese. JSON keys stay in English.",
        }
    }
}

/// Model tier for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    /// Cheap/fast model for routine tasks.
    Efficient,
    /// Stronger model for complex schemas and as retry fallback.
    Powerful,
}

/// `[models]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    /// `"<backend>:<model>"` for routine tasks.
    pub efficient: String,
    /// `"<backend>:<model>"` for heavy reasoning + fallback.
    pub powerful: String,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        let (efficient, powerful) = BackendKind::Devin.default_models();
        Self {
            efficient,
            powerful,
        }
    }
}

/// `[limits]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    /// Max CLI calls per calendar day across runs.
    pub daily_cap: u32,
    /// Per-call timeout, seconds.
    pub call_timeout_s: u64,
    /// Parse/validate retries per agent call.
    pub retry_attempts: u32,
    /// Max chars of scan materials injected into one prompt.
    pub materials_char_cap: usize,
    /// Max file insights listed per prompt.
    pub code_insights_limit: usize,
    /// Per-file source chars embedded in dir_summary prompts.
    pub file_source_chars: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            daily_cap: 300,
            call_timeout_s: 600,
            retry_attempts: 3,
            materials_char_cap: 192_000,
            code_insights_limit: 25,
            file_source_chars: 500,
        }
    }
}

/// `[scan]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Directory recursion limit.
    pub max_depth: usize,
    /// Skip files larger than this (bytes).
    pub max_file_size: u64,
    /// Only consider `git ls-files` output when in a repo.
    pub git_tracked_only: bool,
    /// Include dotfiles/dot-directories.
    pub include_hidden: bool,
    /// Include tests.
    pub include_tests: bool,
    /// Directory names to skip.
    pub excluded_dirs: Vec<String>,
    /// File glob patterns to skip (matched on file name, lowercase).
    pub excluded_files: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            max_file_size: 512 * 1024,
            git_tracked_only: true,
            include_hidden: false,
            include_tests: false,
            excluded_dirs: vec![
                "target".to_string(),
                "node_modules".to_string(),
                "build".to_string(),
                "dist".to_string(),
                "venv".to_string(),
                ".venv".to_string(),
                "__pycache__".to_string(),
                "__tests__".to_string(),
                "__mocks__".to_string(),
                "out".to_string(),
                "coverage".to_string(),
                // Vendored deps + language-specific build trees — only
                // reachable when git_tracked_only can't help (non-git dirs).
                "vendor".to_string(),
                "deps".to_string(),
                "_build".to_string(),
                "bower_components".to_string(),
                "Pods".to_string(),
                "DerivedData".to_string(),
                ".gradle".to_string(),
                ".svelte-kit".to_string(),
                ".next".to_string(),
                ".nuxt".to_string(),
                ".nx".to_string(),
                ".turbo".to_string(),
                ".output".to_string(),
            ],
            excluded_files: vec![
                "*.lock".to_string(),
                "*.log".to_string(),
                "*.tmp".to_string(),
                "*.cache".to_string(),
                "package-lock.json".to_string(),
                "yarn.lock".to_string(),
                "pnpm-lock.yaml".to_string(),
                "Cargo.lock".to_string(),
                ".env".to_string(),
                "agentwiki.toml".to_string(),
            ],
        }
    }
}

/// `[verify]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyConfig {
    /// Run `mermaid-fixer --dry-run` on the output tree when available.
    pub mermaid_fixer: bool,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            mermaid_fixer: true,
        }
    }
}

/// Fully-resolved runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Repo to document.
    pub project_path: PathBuf,
    /// Where the doc tree is written.
    pub output_path: PathBuf,
    /// Internal state dir (cache, quota, research context).
    pub internal_path: PathBuf,
    /// Prompt-override directory (falls back to embedded templates).
    pub prompts_dir: Option<PathBuf>,
    /// Prose language.
    pub target_language: TargetLanguage,
    /// Parallel CLI calls.
    pub max_parallels: usize,
    /// Embedded | agentic.
    pub mode: Mode,
    /// Skip LLM research; load `.agentwiki/research.json`.
    pub skip_research: bool,
    /// Stop after research (no compose/write).
    pub skip_documentation: bool,
    /// Bypass cache reads and writes.
    pub no_cache: bool,
    /// Ignore cached values but record fresh results.
    pub force_regenerate: bool,
    /// Models.
    pub models: ModelsConfig,
    /// Limits.
    pub limits: LimitsConfig,
    /// Scan options.
    pub scan: ScanConfig,
    /// Verify options.
    pub verify: VerifyConfig,
    /// Name of the applied profile (`agentwiki <profile>`), if any.
    pub profile: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            project_path: PathBuf::from("."),
            output_path: PathBuf::from("./agentwiki.docs"),
            internal_path: PathBuf::from(".agentwiki"),
            prompts_dir: None,
            target_language: TargetLanguage::En,
            max_parallels: 2,
            mode: Mode::Embedded,
            skip_research: false,
            skip_documentation: false,
            no_cache: false,
            force_regenerate: false,
            models: ModelsConfig::default(),
            limits: LimitsConfig::default(),
            scan: ScanConfig::default(),
            verify: VerifyConfig::default(),
            profile: None,
        }
    }
}

/// Optional TOML file shape — every field optional, merged over defaults.
/// Also used for `[profiles.<name>]` entries (their `profiles` key is ignored).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TomlConfig {
    project_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    internal_path: Option<PathBuf>,
    prompts_dir: Option<PathBuf>,
    target_language: Option<TargetLanguage>,
    max_parallels: Option<usize>,
    mode: Option<Mode>,
    skip_research: Option<bool>,
    skip_documentation: Option<bool>,
    no_cache: Option<bool>,
    force_regenerate: Option<bool>,
    models: Option<ModelsPartial>,
    limits: Option<LimitsPartial>,
    scan: Option<ScanPartial>,
    verify: Option<VerifyPartial>,
    /// Named profiles selectable via `agentwiki <profile>`.
    profiles: Option<HashMap<String, TomlConfig>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ModelsPartial {
    efficient: Option<String>,
    powerful: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LimitsPartial {
    daily_cap: Option<u32>,
    call_timeout_s: Option<u64>,
    retry_attempts: Option<u32>,
    materials_char_cap: Option<usize>,
    code_insights_limit: Option<usize>,
    file_source_chars: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ScanPartial {
    max_depth: Option<usize>,
    max_file_size: Option<u64>,
    git_tracked_only: Option<bool>,
    include_hidden: Option<bool>,
    include_tests: Option<bool>,
    excluded_dirs: Option<Vec<String>>,
    excluded_files: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct VerifyPartial {
    mermaid_fixer: Option<bool>,
}

/// CLI-supplied overrides (already flattened from clap args).
#[derive(Debug, Default)]
pub struct CliOverrides {
    /// Positional `agentwiki <profile>`.
    pub profile: Option<String>,
    /// `-p`
    pub project_path: Option<PathBuf>,
    /// `-o`
    pub output_path: Option<PathBuf>,
    /// `--target-language`
    pub target_language: Option<TargetLanguage>,
    /// `--model-efficient`
    pub model_efficient: Option<String>,
    /// `--model-powerful`
    pub model_powerful: Option<String>,
    /// `--max-parallels`
    pub max_parallels: Option<usize>,
    /// `--agentic`
    pub agentic: bool,
    /// `--no-cache`
    pub no_cache: bool,
    /// `--force-regenerate`
    pub force_regenerate: bool,
    /// `--skip-research`
    pub skip_research: bool,
    /// `--skip-documentation`
    pub skip_documentation: bool,
}

impl Config {
    /// Merge order: defaults → global `~/.config/agentwiki/config.toml` →
    /// project `agentwiki.toml` → selected profile → CLI overrides →
    /// PATH auto-detection for model tiers nobody set.
    pub fn load(cli: &CliOverrides, config_path: Option<&Path>) -> Result<Config> {
        let mut cfg = Config::default();
        // [efficient, powerful] — tiers explicitly configured somewhere.
        let mut models_set = [false; 2];

        // Global user config (base settings + shared profiles).
        let global_toml = global_config_path().map(|p| load_toml(&p)).transpose()?;
        if let Some(t) = &global_toml {
            track_models(t, &mut models_set);
            cfg.apply_toml(t);
        }

        // TOML layer — look next to the project first, then the cwd.
        let toml_path = config_path.map(|p| p.to_path_buf()).or_else(|| {
            let project = cli
                .project_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("."));
            let candidate = project.join("agentwiki.toml");
            if candidate.is_file() {
                Some(candidate)
            } else {
                PathBuf::from("agentwiki.toml")
                    .is_file()
                    .then_some(PathBuf::from("agentwiki.toml"))
            }
        });
        let project_toml = toml_path.map(|p| load_toml(&p)).transpose()?;
        if let Some(t) = &project_toml {
            track_models(t, &mut models_set);
            cfg.apply_toml(t);
        }

        // Profile layer — no positional arg means `default`. Project
        // profiles shadow global ones, and both shadow the built-ins:
        // `default` (a no-op) and bare backend names (`agentwiki claude`
        // selects that CLI's default model pair).
        let name = cli.profile.as_deref().unwrap_or("default");
        if let Some(profile) = find_profile(name, project_toml.as_ref(), global_toml.as_ref()) {
            track_models(profile, &mut models_set);
            cfg.apply_toml(profile);
        } else if let Some(builtin) = builtin_profile(name) {
            track_models(&builtin, &mut models_set);
            cfg.apply_toml(&builtin);
        } else {
            return Err(unknown_profile(
                name,
                project_toml.as_ref(),
                global_toml.as_ref(),
            ));
        }
        cfg.profile = Some(name.to_string());

        // CLI layer.
        if let Some(p) = &cli.project_path {
            cfg.project_path = p.clone();
        }
        if let Some(p) = &cli.output_path {
            cfg.output_path = p.clone();
        }
        if let Some(l) = cli.target_language {
            cfg.target_language = l;
        }
        if let Some(m) = &cli.model_efficient {
            cfg.models.efficient = m.clone();
            models_set[0] = true;
        }
        if let Some(m) = &cli.model_powerful {
            cfg.models.powerful = m.clone();
            models_set[1] = true;
        }
        if let Some(n) = cli.max_parallels {
            cfg.max_parallels = n.max(1);
        }
        if cli.agentic {
            cfg.mode = Mode::Agentic;
        }
        cfg.no_cache |= cli.no_cache;
        cfg.force_regenerate |= cli.force_regenerate;
        cfg.skip_research |= cli.skip_research;
        cfg.skip_documentation |= cli.skip_documentation;

        // Auto-detect: model tiers nobody configured fall back to the first
        // agent CLI on PATH (devin → codex → claude). Nothing found keeps
        // the built-in default — the spawn error at run time is clear enough.
        if !(models_set[0] && models_set[1])
            && let Some(kind) = BackendKind::detect()
        {
            let (e, p) = kind.default_models();
            if !models_set[0] {
                cfg.models.efficient = e;
            }
            if !models_set[1] {
                cfg.models.powerful = p;
            }
        }

        // internal_path is relative to the project (per-repo cache/state).
        if cfg.internal_path.is_relative() {
            cfg.internal_path = cfg.project_path.join(&cfg.internal_path);
        }
        Ok(cfg)
    }

    fn apply_toml(&mut self, t: &TomlConfig) {
        if let Some(v) = &t.project_path {
            self.project_path = v.clone();
        }
        if let Some(v) = &t.output_path {
            self.output_path = v.clone();
        }
        if let Some(v) = &t.internal_path {
            self.internal_path = v.clone();
        }
        if let Some(v) = &t.prompts_dir {
            self.prompts_dir = Some(v.clone());
        }
        if let Some(v) = t.target_language {
            self.target_language = v;
        }
        if let Some(v) = t.max_parallels {
            self.max_parallels = v.max(1);
        }
        if let Some(v) = t.mode {
            self.mode = v;
        }
        if let Some(v) = t.skip_research {
            self.skip_research = v;
        }
        if let Some(v) = t.skip_documentation {
            self.skip_documentation = v;
        }
        if let Some(v) = t.no_cache {
            self.no_cache = v;
        }
        if let Some(v) = t.force_regenerate {
            self.force_regenerate = v;
        }
        if let Some(m) = &t.models {
            if let Some(v) = &m.efficient {
                self.models.efficient = v.clone();
            }
            if let Some(v) = &m.powerful {
                self.models.powerful = v.clone();
            }
        }
        if let Some(l) = &t.limits {
            if let Some(v) = l.daily_cap {
                self.limits.daily_cap = v;
            }
            if let Some(v) = l.call_timeout_s {
                self.limits.call_timeout_s = v;
            }
            if let Some(v) = l.retry_attempts {
                self.limits.retry_attempts = v;
            }
            if let Some(v) = l.materials_char_cap {
                self.limits.materials_char_cap = v;
            }
            if let Some(v) = l.code_insights_limit {
                self.limits.code_insights_limit = v;
            }
            if let Some(v) = l.file_source_chars {
                self.limits.file_source_chars = v;
            }
        }
        if let Some(s) = &t.scan {
            if let Some(v) = s.max_depth {
                self.scan.max_depth = v;
            }
            if let Some(v) = s.max_file_size {
                self.scan.max_file_size = v;
            }
            if let Some(v) = s.git_tracked_only {
                self.scan.git_tracked_only = v;
            }
            if let Some(v) = s.include_hidden {
                self.scan.include_hidden = v;
            }
            if let Some(v) = s.include_tests {
                self.scan.include_tests = v;
            }
            if let Some(v) = &s.excluded_dirs {
                self.scan.excluded_dirs = v.clone();
            }
            if let Some(v) = &s.excluded_files {
                self.scan.excluded_files = v.clone();
            }
        }
        if let Some(v) = &t.verify
            && let Some(b) = v.mermaid_fixer
        {
            self.verify.mermaid_fixer = b;
        }
    }

    /// `~/.config/agentwiki/config.toml` (or `$XDG_CONFIG_HOME/agentwiki/config.toml`).
    pub fn global_config_file() -> Option<PathBuf> {
        global_config_path()
    }

    /// Per-call timeout.
    pub fn call_timeout(&self) -> Duration {
        Duration::from_secs(self.limits.call_timeout_s)
    }

    /// Model string for a tier.
    pub fn model_for(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Efficient => &self.models.efficient,
            ModelTier::Powerful => &self.models.powerful,
        }
    }
}

/// Locate the global user config: `$XDG_CONFIG_HOME/agentwiki/config.toml`,
/// falling back to `~/.config/agentwiki/config.toml`. `None` when absent.
fn global_config_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(xdg).join("agentwiki/config.toml"));
    }
    if let Some(home) = std::env::home_dir() {
        candidates.push(home.join(".config/agentwiki/config.toml"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Record which model tiers a TOML layer sets explicitly — tiers left
/// unset everywhere become eligible for PATH auto-detection.
fn track_models(t: &TomlConfig, set: &mut [bool; 2]) {
    if let Some(m) = &t.models {
        set[0] |= m.efficient.is_some();
        set[1] |= m.powerful.is_some();
    }
}

/// Find profile `name` in TOML (project shadows global). `None` means the
/// name may still resolve to a built-in — see [`builtin_profile`].
fn find_profile<'a>(
    name: &str,
    project: Option<&'a TomlConfig>,
    global: Option<&'a TomlConfig>,
) -> Option<&'a TomlConfig> {
    project
        .and_then(|t| t.profiles.as_ref()?.get(name))
        .or_else(|| global.and_then(|t| t.profiles.as_ref()?.get(name)))
}

/// Built-in profiles, shadowed by any TOML profile of the same name:
/// `default` is a no-op layer; bare backend names (`devin`, `claude`,
/// `codex`) select that CLI's default model pair.
fn builtin_profile(name: &str) -> Option<TomlConfig> {
    if name == "default" {
        return Some(TomlConfig::default());
    }
    let (kind, _) = BackendKind::parse(name).ok()?;
    if kind == BackendKind::Mock {
        return None;
    }
    let (efficient, powerful) = kind.default_models();
    Some(TomlConfig {
        models: Some(ModelsPartial {
            efficient: Some(efficient),
            powerful: Some(powerful),
        }),
        ..Default::default()
    })
}

/// `unknown profile` error listing the built-ins plus every TOML-defined
/// profile name.
fn unknown_profile(name: &str, project: Option<&TomlConfig>, global: Option<&TomlConfig>) -> Error {
    let mut avail: Vec<String> = ["default", "devin", "claude", "codex"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for t in [project, global].into_iter().flatten() {
        if let Some(p) = &t.profiles {
            avail.extend(p.keys().cloned());
        }
    }
    avail.sort();
    avail.dedup();
    Error::Config(format!(
        "unknown profile '{name}' (available: {})",
        avail.join(", ")
    ))
}

/// Read + parse a TOML config file.
fn load_toml(path: &Path) -> Result<TomlConfig> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_then_cli_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("agentwiki.toml");
        std::fs::write(
            &toml_path,
            r#"
max_parallels = 4
target_language = "vi"
[models]
efficient = "devin:swe-2-low"
powerful = "devin:swe-2-max"
[limits]
daily_cap = 5
"#,
        )
        .unwrap();
        let cli = CliOverrides {
            max_parallels: Some(8),
            ..Default::default()
        };
        let cfg = Config::load(&cli, Some(&toml_path)).unwrap();
        assert_eq!(cfg.max_parallels, 8); // CLI wins
        assert_eq!(cfg.models.efficient, "devin:swe-2-low");
        assert_eq!(cfg.limits.daily_cap, 5);
        assert_eq!(cfg.target_language, TargetLanguage::Vi);
        assert_eq!(cfg.models.powerful, "devin:swe-2-max");
    }

    #[test]
    fn cli_model_flags_count_as_explicit() {
        // Both tiers set on the CLI → PATH detection must not touch them.
        let cli = CliOverrides {
            model_efficient: Some("mock:a".to_string()),
            model_powerful: Some("mock:b".to_string()),
            ..Default::default()
        };
        let cfg = Config::load(&cli, None).unwrap();
        assert_eq!(cfg.models.efficient, "mock:a");
        assert_eq!(cfg.models.powerful, "mock:b");
    }

    #[test]
    fn profile_overrides_toml_cli_wins() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("agentwiki.toml");
        std::fs::write(
            &toml_path,
            r#"
target_language = "en"
max_parallels = 2
[profiles.default]
target_language = "vi"
mode = "agentic"
skip_documentation = true
[profiles.research]
skip_documentation = true
"#,
        )
        .unwrap();
        let cli = CliOverrides {
            profile: Some("default".to_string()),
            max_parallels: Some(9),
            ..Default::default()
        };
        let cfg = Config::load(&cli, Some(&toml_path)).unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("default"));
        assert_eq!(cfg.target_language, TargetLanguage::Vi);
        assert_eq!(cfg.mode, Mode::Agentic);
        assert!(cfg.skip_documentation);
        assert_eq!(cfg.max_parallels, 9); // CLI still wins over profile
    }

    #[test]
    fn unknown_profile_errors_with_available_list() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("agentwiki.toml");
        std::fs::write(&toml_path, "[profiles.default]\nmode = \"agentic\"\n").unwrap();
        let cli = CliOverrides {
            profile: Some("nope".to_string()),
            ..Default::default()
        };
        let err = Config::load(&cli, Some(&toml_path)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown profile 'nope'"), "{msg}");
        assert!(msg.contains("default"), "{msg}");
        // Built-in backend names are advertised too.
        for b in ["devin", "claude", "codex"] {
            assert!(msg.contains(b), "{msg}");
        }
    }

    #[test]
    fn default_profile_is_builtin() {
        // No [profiles.default] anywhere → the name still resolves to a
        // no-op layer so `agentwiki default` works on a fresh machine.
        assert!(find_profile("default", None, None).is_none());
        assert!(builtin_profile("nope").is_none());
        let defined = TomlConfig {
            profiles: Some(HashMap::from([(
                "default".to_string(),
                TomlConfig {
                    max_parallels: Some(7),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let p = find_profile("default", Some(&defined), None).unwrap();
        assert_eq!(p.max_parallels, Some(7)); // explicit config wins
    }

    #[test]
    fn backend_names_are_builtin_profiles() {
        let p = builtin_profile("claude").unwrap();
        let m = p.models.unwrap();
        assert_eq!(m.efficient.as_deref(), Some("claude:sonnet@low"));
        assert_eq!(m.powerful.as_deref(), Some("claude:sonnet@high"));
        assert!(builtin_profile("devin").is_some());
        assert!(builtin_profile("codex").is_some());
        // `mock`/`test` parse as a backend but are not profiles.
        assert!(builtin_profile("mock").is_none());
        assert!(builtin_profile("test").is_none());
    }

    #[test]
    fn toml_profile_shadows_builtin_backend() {
        // A TOML [profiles.claude] wins over the built-in `claude`.
        let defined = TomlConfig {
            profiles: Some(HashMap::from([(
                "claude".to_string(),
                TomlConfig {
                    max_parallels: Some(3),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        let p = find_profile("claude", Some(&defined), None).unwrap();
        assert_eq!(p.max_parallels, Some(3));
    }
}
