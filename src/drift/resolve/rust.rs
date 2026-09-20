//! Rust module resolution: crate layout from `Cargo.toml` positions +
//! `crate`/`self`/`super`/`<name>` path roots → repo files.
//!
//! Layout rules: `src/lib.rs`/`src/main.rs`/`src/bin/*.rs` and each file
//! in `tests/` `examples/` `benches/` are crate roots. `mod x;` in a
//! root resolves under its own dir; in `foo.rs` under `foo/`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::Resolution;
use crate::drift::imports::RustRoot;

/// One crate root discovered in the repo.
#[derive(Debug)]
pub struct RustCrate {
    /// Normalized package/crate name (`-`→`_`); "" for anonymous roots
    /// (integration tests, examples).
    pub name: String,
    /// Crate root file (repo-relative).
    pub root_file: PathBuf,
    /// Dir where `mod x;` / `crate::x` descent starts.
    pub children: PathBuf,
    /// Library root (`src/lib.rs`) — the target of `use <name>::…`.
    pub is_lib: bool,
}

/// Precomputed Rust layout: per-file module context + module→file map.
#[derive(Debug, Default)]
pub struct RustIndex {
    /// All crate roots.
    pub crates: Vec<RustCrate>,
    /// file → (index into `crates`, module path).
    pub file_mod: BTreeMap<PathBuf, (usize, Vec<String>)>,
    /// file → dir where its `mod x;` children live.
    pub children_dir: BTreeMap<PathBuf, PathBuf>,
    /// (crate idx, module path) → file.
    pub by_module: BTreeMap<(usize, Vec<String>), PathBuf>,
    /// package name → lib crate idx.
    pub lib_crates: BTreeMap<String, usize>,
}

/// Build the Rust layout index from scanned files.
pub fn build(root: &Path, files: &BTreeSet<PathBuf>) -> RustIndex {
    let mut idx = RustIndex::default();
    let mut cargo_names: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut crate_of: BTreeMap<PathBuf, usize> = BTreeMap::new(); // file → crate idx

    for f in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
    {
        let pkg = find_pkg_root(root, f);
        let (src_dir, children, module, is_root) = match &pkg {
            Some(pkg) => classify(f, pkg),
            None => (
                PathBuf::new(),
                f.parent().unwrap_or(Path::new("")).to_path_buf(),
                Vec::new(),
                true,
            ),
        };
        let crate_idx = if is_root {
            // A root may already exist — created lazily when one of its
            // module files was indexed first.
            *crate_of.entry(f.clone()).or_insert_with(|| {
                let name = pkg
                    .as_ref()
                    .map(|p| {
                        cargo_names
                            .entry(p.clone())
                            .or_insert_with(|| pkg_name(root, p))
                            .clone()
                    })
                    .unwrap_or_default();
                idx.crates.push(RustCrate {
                    name,
                    root_file: f.clone(),
                    children: children.clone(),
                    is_lib: f.file_name().is_some_and(|n| n == "lib.rs"),
                });
                idx.crates.len() - 1
            })
        } else {
            // Belongs to the package's lib crate (or main.rs bin).
            let lib = src_dir.join("lib.rs");
            let main = src_dir.join("main.rs");
            let root_file = if files.contains(&lib) { lib } else { main };
            *crate_of.entry(root_file.clone()).or_insert_with(|| {
                let name = pkg
                    .as_ref()
                    .map(|p| {
                        cargo_names
                            .entry(p.clone())
                            .or_insert_with(|| pkg_name(root, p))
                            .clone()
                    })
                    .unwrap_or_default();
                idx.crates.push(RustCrate {
                    name,
                    children: src_dir.clone(),
                    root_file,
                    is_lib: true,
                });
                idx.crates.len() - 1
            })
        };
        idx.file_mod.insert(f.clone(), (crate_idx, module.clone()));
        idx.children_dir.insert(f.clone(), children);
        idx.by_module.insert((crate_idx, module), f.clone());
    }
    for (i, c) in idx.crates.iter().enumerate() {
        if c.is_lib && !c.name.is_empty() {
            idx.lib_crates.entry(c.name.clone()).or_insert(i);
        }
    }
    idx
}

/// Nearest ancestor dir of `f` (repo-relative) containing `Cargo.toml`.
fn find_pkg_root(root: &Path, f: &Path) -> Option<PathBuf> {
    let mut d = f.parent();
    while let Some(dir) = d {
        if root.join(dir).join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        d = dir.parent();
    }
    root.join("Cargo.toml").is_file().then(PathBuf::new)
}

