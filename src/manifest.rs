//! Change manifest — fingerprints of the inputs that produced the current
//! `research.json`, persisted to `<internal>/manifest-<output>.json`.
//!
//! The manifest is a *fingerprint store*, never a source of truth for
//! content: results still live in the content-hash cache. A missing,
//! corrupt, or version-mismatched manifest simply means "no prior state"
//! → the next run is a full run.
//!
//! One manifest per output directory: a repo generating `docs/en` and
//! `docs/vi` tracks each tree's freshness independently. The file name is
//! `manifest-<sha256(abs output path)..12>.json`.
//!
//! Incremental runs use [`classify`] on the manifest diff: cosmetic-only
//! changes short-circuit to a 0-call no-op; anything structural runs the
//! normal pipeline (the content cache still absorbs unchanged leaves).
//! The classifier fails open — doubt resolves to `Structural`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::SCHEMA_VERSION;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::scanner::ScanData;

/// Manifest format version — mismatched files are ignored, not migrated.
pub const MANIFEST_VERSION: u32 = 1;

/// Fingerprint of the scanned inputs at pipeline start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version.
    pub version: u32,
    /// RFC3339 creation time.
    pub created_at: String,
    /// Hash of the generation environment: schema + binary version,
    /// mode, language, output path, models, scan/limits config, and the
    /// full prompt set. Any change classifies as structural.
    pub env: String,
    /// `git rev-parse HEAD` at scan time, when the project is a repo.
    pub git_head: Option<String>,
    /// `rel_path` → sha256 of file content.
    pub files: BTreeMap<String, String>,
    /// `rel_dir` (`.` for root) → signature of its direct member files.
    pub dirs: BTreeMap<String, String>,
    /// Resolved internal import edges `(importer, target)`, sorted —
    /// the architecture signal the classifier diffs.
    pub edges: Vec<(String, String)>,
}

/// What changed between a stored manifest and the current scan.
#[derive(Debug, Default)]
pub struct ManifestDiff {
    /// Generation environment changed (language, prompts, models…).
    pub env_changed: bool,
    /// Files present now but not before.
    pub added_files: Vec<String>,
    /// Files present before but not now.
    pub removed_files: Vec<String>,
    /// Files whose content hash changed.
    pub changed_files: Vec<String>,
    /// Directories present now but not before.
    pub added_dirs: Vec<String>,
    /// Directories present before but not now.
    pub removed_dirs: Vec<String>,
    /// Import edges present now but not before.
    pub added_edges: Vec<(String, String)>,
    /// Import edges present before but not now.
    pub removed_edges: Vec<(String, String)>,
}

/// Change significance for the incremental gate.
#[derive(Debug, PartialEq, Eq)]
pub enum Significance {
    /// Nothing that can alter generated docs changed — safe to no-op.
    Cosmetic,
    /// Structure, dependencies, or the environment changed — regenerate.
    /// Carries human-readable reasons for `status`/logs.
    Structural(Vec<String>),
}

impl ManifestDiff {
    /// No differences at all.
    pub fn is_empty(&self) -> bool {
        !self.env_changed
            && self.added_files.is_empty()
            && self.removed_files.is_empty()
            && self.changed_files.is_empty()
            && self.added_dirs.is_empty()
            && self.removed_dirs.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
    }

    /// Directories containing cosmetically-changed files — the "pending"
    /// set `status` surfaces (their content edits are not yet reflected
    /// in the docs).
    pub fn cosmetic_dirs(&self) -> BTreeSet<String> {
        self.changed_files
            .iter()
            .map(|f| {
                Path::new(f)
                    .parent()
                    .map(|p| {
                        let s = p.to_string_lossy().to_string();
                        if s.is_empty() { ".".to_string() } else { s }
                    })
                    .unwrap_or_else(|| ".".to_string())
            })
            .collect()
    }
}

