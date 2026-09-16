//! Project structure: directory grouping and tree formatting for prompts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::files::FileEntry;
use crate::error::Result;

/// A directory that becomes a `dir_summary` fan-out target.
#[derive(Debug, Clone)]
pub struct DirectoryInfo {
    /// Absolute path.
    pub path: PathBuf,
    /// Repo-relative path (`.` for root).
    pub rel_path: PathBuf,
    /// Directory name.
    pub name: String,
    /// Files directly inside (scan order).
    pub files: Vec<FileEntry>,
    /// Direct subdirectory count.
    pub subdirectory_count: usize,
}

/// Full deterministic scan result (phase 0 output, no AI).
#[derive(Debug)]
pub struct ScanData {
    /// Project directory name.
    pub project_name: String,
    /// Absolute project root.
    pub root: PathBuf,
    /// All included files (importance-sorted).
    pub files: Vec<FileEntry>,
    /// Directories containing ≥1 included file.
    pub directories: Vec<DirectoryInfo>,
    /// extension → file count.
    pub file_types: std::collections::HashMap<String, usize>,
    /// README content if found.
    pub readme: Option<String>,
}

/// Group scanned files into per-directory buckets.
pub fn build_structure(root: &Path, files: Vec<FileEntry>) -> ScanData {
    let mut dirs: BTreeMap<PathBuf, Vec<FileEntry>> = BTreeMap::new();
    for f in &files {
        let dir = f.rel_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        dirs.entry(dir).or_default().push(f.clone());
    }

    // Direct subdirectory counts: for each dir, count distinct immediate children.
    let all_dir_paths: Vec<PathBuf> = dirs.keys().cloned().collect();
    let mut child_counts: std::collections::HashMap<PathBuf, usize> = Default::default();
    for d in &all_dir_paths {
        if let Some(parent) = d.parent() {
            *child_counts.entry(parent.to_path_buf()).or_insert(0) += 1;
        }
    }
    // Dirs on disk that contain no files are invisible to us; that's fine —
    // dir_summary only needs dirs with content.

    let mut directories: Vec<DirectoryInfo> = dirs
        .into_iter()
        .map(|(rel, files)| {
            let abs = if rel.as_os_str().is_empty() {
                root.to_path_buf()
            } else {
                root.join(&rel)
            };
            let name = if rel.as_os_str().is_empty() {
                root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string())
            } else {
                rel.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel.to_string_lossy().to_string())
            };
            DirectoryInfo {
                path: abs,
                subdirectory_count: child_counts.get(&rel).copied().unwrap_or(0),
                rel_path: rel,
                name,
                files,
            }
        })
        .collect();
    directories.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    let mut file_types = std::collections::HashMap::new();
    for f in &files {
        if let Some(e) = &f.extension {
            *file_types.entry(e.clone()).or_insert(0) += 1;
        }
    }

    ScanData {
        project_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string()),
        root: root.to_path_buf(),
        directories,
        files,
        file_types,
        readme: None,
    }
}

/// Load README content if a recognizable readme exists at the root.
pub fn extract_docs(root: &Path, files: &[FileEntry]) -> Option<String> {
    let readme = files.iter().find(|f| {
        f.rel_path.parent().is_none_or(|p| p.as_os_str().is_empty())
            && f.name.to_lowercase().starts_with("readme")
    });
    let entry = readme?;
    std::fs::read_to_string(&entry.abs_path)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string(root.join(&entry.rel_path)).ok()
        })
}

/// Render the project as a tree (files included) for prompts.
pub fn format_as_tree(scan: &ScanData) -> String {
    let mut tree = PathNode::default();
    for f in &scan.files {
        tree.insert_file(&f.rel_path);
    }
    format!(
        "### Project Structure Information\nProject Name: {}\nTotal files: {}\n\nProject Directory Structure:\n```\n{}\n```\n",
        scan.project_name,
        scan.files.len(),
        tree.render()
    )
}

/// Directories-only tree (used when file count exceeds a limit).
pub fn format_as_directory_tree(scan: &ScanData) -> String {
    let mut tree = PathNode::default();
    for d in &scan.directories {
        if !d.rel_path.as_os_str().is_empty() {
            tree.insert_dir(&d.rel_path);
        }
    }
    format!(
        "### Project Directory Structure\nProject Name: {}\nTotal files: {}\n\nDirectory Tree:\n```\n{}\n```\n",
        scan.project_name,
        scan.files.len(),
        tree.render()
    )
}

#[derive(Default)]
struct PathNode {
    children: BTreeMap<String, PathNode>,
    is_file: bool,
}

impl PathNode {
    fn insert_file(&mut self, rel: &Path) {
        let mut node = self;
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        for (i, part) in parts.iter().enumerate() {
            node = node.children.entry(part.clone()).or_default();
            if i == parts.len() - 1 {
                node.is_file = true;
            }
        }
    }

    fn insert_dir(&mut self, rel: &Path) {
        let mut node = self;
        for part in rel.components() {
            node = node
                .children
                .entry(part.as_os_str().to_string_lossy().to_string())
                .or_default();
        }
    }

    fn render(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, "");
        s
    }

    fn write(&self, out: &mut String, prefix: &str) {
        let mut entries: Vec<_> = self.children.iter().collect();
        // Directories first, then files — matches conventional tree output.
        entries.sort_by_key(|(_, n)| n.is_file);
        for (i, (name, node)) in entries.iter().enumerate() {
            let last = i == entries.len() - 1;
            out.push_str(prefix);
            out.push_str(if last { "└── " } else { "├── " });
            out.push_str(name);
            if !node.is_file {
                out.push('/');
            }
            out.push('\n');
            let next = format!("{}{}", prefix, if last { "    " } else { "│   " });
            node.write(out, &next);
        }
    }
}

/// Read a file's content for prompt embedding, capped per file.
pub fn read_capped(path: &Path, max_chars: usize) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::error::Error::io(path.to_path_buf(), e))?;
    Ok(if content.chars().count() > max_chars {
        let mut s: String = content.chars().take(max_chars).collect();
        s.push_str("\n[truncated]");
        s
    } else {
        content
    })
}
