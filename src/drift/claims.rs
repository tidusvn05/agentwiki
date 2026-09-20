//! Claim loading, claim export, and claim-endpoint normalization.
//!
//! "Claims" are the `relationships.core_dependencies` edges the research
//! phase emits. `.agentwiki/` is gitignored, so CI needs a committed copy:
//! the pipeline writes `{relationships: …}` to
//! `<output>/agentwiki.claims.json` next to the docs, and `--export-claims`
//! does the same to a caller-chosen file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::agent::reports::CoreDependency;
use crate::error::{Error, Result};

/// File name of the claims copy the pipeline writes next to the docs —
/// committed with the docs so `drift` works on a bare checkout.
pub const CLAIMS_FILENAME: &str = "agentwiki.claims.json";

/// How a claim endpoint resolved against the scanned tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Empty after normalization.
    Empty,
    /// A scanned file (repo-relative path).
    File(PathBuf),
    /// A directory (exists on disk or implied by scanned files).
    Dir(PathBuf),
    /// Nothing matches — no fuzzy fallback by name.
    Unknown,
}

/// Read claims from `path`. Accepts a full `research.json` map, a trimmed
/// `{relationships: …}` export, or a bare `{core_dependencies: […]}`.
///
/// Sync + strict: `ResearchContext::get_typed` swallows parse errors, which
/// would silently turn a corrupt claims file into "no claims".
pub fn load_claims(path: &Path) -> Result<Vec<CoreDependency>> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::Parse {
        agent: format!("claims {}", path.display()),
        message: e.to_string(),
    })?;
    let deps = value
        .get("relationships")
        .and_then(|r| r.get("core_dependencies"))
        .or_else(|| value.get("core_dependencies"))
        .ok_or_else(|| Error::Parse {
            agent: format!("claims {}", path.display()),
            message: "no relationships.core_dependencies array".to_string(),
        })?;
    serde_json::from_value(deps.clone()).map_err(|e| Error::Parse {
        agent: format!("claims {}", path.display()),
        message: e.to_string(),
    })
}

/// Write `{relationships: …}` from a full `research.json` to `dest`
/// (commit-able, unlike `.agentwiki/`). Returns the edge count.
pub fn export_claims(research_path: &Path, dest: &Path) -> Result<usize> {
    let text = std::fs::read_to_string(research_path).map_err(|e| Error::io(research_path, e))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::Parse {
        agent: format!("{}", research_path.display()),
        message: e.to_string(),
    })?;
    let rel = value
        .get("relationships")
        .cloned()
        .ok_or_else(|| Error::Parse {
            agent: format!("{}", research_path.display()),
            message: "no relationships key".to_string(),
        })?;
    let n = rel
        .get("core_dependencies")
        .and_then(|d| d.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let body = serde_json::to_string_pretty(&serde_json::json!({ "relationships": rel }))
        .map_err(|e| Error::Pipeline(format!("claims export serialize: {e}")))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    crate::util::write_atomic(dest, body.as_bytes())?;
    Ok(n)
}

/// Normalize a raw claim endpoint (`from`/`to`) into a tree position.
///
/// Steps: trim, strip backticks, `\`→`/`, drop leading `./` and trailing
/// `/`, drop a free-text `( … )` suffix; then exact file → exact dir →
/// unique path-suffix match → on-disk dir check → `Unknown`.
pub fn normalize_endpoint(
    raw: &str,
    files: &BTreeSet<PathBuf>,
    dirs: &BTreeSet<PathBuf>,
    root: &Path,
) -> Endpoint {
    let mut s = raw.trim().trim_matches('`').trim().replace('\\', "/");
    while let Some(t) = s.strip_prefix("./") {
        s = t.to_string();
    }
    while s.ends_with('/') {
        s.pop();
    }
    // Free-text suffixes like "src (config.rs, cli.rs…)".
    if let Some(i) = s.find(" (") {
        s.truncate(i);
    }
    let s = s.trim();
    if s.is_empty() || s == "." {
        return Endpoint::Empty;
    }
    let p = PathBuf::from(s);
    if files.contains(&p) {
        return Endpoint::File(p);
    }
    if dirs.contains(&p) {
        return Endpoint::Dir(p);
    }
    // Unique path-suffix match: `s` is a component-aligned tail of exactly
    // one scanned file or known dir.
    let suffix = format!("/{s}");
    let mut matches = files.iter().chain(dirs.iter()).filter(|c| {
        c.as_os_str()
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(&suffix)
    });
    if let (Some(m), None) = (matches.next(), matches.next()) {
        return if files.contains(m) {
            Endpoint::File(m.clone())
        } else {
            Endpoint::Dir(m.clone())
        };
    }
    // Dirs that exist on disk but produced zero scanned files still
    // resolve — they land in `non_code_endpoint`, not `unknown_endpoint`.
    let abs = root.join(s);
    if abs.is_dir() {
        return Endpoint::Dir(p);
    }
    if abs.is_file() {
        return Endpoint::File(p);
    }
    Endpoint::Unknown
}

/// `/`-separated display form of a normalized path.
pub fn display_path(p: &Path) -> String {
    p.as_os_str().to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
        let files: BTreeSet<PathBuf> = [
            "src/main.rs",
            "src/agent/mod.rs",
            "src/agent/runner.rs",
            "README.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let dirs: BTreeSet<PathBuf> = ["src", "src/agent"].iter().map(PathBuf::from).collect();
        (files, dirs)
    }

    #[test]
    fn normalize_exact_and_suffix() {
        let (files, dirs) = idx();
        let root = Path::new("/nonexistent-root-xyz");
        assert_eq!(
            normalize_endpoint("src/main.rs", &files, &dirs, root),
            Endpoint::File(PathBuf::from("src/main.rs"))
        );
        assert_eq!(
            normalize_endpoint("`src/agent/`", &files, &dirs, root),
            Endpoint::Dir(PathBuf::from("src/agent"))
        );
        assert_eq!(
            normalize_endpoint("./agent", &files, &dirs, root),
            Endpoint::Dir(PathBuf::from("src/agent"))
        );
        assert_eq!(
            normalize_endpoint("src (config.rs, cli.rs)", &files, &dirs, root),
            Endpoint::Dir(PathBuf::from("src"))
        );
        assert_eq!(
            normalize_endpoint("nowhere/at/all", &files, &dirs, root),
            Endpoint::Unknown
        );
        assert_eq!(
            normalize_endpoint("   ", &files, &dirs, root),
            Endpoint::Empty
        );
    }

    #[test]
    fn load_claims_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let full = dir.path().join("research.json");
        std::fs::write(
            &full,
            r#"{"relationships":{"core_dependencies":[{"from":"a","to":"b","dependency_type":"import","importance":3}]},"other":{}}"#,
        )
        .unwrap();
        let claims = load_claims(&full).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].from, "a");

        let bare = dir.path().join("claims.json");
        std::fs::write(&bare, r#"{"core_dependencies":[{"from":"x","to":"y"}]}"#).unwrap();
        assert_eq!(load_claims(&bare).unwrap()[0].to, "y");

        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, r#"{"nope":1}"#).unwrap();
        assert!(load_claims(&bad).is_err());
    }
}
