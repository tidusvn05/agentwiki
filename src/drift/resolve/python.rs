//! Python resolution: relative `from .x import y`, `from ..p import z`,
//! `from . import x`, and absolute imports via path-suffix lookup
//! (ambiguous → `Unresolved`).
//!
//! `from pkg import sub` tries `sub` as a submodule first, else treats it
//! as an item of the module/package file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::Resolution;

/// Resolve a Python import spec. May yield several internal targets
/// (`from p import a, b` where both are submodules).
pub fn resolve(
    py_files: &BTreeSet<PathBuf>,
    importer: &Path,
    level: u32,
    module: &[String],
    names: &[String],
) -> Resolution {
    let dir = importer
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    if level > 0 {
        // `from .` = importer's package dir; each extra dot climbs one.
        let mut base = dir.clone();
        for _ in 1..level {
            match base.parent() {
                Some(p) => base = p.to_path_buf(),
                None => return Resolution::Unresolved,
            }
        }
        if module.is_empty() {
            // `from . import x` — x is a submodule or an item of the
            // package's `__init__.py`.
            let mut hits = Vec::new();
            let mut leftover = false;
            for n in names {
                if let Some(f) = module_file(&base.join(n), py_files) {
                    hits.push(f);
                } else {
                    leftover = true;
                }
            }
            if leftover {
                let init = base.join("__init__.py");
                if py_files.contains(&init) {
                    hits.push(init);
                }
            }
            return if hits.is_empty() {
                Resolution::Unresolved
            } else {
                Resolution::Internal(hits)
            };
        }
        let Some(mf) = resolve_module(&base, module, py_files) else {
            return Resolution::Unresolved;
        };
        return resolve_names(py_files, &mf, names);
    }

    // Absolute: path-suffix match over the repo's `.py` files.
    let mut cands: Vec<PathBuf> = Vec::new();
    for f in py_files {
        if module_match(f, module) {
            cands.push(f.clone());
        }
    }
    match cands.len() {
        0 => Resolution::External,
        1 => resolve_names(py_files, &cands[0], names),
        _ => Resolution::Unresolved,
    }
}

/// `base/<module segs>` → `x.py` or `x/__init__.py`.
fn resolve_module(base: &Path, segs: &[String], files: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    let p = segs.iter().fold(base.to_path_buf(), |d, s| d.join(s));
    module_file(&p, files)
}

/// `x` → `x.py` or `x/__init__.py`.
fn module_file(p: &Path, files: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    let as_file = p.with_extension("py");
    if files.contains(&as_file) {
        return Some(as_file);
    }
    let as_init = p.join("__init__.py");
    if files.contains(&as_init) {
        return Some(as_init);
    }
    None
}

/// `f` ends with `<segs>.py` or `<segs>/__init__.py` (component-aligned).
fn module_match(f: &Path, segs: &[String]) -> bool {
    let fs = f.as_os_str().to_string_lossy().replace('\\', "/");
    let joined = segs.join("/");
    for suffix in [format!("{joined}.py"), format!("{joined}/__init__.py")] {
        if fs == suffix || fs.ends_with(&format!("/{suffix}")) {
            return true;
        }
    }
    false
}

/// `from m import a, b`: each name tries as submodule of `m` first; the
/// rest resolve to `m` itself.
fn resolve_names(files: &BTreeSet<PathBuf>, mfile: &Path, names: &[String]) -> Resolution {
    // Only packages (an `__init__.py` target) can hold submodules.
    let is_pkg = mfile.file_name().is_some_and(|n| n == "__init__.py");
    let mut hits = Vec::new();
    let mut leftover = false;
    for n in names {
        let sub = is_pkg.then(|| {
            let dir = mfile.parent().unwrap().to_path_buf();
            module_file(&dir.join(n), files)
        });
        match sub.flatten() {
            Some(f) => hits.push(f),
            None => leftover = true,
        }
    }
    if leftover || hits.is_empty() {
        hits.push(mfile.to_path_buf());
    }
    Resolution::Internal(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> BTreeSet<PathBuf> {
        [
            "src/main.py",
            "src/api.py",
            "src/models.py",
            "src/pkg/__init__.py",
            "src/pkg/sub.py",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }

    fn imp() -> PathBuf {
        PathBuf::from("src/main.py")
    }

    #[test]
    fn relative_sibling() {
        match resolve(&files(), &imp(), 1, &["models".into()], &["Task".into()]) {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/models.py")]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn relative_up_and_pkg_submodule() {
        // `from ..pkg import sub` in src/x/y.py → src/pkg/sub.py
        let imp = PathBuf::from("src/x/y.py");
        match resolve(&files(), &imp, 2, &["pkg".into()], &["sub".into()]) {
            Resolution::Internal(v) => {
                assert!(v.contains(&PathBuf::from("src/pkg/sub.py")))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn from_dot_import() {
        // `from . import api` → src/api.py
        match resolve(&files(), &imp(), 1, &[], &["api".into()]) {
            Resolution::Internal(v) => assert!(v.contains(&PathBuf::from("src/api.py"))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn absolute_suffix_and_external() {
        match resolve(&files(), &imp(), 0, &["models".into()], &["Task".into()]) {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/models.py")]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve(&files(), &imp(), 0, &["sqlite3".into()], &[]),
            Resolution::External
        ));
        // ambiguous suffix → Unresolved
        let mut fs = files();
        fs.insert(PathBuf::from("other/models.py"));
        assert!(matches!(
            resolve(&fs, &imp(), 0, &["models".into()], &["x".into()]),
            Resolution::Unresolved
        ));
    }
}
