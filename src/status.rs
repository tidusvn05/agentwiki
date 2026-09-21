//! `agentwiki status` — read-only freshness report: what changed since
//! the last run (manifest diff + cosmetic/structural classification),
//! how old the research and each generated doc are, and which docs the
//! next run would regenerate. Exit `0`; `2` when the project can't be
//! scanned at all.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{CliOverrides, Config};
use crate::manifest::{self, Manifest, Significance};

/// Run the status report. Returns the process exit code.
pub async fn run(
    project_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    json: bool,
) -> i32 {
    let ov = CliOverrides {
        project_path: project_path.clone(),
        output_path: output_path.clone(),
        ..Default::default()
    };
    let cfg = match Config::load(&ov, config_path.as_deref()) {
        Ok(c) => c,
        Err(_) => {
            let mut c = Config::default();
            if let Some(p) = &project_path {
                c.internal_path = p.join(&c.internal_path);
                c.project_path = p.clone();
            }
            if let Some(o) = &output_path {
                c.output_path = o.clone();
            }
            c
        }
    };

    let scan = match crate::scanner::scan(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: scan failed: {e}");
            return 2;
        }
    };
    let cur = Manifest::build(&scan, &cfg);
    let manifest_file = manifest::manifest_path(&cfg);
    let prev = Manifest::load(&manifest_file);
    let report = build_report(&cfg, &cur, prev.as_ref(), &manifest_file);

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => tracing::warn!("failed to serialize status: {e}"),
        }
    } else {
        print!("{}", render(&report));
    }
    0
}

/// One output doc's freshness.
#[derive(Debug, Serialize)]
struct DocStatus {
    /// Path relative to the output dir.
    path: String,
    /// Seconds since last modification (absent when the file is missing).
    age_secs: Option<u64>,
    /// `stale` when inputs that feed this doc changed; `fresh` otherwise;
    /// `missing` when absent.
    state: &'static str,
}

/// The serializable report.
#[derive(Debug, Serialize)]
struct StatusReport {
    /// Project root scanned.
    project: String,
    /// Manifest slot this report compares against.
    manifest: String,
    /// RFC3339 time the manifest was written (absent when none).
    manifest_written_at: Option<String>,
    /// Recorded `git rev-parse HEAD` (absent without a manifest or git).
    manifest_git_head: Option<String>,
    /// Commits between the recorded HEAD and current HEAD.
    commits_since: Option<usize>,
    /// `research.json` age in seconds (absent when missing).
    research_age_secs: Option<u64>,
    /// `cosmetic` | `structural` | `no-baseline`.
    classification: &'static str,
    /// Why structural (empty otherwise).
    reasons: Vec<String>,
    /// Files added/removed/content-changed since the manifest.
    files: FileCounts,
    /// Dirs added/removed.
    dirs: DirCounts,
    /// Import edges added/removed.
    edges: EdgeCounts,
    /// Dirs holding cosmetic-only edits not yet reflected in the docs.
    pending_cosmetic_dirs: Vec<String>,
    /// Per-doc freshness.
    docs: Vec<DocStatus>,
}

#[derive(Debug, Serialize)]
struct FileCounts {
    added: usize,
    removed: usize,
    changed: usize,
}

#[derive(Debug, Serialize)]
struct DirCounts {
    added: usize,
    removed: usize,
}

#[derive(Debug, Serialize)]
struct EdgeCounts {
    added: usize,
    removed: usize,
}