/// Fail-open classification: anything that can change generated docs —
/// environment, directory membership, file membership, or the resolved
/// import graph — is structural. Only pure content edits inside existing
/// files are cosmetic.
pub fn classify(diff: &ManifestDiff) -> Significance {
    let mut reasons = Vec::new();
    if diff.env_changed {
        reasons.push(
            "generation environment changed (language, prompts, models or scan config)".to_string(),
        );
    }
    if !diff.added_dirs.is_empty() || !diff.removed_dirs.is_empty() {
        reasons.push(format!(
            "directory set changed (+{} −{})",
            diff.added_dirs.len(),
            diff.removed_dirs.len()
        ));
    }
    if !diff.added_files.is_empty() || !diff.removed_files.is_empty() {
        reasons.push(format!(
            "file set changed (+{} −{})",
            diff.added_files.len(),
            diff.removed_files.len()
        ));
    }
    if !diff.added_edges.is_empty() || !diff.removed_edges.is_empty() {
        reasons.push(format!(
            "import graph changed (+{} −{} edges)",
            diff.added_edges.len(),
            diff.removed_edges.len()
        ));
    }
    if reasons.is_empty() {
        Significance::Cosmetic
    } else {
        Significance::Structural(reasons)
    }
}

impl Manifest {
    /// Fingerprint the current scan: file hashes, dir signatures,
    /// resolved import edges, environment, git HEAD.
    ///
    /// Cost is proportional to repo size (one read + one sanitize pass
    /// per file) — trivial next to an LLM call, and paid once per run.
    pub fn build(scan: &ScanData, config: &Config) -> Self {
        let mut files = BTreeMap::new();
        for f in &scan.files {
            let hash = std::fs::read(&f.abs_path)
                .map(|b| hex::encode(Sha256::digest(&b)))
                .unwrap_or_default();
            files.insert(crate::drift::claims::display_path(&f.rel_path), hash);
        }

        let mut dirs = BTreeMap::new();
        for d in &scan.directories {
            let key = if d.rel_path.as_os_str().is_empty() {
                ".".to_string()
            } else {
                crate::drift::claims::display_path(&d.rel_path)
            };
            let mut members: Vec<String> = d
                .files
                .iter()
                .map(|f| {
                    format!(
                        "{}:{}",
                        crate::drift::claims::display_path(&f.rel_path),
                        files
                            .get(&crate::drift::claims::display_path(&f.rel_path))
                            .map(|s| s.as_str())
                            .unwrap_or("")
                    )
                })
                .collect();
            members.sort();
            dirs.insert(key, hash_strs(&members));
        }

        let test_globs: Vec<glob::Pattern> = config
            .drift
            .test_globs
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        let fg =
            crate::drift::resolve::build_file_graph(&config.drift, &scan.root, scan, &test_globs);
        let edges: BTreeSet<(String, String)> = fg
            .edges
            .iter()
            .map(|e| {
                (
                    crate::drift::claims::display_path(&e.importer),
                    crate::drift::claims::display_path(&e.target),
                )
            })
            .collect();

        Self {
            version: MANIFEST_VERSION,
            created_at: crate::quota::now_rfc3339(),
            env: env_fingerprint(config),
            git_head: git_head(&scan.root),
            files,
            dirs,
            edges: edges.into_iter().collect(),
        }
    }

    /// Load a stored manifest; `None` on missing, corrupt, or wrong
    /// version — all meaning "no usable prior state".
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let m: Self = serde_json::from_str(&text).ok()?;
        (m.version == MANIFEST_VERSION).then_some(m)
    }

    /// Persist (atomic write, best effort reported by caller).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Pipeline(format!("manifest serialize: {e}")))?;
        crate::util::write_atomic(path, body.as_bytes())
    }

    /// Diff this (older) manifest against a fresh one.
    pub fn diff(&self, cur: &Manifest) -> ManifestDiff {
        let keys: BTreeSet<&String> = self.files.keys().chain(cur.files.keys()).collect();
        let (mut added_files, mut removed_files, mut changed_files) = (vec![], vec![], vec![]);
        for k in keys {
            match (self.files.get(k), cur.files.get(k)) {
                (None, Some(_)) => added_files.push(k.clone()),
                (Some(_), None) => removed_files.push(k.clone()),
                (Some(a), Some(b)) if a != b => changed_files.push(k.clone()),
                _ => {}
            }
        }
        let dir_keys: BTreeSet<&String> = self.dirs.keys().chain(cur.dirs.keys()).collect();
        let (mut added_dirs, mut removed_dirs) = (vec![], vec![]);
        for k in dir_keys {
            match (self.dirs.contains_key(k), cur.dirs.contains_key(k)) {
                (false, true) => added_dirs.push(k.clone()),
                (true, false) => removed_dirs.push(k.clone()),
                _ => {}
            }
        }
        let old: BTreeSet<&(String, String)> = self.edges.iter().collect();
        let new: BTreeSet<&(String, String)> = cur.edges.iter().collect();
        ManifestDiff {
            env_changed: self.env != cur.env,
            added_files,
            removed_files,
            changed_files,
            added_dirs,
            removed_dirs,
            added_edges: new.difference(&old).map(|e| (*e).clone()).collect(),
            removed_edges: old.difference(&new).map(|e| (*e).clone()).collect(),
        }
    }

    /// Content fingerprint of every scanned file — the cache-key input
    /// for agentic-mode agents that may read the whole repo.
    pub fn fingerprint_all(&self) -> String {
        let pairs: Vec<String> = self.files.iter().map(|(p, h)| format!("{p}:{h}")).collect();
        hash_strs(&pairs)
    }

    /// Content fingerprint of one directory's whole subtree — the
    /// cache-key input for agentic `dir_summary@<dir>` (the agent's cwd
    /// is the repo root; its mandate is that directory).
    pub fn subtree_fingerprint(&self, dir: &str) -> String {
        if dir.is_empty() || dir == "." {
            return self.fingerprint_all();
        }
        let prefix = format!("{dir}/");
        let pairs: Vec<String> = self
            .files
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .map(|(p, h)| format!("{p}:{h}"))
            .collect();
        hash_strs(&pairs)
    }
}

