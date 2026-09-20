//! JS/TS resolution: relative specifiers try the exact path, common
//! extensions, then `/index.*`; `./x.js` ↔ `./x.ts`. Aliases like `@/` or
//! `~/` (tsconfig paths) → `Unresolved`; bare specifiers → `External`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::Resolution;

/// Extensions tried after the literal specifier.
const EXTS: &[&str] = &[
    "ts", "tsx", "d.ts", "js", "jsx", "mjs", "cjs", "json", "vue", "svelte",
];

/// Resolve a JS/TS specifier from `importer`.
pub fn resolve(files: &BTreeSet<PathBuf>, importer: &Path, spec: &str) -> Resolution {
    if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
        let dir = importer
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let base = normalize(&dir.join(spec));
        return resolve_path(files, &base);
    }
    if spec.starts_with("@/") || spec.starts_with("~/") {
        return Resolution::Unresolved; // tsconfig paths — not mapped yet
    }
    Resolution::External
}

/// Normalize `a/../b` and `a/./b` lexically.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Try: literal path → `+ext` → `.js↔.ts` → `/index.ext`.
fn resolve_path(files: &BTreeSet<PathBuf>, base: &Path) -> Resolution {
    if files.contains(base) {
        return Resolution::Internal(vec![base.to_path_buf()]);
    }
    // `./x.js` may really be `./x.ts`.
    let mut bases = vec![base.to_path_buf()];
    if let Some(s) = base.to_str()
        && let Some(stripped) = s.strip_suffix(".js")
    {
        bases.push(PathBuf::from(format!("{stripped}.ts")));
    }
    for b in &bases {
        for ext in EXTS {
            let cand = b.with_extension(ext);
            if files.contains(&cand) {
                return Resolution::Internal(vec![cand]);
            }
        }
        for ext in EXTS {
            let cand = b.join(format!("index.{ext}"));
            if files.contains(&cand) {
                return Resolution::Internal(vec![cand]);
            }
        }
    }
    Resolution::Unresolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> BTreeSet<PathBuf> {
        [
            "src/a.ts",
            "src/util.ts",
            "src/deep/x.ts",
            "src/lib/index.ts",
            "src/comp.jsx",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }

    fn imp() -> PathBuf {
        PathBuf::from("src/a.ts")
    }

    #[test]
    fn relative_ext_and_index() {
        match resolve(&files(), &imp(), "./util") {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/util.ts")]),
            other => panic!("{other:?}"),
        }
        match resolve(&files(), &imp(), "./lib") {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/lib/index.ts")]),
            other => panic!("{other:?}"),
        }
        match resolve(&files(), &PathBuf::from("src/deep/x.ts"), "../util") {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/util.ts")]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn js_maps_to_ts_and_external() {
        match resolve(&files(), &imp(), "./util.js") {
            Resolution::Internal(v) => assert_eq!(v, [PathBuf::from("src/util.ts")]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve(&files(), &imp(), "react"),
            Resolution::External
        ));
        assert!(matches!(
            resolve(&files(), &imp(), "@/alias"),
            Resolution::Unresolved
        ));
        assert!(matches!(
            resolve(&files(), &imp(), "./missing"),
            Resolution::Unresolved
        ));
    }
}
