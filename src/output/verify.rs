//! Post-write verification: file integrity + mermaid syntax checks.
//!
//! When `mermaid-fixer` is installed we run `-d <out> --dry-run` for a real
//! syntax pass; a built-in heuristic check always runs regardless.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::pipeline::PipelineCtx;

/// Result of the verify stage.
#[derive(Debug, Default, Serialize)]
pub struct VerifyReport {
    /// Expected docs missing from disk.
    pub missing: Vec<String>,
    /// Docs that exist but are empty.
    pub empty: Vec<String>,
    /// Total mermaid blocks found.
    pub mermaid_blocks: usize,
    /// Per-file heuristic syntax issues.
    pub mermaid_issues: Vec<String>,
    /// Whether `mermaid-fixer` ran.
    pub fixer_available: bool,
    /// Truncated mermaid-fixer stdout.
    pub fixer_output: String,
}

/// Expected top-level docs (`4.Deep-Exploration/` checked separately).
const EXPECTED: &[&str] = &[
    "1.Overview.md",
    "2.Architecture.md",
    "3.Workflow.md",
    "5.Boundary-Interfaces.md",
    "6.Database-Overview.md",
];

/// Valid mermaid diagram headers.
const MERMAID_HEADERS: &[&str] = &[
    "graph", "flowchart", "sequencediagram", "classdiagram", "statediagram",
    "statediagram-v2", "erdiagram", "journey", "gantt", "pie", "mindmap",
    "timeline", "gitgraph", "c4context", "c4container", "c4component",
    "sankey-beta", "xychart-beta", "block-beta", "packet-beta",
];

/// Verify the written doc tree. Never fails the pipeline — issues are
/// collected into the report and logged.
pub async fn verify(pctx: &PipelineCtx) -> Result<VerifyReport, crate::error::Error> {
    let out = &pctx.config.output_path;
    let mut report = VerifyReport::default();

    // Integrity: expected files present and non-empty.
    for rel in EXPECTED {
        let path = out.join(rel);
        match std::fs::metadata(&path) {
            Err(_) => report.missing.push(rel.to_string()),
            Ok(m) if m.len() == 0 => report.empty.push(rel.to_string()),
            _ => {}
        }
    }
    if !out.join("4.Deep-Exploration").is_dir() {
        report.missing.push("4.Deep-Exploration/".to_string());
    }

    // Heuristic mermaid check over every .md file.
    for path in md_files(out) {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let issues = check_mermaid(&text);
        report.mermaid_blocks += issues.blocks;
        for i in issues.problems {
            report
                .mermaid_issues
                .push(format!("{}: {i}", path.display()));
        }
    }

    // Real syntax pass when the fixer is available.
    if pctx.config.verify.mermaid_fixer
        && let Some(out_str) = run_mermaid_fixer(out)
    {
        report.fixer_available = true;
        report.fixer_output = crate::backend::tail(&out_str, 2000);
    }

    for m in &report.missing {
        tracing::warn!("missing expected doc: {m}");
    }
    for i in &report.mermaid_issues {
        tracing::warn!("mermaid issue: {i}");
    }
    Ok(report)
}

/// All `*.md` files under `dir` (recursive).
fn md_files(dir: &Path) -> Vec<std::path::PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect()
}

struct MermaidCheck {
    blocks: usize,
    problems: Vec<String>,
}

/// Cheap mermaid sanity: valid diagram header + non-empty body.
fn check_mermaid(text: &str) -> MermaidCheck {
    let mut out = MermaidCheck {
        blocks: 0,
        problems: Vec::new(),
    };
    let mut in_block = false;
    let mut block_start = 0usize;
    let mut block_body = String::new();
    for (lineno, line) in text.lines().enumerate() {
        let l = line.trim();
        if !in_block && l.starts_with("```mermaid") {
            in_block = true;
            block_start = lineno + 1;
            block_body.clear();
            continue;
        }
        if in_block {
            if l.starts_with("```") {
                in_block = false;
                out.blocks += 1;
                let header = block_body
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let word = header.split_whitespace().next().unwrap_or("");
                if !MERMAID_HEADERS.contains(&word) {
                    out.problems.push(format!(
                        "line {block_start}: unknown mermaid header `{header}`"
                    ));
                } else if block_body.lines().count() < 2 {
                    out.problems.push(format!("line {block_start}: empty diagram"));
                }
                continue;
            }
            block_body.push_str(line);
            block_body.push('\n');
        }
    }
    if in_block {
        out.problems
            .push(format!("line {block_start}: unterminated mermaid block"));
    }
    out
}

/// `mermaid-fixer -d <dir> --dry-run` → stdout, `None` if unavailable.
fn run_mermaid_fixer(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("mermaid-fixer")
        .arg("-d")
        .arg(dir)
        .arg("--dry-run")
        .output()
        .ok()?;
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = write!(s, "{}", String::from_utf8_lossy(&out.stderr));
    Some(s)
}