/// `(src_dir, children_dir, module_path, is_crate_root)` for file `f`
/// under package `pkg` (repo-relative paths).
fn classify(f: &Path, pkg: &Path) -> (PathBuf, PathBuf, Vec<String>, bool) {
    let src_dir = pkg.join("src");
    let name = f.file_name().unwrap_or_default().to_string_lossy();
    let segs_of = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect()
    };

    if let Ok(rel) = f.strip_prefix(&src_dir) {
        // Under src/: lib.rs/main.rs/mod.rs anchor to their own dir;
        // bin/*.rs are roots; foo.rs is a module file.
        let mut module: Vec<String> = segs_of(rel.parent().unwrap_or(Path::new("")));
        let is_anchor = matches!(name.as_ref(), "lib.rs" | "main.rs" | "mod.rs");
        if rel.parent().is_some_and(|p| p == Path::new("bin")) && name != "mod.rs" {
            return (
                src_dir.clone(),
                f.parent().unwrap().to_path_buf(),
                vec![],
                true,
            );
        }
        if !is_anchor {
            module.push(name.trim_end_matches(".rs").to_string());
        }
        let children = if is_anchor {
            f.parent().unwrap().to_path_buf()
        } else {
            f.parent().unwrap().join(name.trim_end_matches(".rs"))
        };
        let is_root = matches!(name.as_ref(), "lib.rs" | "main.rs")
            && rel.parent().is_none_or(|p| p.as_os_str().is_empty());
        return (src_dir, children, module, is_root);
    }
    // tests/, examples/, benches/ — every file is an anonymous crate root.
    for top in ["tests", "examples", "benches"] {
        let base = pkg.join(top);
        if f.starts_with(&base) {
            return (src_dir, f.parent().unwrap().to_path_buf(), vec![], true);
        }
    }
    // Unknown layout — file is its own root.
    (
        src_dir,
        f.parent().unwrap_or(Path::new("")).to_path_buf(),
        vec![],
        true,
    )
}

/// `[package] name` from `pkg/Cargo.toml` (dash→underscore); falls back
/// to the dir name.
fn pkg_name(root: &Path, pkg: &PathBuf) -> String {
    let p = root.join(pkg).join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&p)
        && let Ok(v) = text.parse::<toml::Table>()
        && let Some(n) = v
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
    {
        return n.replace('-', "_");
    }
    pkg.file_name()
        .map(|n| n.to_string_lossy().replace('-', "_"))
        .unwrap_or_default()
}

/// Resolve a Rust path spec from `importer`.
pub fn resolve(
    idx: &RustIndex,
    files: &std::collections::BTreeSet<PathBuf>,
    importer: &PathBuf,
    root: &RustRoot,
    segs: &[String],
) -> Resolution {
    let Some((crate_idx, file_mod)) = idx.file_mod.get(importer).cloned() else {
        return Resolution::Unresolved;
    };
    let (base_file, base_dir) = match root {
        RustRoot::Crate => {
            let c = &idx.crates[crate_idx];
            (c.root_file.clone(), c.children.clone())
        }
        RustRoot::Named(n) => {
            if let Some(&ci) = idx.lib_crates.get(n) {
                let c = &idx.crates[ci];
                (c.root_file.clone(), c.children.clone())
            } else {
                // 2015-style / local path: `use foo::x` where `foo` is a
                // sibling module of the importer. Only internal if the
                // first segment hits a module file — else it's an
                // extern crate.
                let dir = idx.children_dir.get(importer).cloned().unwrap_or_default();
                let mut full = vec![n.clone()];
                full.extend(segs.iter().cloned());
                let f = dir.join(format!("{n}.rs"));
                let m = dir.join(n).join("mod.rs");
                if files.contains(&f) || files.contains(&m) {
                    return Resolution::Internal(vec![descend(
                        files,
                        importer.clone(),
                        &dir,
                        &full,
                    )]);
                }
                return Resolution::External;
            }
        }
        RustRoot::SelfMod { inline } => {
            let dir = idx.children_dir.get(importer).cloned().unwrap_or_default();
            let base = inline.iter().fold(dir, |d, s| d.join(s));
            (importer.clone(), base)
        }
        RustRoot::Super { levels, inline } => {
            let mut ctx = file_mod.clone();
            ctx.extend(inline.iter().cloned());
            if *levels > ctx.len() {
                return Resolution::Unresolved;
            }
            ctx.truncate(ctx.len() - levels);
            if ctx.len() >= file_mod.len() {
                // Still inside the file's own subtree (inline mods).
                let dir = idx.children_dir.get(importer).cloned().unwrap_or_default();
                let base = ctx[file_mod.len()..].iter().fold(dir, |d, s| d.join(s));
                (importer.clone(), base)
            } else {
                match idx.by_module.get(&(crate_idx, ctx.clone())) {
                    Some(f) => {
                        let d = idx.children_dir.get(f).cloned().unwrap_or_default();
                        (f.clone(), d)
                    }
                    None => return Resolution::Unresolved,
                }
            }
        }
    };
    Resolution::Internal(vec![descend(files, base_file, &base_dir, segs)])
}

/// Walk module segments from `base_dir`: `seg` → `seg.rs` or
/// `seg/mod.rs`; stop at the first non-module segment (an item).
/// Returns the deepest file reached (never below `base_file`).
fn descend(
    files: &std::collections::BTreeSet<PathBuf>,
    base_file: PathBuf,
    base_dir: &Path,
    segs: &[String],
) -> PathBuf {
    let mut cur = base_file;
    let mut dir = base_dir.to_path_buf();
    for seg in segs {
        if seg == "*" || seg == "self" {
            break;
        }
        let file = dir.join(format!("{seg}.rs"));
        let modrs = dir.join(seg).join("mod.rs");
        if files.contains(&file) {
            dir = dir.join(seg); // children of foo.rs live in foo/
            cur = file;
        } else if files.contains(&modrs) {
            dir = dir.join(seg);
            cur = modrs;
        } else {
            break; // item of the current module — edge to `cur`
        }
    }
    cur
}