fn build_report(
    cfg: &Config,
    cur: &Manifest,
    prev: Option<&Manifest>,
    manifest_file: &Path,
) -> StatusReport {
    let diff = prev.map(|p| p.diff(cur));
    let sig = diff.as_ref().map(manifest::classify);
    let classification = match (&prev, &sig) {
        (Some(_), Some(Significance::Cosmetic)) => "cosmetic",
        (Some(_), Some(Significance::Structural(_))) => "structural",
        _ => "no-baseline",
    };
    let reasons = match &sig {
        Some(Significance::Structural(r)) => r.clone(),
        _ => Vec::new(),
    };

    let structural = classification == "structural";
    let changed_paths: Vec<String> = diff
        .as_ref()
        .map(|d| {
            d.added_files
                .iter()
                .chain(&d.removed_files)
                .chain(&d.changed_files)
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    StatusReport {
        project: cfg.project_path.display().to_string(),
        manifest: manifest_file.display().to_string(),
        manifest_written_at: prev.map(|p| p.created_at.clone()),
        manifest_git_head: prev.and_then(|p| p.git_head.clone()),
        commits_since: prev
            .and_then(|p| p.git_head.as_ref())
            .and_then(|h| manifest::commits_since(&cfg.project_path, h)),
        research_age_secs: file_age(&cfg.internal_path.join("research.json")),
        classification,
        reasons,
        files: FileCounts {
            added: diff.as_ref().map(|d| d.added_files.len()).unwrap_or(0),
            removed: diff.as_ref().map(|d| d.removed_files.len()).unwrap_or(0),
            changed: diff.as_ref().map(|d| d.changed_files.len()).unwrap_or(0),
        },
        dirs: DirCounts {
            added: diff.as_ref().map(|d| d.added_dirs.len()).unwrap_or(0),
            removed: diff.as_ref().map(|d| d.removed_dirs.len()).unwrap_or(0),
        },
        edges: EdgeCounts {
            added: diff.as_ref().map(|d| d.added_edges.len()).unwrap_or(0),
            removed: diff.as_ref().map(|d| d.removed_edges.len()).unwrap_or(0),
        },
        pending_cosmetic_dirs: diff
            .as_ref()
            .map(|d| d.cosmetic_dirs().into_iter().collect())
            .unwrap_or_default(),
        docs: doc_statuses(cfg, structural, &changed_paths),
    }
}

/// Seconds since `path`'s mtime; `None` when missing/unreadable.
fn file_age(path: &Path) -> Option<u64> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    mtime.elapsed().ok().map(|d| d.as_secs())
}

/// Per-doc freshness: global docs go stale on any input delta; a
/// deep-dive doc goes stale when its domain's `code_paths` (read back
/// from `research.json`) intersect the changed paths — or whenever the
/// classification is structural, since the domain map itself may shift.
fn doc_statuses(cfg: &Config, structural: bool, changed_paths: &[String]) -> Vec<DocStatus> {
    let out = &cfg.output_path;
    let any_change = structural || !changed_paths.is_empty();
    let mut docs: Vec<DocStatus> = crate::output::writer::DOCS
        .iter()
        .map(|(_, rel)| DocStatus {
            path: rel.to_string(),
            age_secs: file_age(&out.join(rel)),
            state: doc_state(&out.join(rel), any_change),
        })
        .collect();

    // Deep-dives: one doc per domain in research.json.
    let domains = domain_paths(cfg);
    let dd = out.join(crate::output::writer::DEEP_DIVE_DIR);
    let mut seen: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dd) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "md")
                && let Some(name) = p.file_name().and_then(|n| n.to_str())
            {
                seen.push(name.to_string());
            }
        }
    }
    seen.sort();
    for name in seen {
        // Map file name back to a domain via sanitized filename — the
        // same transform the writer applies.
        let domain = domains.iter().find(|(d, _)| {
            crate::output::writer::sanitize_filename(d) == name.trim_end_matches(".md")
        });
        let touched = match domain {
            Some((_, paths)) => paths.iter().any(|cp| {
                changed_paths
                    .iter()
                    .any(|f| f.contains(cp.as_str()) || cp.contains(f.as_str()))
            }),
            None => false, // doc without a matching domain — leftover
        };
        let path = format!("{}/{name}", crate::output::writer::DEEP_DIVE_DIR);
        let state = if structural || touched {
            doc_state(&out.join(&path), true)
        } else {
            doc_state(&out.join(&path), false)
        };
        docs.push(DocStatus {
            age_secs: file_age(&out.join(&path)),
            path,
            state,
        });
    }
    docs
}

fn doc_state(path: &Path, stale: bool) -> &'static str {
    if !path.is_file() {
        "missing"
    } else if stale {
        "stale"
    } else {
        "fresh"
    }
}

/// `domain name → code_paths` from the saved research context.
fn domain_paths(cfg: &Config) -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(cfg.internal_path.join("research.json")).ok();
    let v: serde_json::Value = text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let report: Option<crate::agent::reports::DomainModulesReport> =
        serde_json::from_value(v.get("domain_modules").cloned().unwrap_or_default()).ok();
    report
        .map(|r| {
            r.domain_modules
                .into_iter()
                .map(|d| (d.name, d.code_paths))
                .collect()
        })
        .unwrap_or_default()
}

fn render(r: &StatusReport) -> String {
    use std::fmt::Write as _;
    let mut s = format!("agentwiki status — {}\n\n", r.project);
    match &r.manifest_written_at {
        Some(ts) => {
            let _ = writeln!(s, "manifest: {} ({ts})", r.manifest);
        }
        None => {
            let _ = writeln!(
                s,
                "manifest: none — no tracked baseline; the next run is a full run\n  \
                 (baselines are recorded by `--incremental` and agentic runs)"
            );
        }
    }
    match (&r.manifest_git_head, r.commits_since) {
        (Some(h), Some(n)) => {
            let _ = writeln!(s, "git: documented at {h}, {n} commit(s) since");
        }
        (Some(h), None) => {
            let _ = writeln!(s, "git: documented at {h}");
        }
        _ => {}
    }
    match r.research_age_secs {
        Some(a) => {
            let _ = writeln!(s, "research.json: {}", human_age(a));
        }
        None => {
            let _ = writeln!(s, "research.json: missing");
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "classification: {}", r.classification);
    for reason in &r.reasons {
        let _ = writeln!(s, "  - {reason}");
    }
    if r.manifest_written_at.is_some() {
        let _ = writeln!(
            s,
            "files: +{} −{} ~{} · dirs: +{} −{} · import edges: +{} −{}",
            r.files.added,
            r.files.removed,
            r.files.changed,
            r.dirs.added,
            r.dirs.removed,
            r.edges.added,
            r.edges.removed,
        );
        if !r.pending_cosmetic_dirs.is_empty() {
            let _ = writeln!(
                s,
                "cosmetic-only edits pending in: {}",
                r.pending_cosmetic_dirs.join(", ")
            );
        }
    }
    match r.classification {
        "cosmetic" => {
            let _ = writeln!(
                s,
                "→ `agentwiki --incremental` would no-op (0 calls); a plain run rewrites from cache"
            );
        }
        "structural" => {
            let _ = writeln!(s, "→ the next run regenerates research");
        }
        _ => {}
    }
    let _ = writeln!(s);

    if !r.docs.is_empty() {
        let _ = writeln!(s, "docs:");
        for d in &r.docs {
            let age = d.age_secs.map(human_age).unwrap_or_else(|| "-".to_string());
            let _ = writeln!(s, "  {} — {} ({age})", d.path, d.state);
        }
    }
    s
}

fn human_age(secs: u64) -> String {
    if secs < 120 {
        format!("{secs}s ago")
    } else if secs < 7200 {
        format!("{}m ago", secs / 60)
    } else if secs < 172_800 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
