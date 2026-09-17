//! Filesystem walk: exclusion rules, git-tracked filter, importance scoring.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use glob::Pattern;
use walkdir::WalkDir;

use crate::config::ScanConfig;
use crate::error::{Error, Result};

/// One scannable file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Repo-relative path.
    pub rel_path: PathBuf,
    /// Absolute path.
    pub abs_path: PathBuf,
    /// File name.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase extension, if any.
    pub extension: Option<String>,
    /// Heuristic 0.0–1.0 importance.
    pub importance_score: f64,
}

/// Binary-ish extensions never worth sending to an LLM.
const BINARY_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "ico", "svg", "mp3", "mp4", "avi", "mov", "pdf", "zip",
    "tar", "gz", "zst", "bz2", "xz", "7z", "exe", "dll", "so", "dylib", "a", "o", "obj", "class",
    "jar", "wasm", "woff", "woff2", "ttf", "otf", "eot", "pyc", "pyo", "db", "sqlite", "bin",
    "dat", "pack", "idx",
];

/// Directories always skipped regardless of config (our own state + VCS).
const ALWAYS_EXCLUDED_DIRS: &[&str] = &[".agentwiki", ".git", ".hg", ".svn"];

/// Collect scan-eligible files under `root`.
pub fn scan_files(root: &Path, cfg: &ScanConfig, output_path: &Path) -> Result<Vec<FileEntry>> {
    let tracked = if cfg.git_tracked_only {
        git_tracked_files(root)
    } else {
        None
    };
    if cfg.git_tracked_only && tracked.as_ref().is_some_and(|t| t.is_empty()) {
        tracing::warn!(
            "git_tracked_only is on but `git ls-files` returned nothing; scanning all files"
        );
    }

    let excluded_file_patterns: Vec<Pattern> = cfg
        .excluded_files
        .iter()
        .filter_map(|p| Pattern::new(&p.to_lowercase()).ok())
        .collect();

    let out_abs = output_path
        .canonicalize()
        .unwrap_or_else(|_| output_path.to_path_buf());

    let mut files = Vec::new();
    let walker = WalkDir::new(root)
        .max_depth(cfg.max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                !is_excluded_dir(e.file_name().to_string_lossy().as_ref(), cfg)
            } else {
                true
            }
        });

    for entry in walker {
        let entry = entry
            .map_err(|e| Error::io(root.to_path_buf(), std::io::Error::other(e.to_string())))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        // Never ingest our own output directory.
        if abs.canonicalize().is_ok_and(|c| c.starts_with(&out_abs)) {
            continue;
        }
        let rel = match abs.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let lower_name = name.to_lowercase();

        if !cfg.include_hidden && lower_name.starts_with('.') {
            continue;
        }
        if excluded_file_patterns
            .iter()
            .any(|p| p.matches(&lower_name))
        {
            continue;
        }
        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        if let Some(e) = &ext
            && BINARY_EXTENSIONS.contains(&e.as_str())
        {
            continue;
        }
        if let Some(tracked) = &tracked
            && !tracked.is_empty()
            && !tracked.contains(&rel)
        {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > cfg.max_file_size {
            continue;
        }
        if is_binary_by_content(&abs) {
            continue;
        }

        let mut fe = FileEntry {
            rel_path: rel,
            abs_path: abs,
            name,
            size,
            extension: ext,
            importance_score: 0.0,
        };
        fe.importance_score = importance(&fe);
        files.push(fe);
    }

    files.sort_by(|a, b| {
        b.importance_score
            .partial_cmp(&a.importance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    Ok(files)
}

fn is_excluded_dir(name: &str, cfg: &ScanConfig) -> bool {
    let lower = name.to_lowercase();
    if ALWAYS_EXCLUDED_DIRS.contains(&lower.as_str()) {
        return true;
    }
    if !cfg.include_hidden && name.starts_with('.') {
        return true;
    }
    cfg.excluded_dirs
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
}

/// `git ls-files` → repo-relative paths. `None` when git is unavailable or
/// the project is not a repo.
fn git_tracked_files(root: &Path) -> Option<HashSet<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// Sniff first bytes for NUL — cheap binary detection for extensionless files.
fn is_binary_by_content(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 4096];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    buf[..n].contains(&0)
}

/// Heuristic file importance, ported from deepwiki-rs.
fn importance(file: &FileEntry) -> f64 {
    let mut score: f64 = 0.0;
    let path_str = file.rel_path.to_string_lossy().to_lowercase();

    if path_str.contains("cmd") || path_str.contains("internal") || path_str.contains("pkg") {
        score += 0.3;
    }
    if path_str.contains("main") || path_str.contains("index") {
        score += 0.15;
    }
    if path_str.contains("config") || path_str.contains("setup") {
        score += 0.1;
    }
    if file.size > 1024 && file.size < 50 * 1024 {
        score += 0.15;
    }
    if let Some(ext) = &file.extension {
        match ext.as_str() {
            "rs" | "py" | "java" | "kt" | "cpp" | "c" | "go" | "rb" | "php" | "m" | "swift"
            | "dart" | "cs" => score += 0.4,
            "sql" | "sqlproj" => score += 0.3,
            "jsx" | "tsx" | "vue" | "svelte" => score += 0.2,
            "js" | "ts" | "mjs" | "cjs" => score += 0.15,
            "csproj" | "sln" => score += 0.2,
            "gradle" | "pom" => score += 0.15,
            "toml" | "yaml" | "yml" | "json" | "xml" | "ini" => score += 0.1,
            "css" | "scss" | "sass" | "less" | "html" | "htm" => score += 0.05,
            _ => {}
        }
    }
    if path_str.contains("database")
        || path_str.contains("schema")
        || path_str.contains("migrations")
    {
        score += 0.15;
    }
    score.min(1.0)
}