/// `<internal>/manifest-<sha12>.json`, keyed by the absolute output path
/// so multi-language/multi-output repos keep independent freshness.
pub fn manifest_path(config: &Config) -> PathBuf {
    let out = &config.output_path;
    let abs = if out.is_absolute() {
        out.clone()
    } else {
        config.project_path.join(out)
    };
    let abs = abs.canonicalize().unwrap_or(abs);
    let digest = hex::encode(Sha256::digest(abs.to_string_lossy().as_bytes()));
    config
        .internal_path
        .join(format!("manifest-{}.json", &digest[..12]))
}

/// `git rev-parse HEAD` in `root`; `None` when git or the repo is absent.
pub fn git_head(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `git rev-list --count <since>..HEAD`; `None` when unavailable.
pub fn commits_since(root: &Path, since: &str) -> Option<usize> {
    let out = std::process::Command::new("git")
        .args(["rev-list", "--count", &format!("{since}..HEAD")])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Hash of the non-code inputs that shape generation — the "manifest
/// key ⊇ cache key" side: everything `Cache::key` doesn't see.
fn env_fingerprint(config: &Config) -> String {
    let mut h = Sha256::new();
    let mut feed = |s: &str| {
        h.update(s.as_bytes());
        h.update(b"\x00");
    };
    feed(SCHEMA_VERSION);
    feed(env!("CARGO_PKG_VERSION"));
    feed(&format!("{:?}", config.mode));
    feed(&format!("{:?}", config.target_language));
    let out = &config.output_path;
    let abs = if out.is_absolute() {
        out.clone()
    } else {
        config.project_path.join(out)
    };
    feed(&abs.canonicalize().unwrap_or(abs).to_string_lossy());
    feed(&config.models.efficient);
    feed(&config.models.powerful);
    feed(&config.limits.daily_cap.to_string());
    feed(&config.limits.call_timeout_s.to_string());
    feed(&config.limits.retry_attempts.to_string());
    feed(&config.limits.materials_char_cap.to_string());
    feed(&config.limits.code_insights_limit.to_string());
    feed(&config.limits.file_source_chars.to_string());
    feed(&config.scan.max_depth.to_string());
    feed(&config.scan.max_file_size.to_string());
    feed(&config.scan.git_tracked_only.to_string());
    feed(&config.scan.include_hidden.to_string());
    feed(&config.scan.include_tests.to_string());
    for d in &config.scan.excluded_dirs {
        feed(d);
    }
    for f in &config.scan.excluded_files {
        feed(f);
    }
    // Prompt set: template name + resolved content (covers embedded
    // edits and `prompts_dir` overrides alike).
    let loader = crate::prompt::PromptLoader::new(config.prompts_dir.clone());
    for spec in crate::agent::registry::all_specs() {
        if !spec.prompt_tmpl.is_empty()
            && let Ok(t) = loader.load(spec.prompt_tmpl)
        {
            feed(spec.prompt_tmpl);
            feed(&t);
        }
    }
    hex::encode(h.finalize())
}

fn hash_strs(items: &[String]) -> String {
    let mut h = Sha256::new();
    for s in items {
        h.update(s.as_bytes());
        h.update(b"\x00");
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{self};

    /// `tempdir()` roots are dot-prefixed (`.tmpXXX`) and `include_hidden`
    /// prunes them at the walk root — the fixture must live in a normal
    /// child directory.
    fn project_dir(tmp: &Path) -> PathBuf {
        tmp.join("proj")
    }

    fn write_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.py"), "from .b import f\n").unwrap();
        std::fs::write(root.join("src/b.py"), "def f():\n    return 1\n").unwrap();
    }

    fn build_manifest(root: &Path) -> Manifest {
        let mut config = Config {
            project_path: root.to_path_buf(),
            ..Default::default()
        };
        config.scan.git_tracked_only = false;
        let scan = scanner::scan(&config).unwrap();
        Manifest::build(&scan, &config)
    }

    fn manifest_fixture(root: &Path) -> Manifest {
        write_fixture(root);
        build_manifest(root)
    }

    #[test]
    fn content_only_change_is_cosmetic() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m1 = manifest_fixture(&root);
        // Comment-only edit — no import change.
        std::fs::write(root.join("src/b.py"), "# hi\ndef f():\n    return 1\n").unwrap();
        let m2 = build_manifest(&root);
        let diff = m1.diff(&m2);
        assert_eq!(diff.changed_files, vec!["src/b.py".to_string()]);
        assert_eq!(classify(&diff), Significance::Cosmetic);
        assert!(diff.cosmetic_dirs().contains("src"));
    }

    #[test]
    fn external_import_stays_cosmetic() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m1 = manifest_fixture(&root);
        // `import os` is external — no internal edge appears.
        std::fs::write(
            root.join("src/b.py"),
            "import os\n\ndef f():\n    return 1\n",
        )
        .unwrap();
        let m2 = build_manifest(&root);
        assert_eq!(classify(&m1.diff(&m2)), Significance::Cosmetic);
    }

    #[test]
    fn new_file_and_edge_are_structural() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m1 = manifest_fixture(&root);
        std::fs::write(root.join("src/c.py"), "from .a import f\n").unwrap();
        let m2 = build_manifest(&root);
        let diff = m1.diff(&m2);
        assert!(matches!(classify(&diff), Significance::Structural(_)));
        assert_eq!(diff.added_files, vec!["src/c.py".to_string()]);
        assert_eq!(
            diff.added_edges,
            vec![("src/c.py".to_string(), "src/a.py".to_string())]
        );
    }

    #[test]
    fn removed_file_is_structural() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m1 = manifest_fixture(&root);
        std::fs::remove_file(root.join("src/b.py")).unwrap();
        let m2 = build_manifest(&root);
        let diff = m1.diff(&m2);
        assert!(matches!(classify(&diff), Significance::Structural(_)));
        assert_eq!(diff.removed_files, vec!["src/b.py".to_string()]);
        // The `a.py → b.py` edge vanished too.
        assert_eq!(diff.removed_edges.len(), 1);
    }

    #[test]
    fn env_change_is_structural() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m1 = manifest_fixture(&root);
        let mut config = Config {
            project_path: root.clone(),
            target_language: crate::config::TargetLanguage::Vi,
            ..Default::default()
        };
        config.scan.git_tracked_only = false;
        let scan = scanner::scan(&config).unwrap();
        let m2 = Manifest::build(&scan, &config);
        let diff = m1.diff(&m2);
        assert!(diff.env_changed);
        assert!(matches!(classify(&diff), Significance::Structural(_)));
    }

    #[test]
    fn roundtrip_and_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let root = project_dir(tmp.path());
        let m = manifest_fixture(&root);
        let p = tmp.path().join("manifest.json");
        m.save(&p).unwrap();
        let loaded = Manifest::load(&p).unwrap();
        assert_eq!(loaded.files, m.files);
        assert_eq!(loaded.edges, m.edges);
        assert!(loaded.diff(&m).is_empty());
        // Corrupt → None (caller falls back to a full run).
        std::fs::write(&p, "not json").unwrap();
        assert!(Manifest::load(&p).is_none());
    }
}
