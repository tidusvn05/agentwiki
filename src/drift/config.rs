//! `[drift]` config section — thresholds and filters for `agentwiki drift`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Resolved `[drift]` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DriftConfig {
    /// Claims file override (`--claims` wins over this).
    pub claims_path: Option<PathBuf>,
    /// Baseline file override (default `<project>/.agentwiki-drift-baseline.json`).
    pub baseline_path: Option<PathBuf>,
    /// Max hops for a transitive `confirmed` path.
    pub max_transitive_depth: usize,
    /// `dependency_type` labels that import evidence can verify
    /// (`data_flow` is never checkable).
    pub checkable_kinds: BTreeSet<String>,
    /// Min fraction of a node's code files in a supported language.
    pub min_language_coverage: f64,
    /// Max fraction of a node's imports allowed to stay `Unresolved`.
    pub max_unresolved_ratio: f64,
    /// Node in-degree / node-count ratio at which a node is a hub.
    pub hub_in_degree_ratio: f64,
    /// Hub detection only kicks in at this many nodes.
    pub hub_min_nodes: usize,
    /// Distinct (importer, target) file pairs behind an undocumented edge.
    pub min_undocumented_imports: usize,
    /// Cap on reported undocumented findings.
    pub max_undocumented_reported: usize,
    /// Claimed-node paths excluded from undocumented findings.
    pub ignore_nodes: Vec<String>,
    /// File globs excluded from undocumented findings (utility files).
    pub ignore_files: Vec<String>,
    /// Path globs marking test files (their imports are `test_only`).
    pub test_globs: Vec<String>,
    /// Imports under `#[cfg(test)]` count as `test_only`.
    pub exclude_cfg_test: bool,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            claims_path: None,
            baseline_path: None,
            max_transitive_depth: 3,
            checkable_kinds: [
                "import",
                "function_call",
                "inheritance",
                "composition",
                "module",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            min_language_coverage: 0.8,
            max_unresolved_ratio: 0.25,
            hub_in_degree_ratio: 0.5,
            hub_min_nodes: 5,
            min_undocumented_imports: 3,
            max_undocumented_reported: 20,
            ignore_nodes: Vec::new(),
            ignore_files: [
                "**/error.*",
                "**/errors.*",
                "**/config.*",
                "**/util.*",
                "**/utils.*",
                "**/utils/**",
                "**/types.*",
                "**/constants.*",
                "**/prelude.*",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            test_globs: [
                "tests/**",
                "test/**",
                "**/__tests__/**",
                "**/*_test.*",
                "**/test_*.py",
                "**/*.spec.*",
                "**/*.test.*",
                "**/conftest.py",
                "benches/**",
                "examples/**",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            exclude_cfg_test: true,
        }
    }
}

/// Optional TOML shape for `[drift]` — every field optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DriftPartial {
    /// See [`DriftConfig`].
    pub claims_path: Option<PathBuf>,
    /// See [`DriftConfig`].
    pub baseline_path: Option<PathBuf>,
    /// See [`DriftConfig`].
    pub max_transitive_depth: Option<usize>,
    /// See [`DriftConfig`].
    pub checkable_kinds: Option<Vec<String>>,
    /// See [`DriftConfig`].
    pub min_language_coverage: Option<f64>,
    /// See [`DriftConfig`].
    pub max_unresolved_ratio: Option<f64>,
    /// See [`DriftConfig`].
    pub hub_in_degree_ratio: Option<f64>,
    /// See [`DriftConfig`].
    pub hub_min_nodes: Option<usize>,
    /// See [`DriftConfig`].
    pub min_undocumented_imports: Option<usize>,
    /// See [`DriftConfig`].
    pub max_undocumented_reported: Option<usize>,
    /// See [`DriftConfig`].
    pub ignore_nodes: Option<Vec<String>>,
    /// See [`DriftConfig`].
    pub ignore_files: Option<Vec<String>>,
    /// See [`DriftConfig`].
    pub test_globs: Option<Vec<String>>,
    /// See [`DriftConfig`].
    pub exclude_cfg_test: Option<bool>,
}

impl DriftConfig {
    /// Claims file to read: `[drift].claims_path` (repo-relative) or the
    /// default `<internal>/research.json`. `--claims` wins over both.
    pub fn claims_path(&self, project_root: &Path, internal: &Path) -> PathBuf {
        match &self.claims_path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => project_root.join(p),
            None => internal.join("research.json"),
        }
    }

    /// Baseline file: `[drift].baseline_path` (repo-relative) or
    /// `<project>/.agentwiki-drift-baseline.json`.
    pub fn baseline_path(&self, project_root: &Path) -> PathBuf {
        match &self.baseline_path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => project_root.join(p),
            None => project_root.join(".agentwiki-drift-baseline.json"),
        }
    }

    /// Merge a `[drift]` TOML section over `self`.
    pub fn apply(&mut self, p: &DriftPartial) {
        if let Some(v) = &p.claims_path {
            self.claims_path = Some(v.clone());
        }
        if let Some(v) = &p.baseline_path {
            self.baseline_path = Some(v.clone());
        }
        if let Some(v) = p.max_transitive_depth {
            self.max_transitive_depth = v.max(1);
        }
        if let Some(v) = &p.checkable_kinds {
            self.checkable_kinds = v.iter().cloned().collect();
        }
        if let Some(v) = p.min_language_coverage {
            self.min_language_coverage = v.clamp(0.0, 1.0);
        }
        if let Some(v) = p.max_unresolved_ratio {
            self.max_unresolved_ratio = v.clamp(0.0, 1.0);
        }
        if let Some(v) = p.hub_in_degree_ratio {
            self.hub_in_degree_ratio = v.clamp(0.0, 1.0);
        }
        if let Some(v) = p.hub_min_nodes {
            self.hub_min_nodes = v;
        }
        if let Some(v) = p.min_undocumented_imports {
            self.min_undocumented_imports = v.max(1);
        }
        if let Some(v) = p.max_undocumented_reported {
            self.max_undocumented_reported = v;
        }
        if let Some(v) = &p.ignore_nodes {
            self.ignore_nodes = v.clone();
        }
        if let Some(v) = &p.ignore_files {
            self.ignore_files = v.clone();
        }
        if let Some(v) = &p.test_globs {
            self.test_globs = v.clone();
        }
        if let Some(v) = p.exclude_cfg_test {
            self.exclude_cfg_test = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_merge_keeps_defaults() {
        let mut c = DriftConfig::default();
        let p: DriftPartial = toml::from_str(
            r#"
            max_transitive_depth = 5
            min_undocumented_imports = 1
            ignore_nodes = ["src/generated"]
            "#,
        )
        .unwrap();
        c.apply(&p);
        assert_eq!(c.max_transitive_depth, 5);
        assert_eq!(c.min_undocumented_imports, 1);
        assert_eq!(c.ignore_nodes, ["src/generated"]);
        // untouched fields keep defaults
        assert_eq!(c.hub_min_nodes, 5);
        assert!(c.exclude_cfg_test);
    }

    #[test]
    fn values_are_clamped() {
        let mut c = DriftConfig::default();
        let p: DriftPartial = toml::from_str(
            "max_transitive_depth = 0\nmin_language_coverage = 3.0\nmin_undocumented_imports = 0\n",
        )
        .unwrap();
        c.apply(&p);
        assert_eq!(c.max_transitive_depth, 1);
        assert_eq!(c.min_language_coverage, 1.0);
        assert_eq!(c.min_undocumented_imports, 1);
    }
}
