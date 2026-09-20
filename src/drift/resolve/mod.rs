//! Import → file resolution and the per-repo file graph build.
//!
//! `resolve()` maps one raw import to `Internal(files)` / `External` /
//! `Unresolved`. `build_file_graph` runs extract+resolve over every
//! scanned code file into a [`FileGraph`].

pub mod js;
pub mod python;
pub mod rust;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::drift::DriftConfig;
use crate::drift::graph::{FileEdge, FileGraph};
use crate::drift::imports::{self, ImportSpec, Lang, RawImport};
use crate::scanner::ScanData;

/// Outcome of resolving one raw import.
#[derive(Debug)]
pub enum Resolution {
    /// Repo-internal target file(s).
    Internal(Vec<PathBuf>),
    /// Third-party/stdlib — not evidence, not unresolved.
    External,
    /// Looked internal but couldn't be pinned (aliases, ambiguity).
    Unresolved,
}

/// The scanned file set plus per-language indexes.
pub struct RepoIndex {
    /// All scanned repo-relative paths.
    pub files: BTreeSet<PathBuf>,
    /// Rust crate layout.
    pub rust: rust::RustIndex,
    /// `.py` files for suffix matching.
    pub py: BTreeSet<PathBuf>,
}

impl RepoIndex {
    /// Build indexes from a scan's file list + repo root.
    pub fn build(root: &Path, scan: &ScanData) -> Self {
        let files: BTreeSet<PathBuf> = scan.files.iter().map(|f| f.rel_path.clone()).collect();
        let py = files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "py"))
            .cloned()
            .collect();
        Self {
            rust: rust::build(root, &files),
            py,
            files,
        }
    }

    /// Crate name for a Rust file (for `use <name>::…` recognition).
    pub fn crate_name_of(&self, f: &PathBuf) -> Option<&str> {
        self.rust
            .file_mod
            .get(f)
            .map(|(i, _)| self.rust.crates[*i].name.as_str())
            .filter(|s| !s.is_empty())
    }
}

/// Resolve one raw import from `importer` (repo-relative).
pub fn resolve(idx: &RepoIndex, importer: &PathBuf, imp: &RawImport) -> Resolution {
    match &imp.spec {
        ImportSpec::Rust { root, segs } => {
            rust::resolve(&idx.rust, &idx.files, importer, root, segs)
        }
        ImportSpec::Python {
            level,
            module,
            names,
        } => python::resolve(&idx.py, importer, *level, module, names),
        ImportSpec::Js { specifier } => js::resolve(&idx.files, importer, specifier),
    }
}

/// Extract + resolve imports for every scanned supported-language file.
/// Unreadable files are skipped (they simply contribute no edges).
pub fn build_file_graph(
    cfg: &DriftConfig,
    root: &Path,
    scan: &ScanData,
    test_globs: &[glob::Pattern],
) -> FileGraph {
    let idx = RepoIndex::build(root, scan);
    let mut g = FileGraph::default();
    for f in &scan.files {
        let lang = f
            .extension
            .as_deref()
            .map(Lang::from_extension)
            .unwrap_or(Lang::Other);
        if !lang.is_supported() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&f.abs_path) else {
            continue;
        };
        let sanitized = imports::sanitize::sanitize(lang, &content);
        let rel_str = f.rel_path.as_os_str().to_string_lossy().replace('\\', "/");
        let test_file = test_globs.iter().any(|p| p.matches(&rel_str));
        let crate_name = idx.crate_name_of(&f.rel_path);
        let raw = imports::extract_imports(
            lang,
            &f.rel_path,
            &sanitized,
            test_file,
            crate_name,
            cfg.exclude_cfg_test,
        );
        let mut unresolved = 0usize;
        for imp in raw {
            *g.total_imports.entry(f.rel_path.clone()).or_default() += 1;
            match resolve(&idx, &f.rel_path, &imp) {
                Resolution::Internal(targets) => {
                    for t in targets {
                        if t != f.rel_path {
                            g.edges.push(FileEdge {
                                importer: f.rel_path.clone(),
                                target: t,
                                kind: imp.kind,
                                test_only: imp.test_only,
                                line: imp.line,
                            });
                        }
                    }
                }
                Resolution::External => {}
                Resolution::Unresolved => unresolved += 1,
            }
        }
        if unresolved > 0 {
            g.unresolved.insert(f.rel_path.clone(), unresolved);
        }
    }
    // Weak edges through re-export facades (lib.rs, mod.rs, __init__.py,
    // index.*) — import of a facade also reaches what it re-exports.
    crate::drift::graph::facade_closure(&mut g.edges);
    g
}
