//! Doc-tree writer: ctx → `1.Overview.md` … `4.Deep-Exploration/*`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::pipeline::PipelineCtx;

/// `(ctx key, output path relative to output dir)`. Also read by
/// `agentwiki status` to report per-doc freshness.
pub const DOCS: &[(&str, &str)] = &[
    ("overview", "1.Overview.md"),
    ("architecture_doc", "2.Architecture.md"),
    ("workflow_doc", "3.Workflow.md"),
    ("boundary_doc", "5.Boundary-Interfaces.md"),
    ("database_doc", "6.Database-Overview.md"),
];

/// Deep-dive subdirectory — one `<sanitized domain>.md` per domain.
pub const DEEP_DIVE_DIR: &str = "4.Deep-Exploration";

/// `<internal>/written-docs-<sha12>.json` — the doc set `write_docs`
/// produced for this output dir (paths relative to it). Written *last*,
/// so its existence and mtime prove the write completed — and completed
/// after the `research.json` it renders (`docs_reusable` relies on that
/// ordering to detect a run interrupted mid-compose).
pub fn written_docs_path(config: &Config) -> PathBuf {
    config.internal_path.join(format!(
        "written-docs-{}.json",
        crate::manifest::output_key(config)
    ))
}

/// The doc set recorded by the last completed `write_docs`; `None` on
/// missing or corrupt — callers treat both as "unknown" and fail open
/// toward rewriting.
pub fn load_written_docs(config: &Config) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(written_docs_path(config)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_written_docs(config: &Config, rels: &[String]) -> Result<()> {
    let path = written_docs_path(config);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let body = serde_json::to_string_pretty(rels)
        .map_err(|e| Error::Pipeline(format!("written-docs serialize: {e}")))?;
    crate::util::write_atomic(&path, body.as_bytes())
}

/// Write all compose-phase results to the output directory.
pub async fn write_docs(pctx: &PipelineCtx) -> Result<Vec<PathBuf>> {
    let out_dir = &pctx.config.output_path;
    std::fs::create_dir_all(out_dir).map_err(|e| Error::io(out_dir, e))?;
    // The previous written list bounds stale-doc cleanup — read it
    // before anything is written so a failed run leaves it intact.
    let prev = load_written_docs(&pctx.config).unwrap_or_default();
    let mut written = Vec::new();
    let mut rels: Vec<String> = Vec::new();

    for (key, rel) in DOCS {
        match pctx.ctx.get(key).await {
            Some(Value::String(md)) => {
                written.push(write_file(out_dir, rel, &md)?);
                rels.push(rel.to_string());
            }
            Some(other) => {
                written.push(write_file(
                    out_dir,
                    rel,
                    &serde_json::to_string_pretty(&other).unwrap_or_default(),
                )?);
                rels.push(rel.to_string());
            }
            None => {
                tracing::warn!(key, "no document produced, skipping");
            }
        }
    }

    // Per-domain deep dives → `4.Deep-Exploration/<Domain>.md`.
    if let Some(Value::Object(map)) = pctx.ctx.get("deep_dive").await {
        for (domain, v) in &map {
            let md = match v {
                Value::String(t) => t.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let rel = format!("{DEEP_DIVE_DIR}/{}.md", sanitize_filename(domain));
            written.push(write_file(out_dir, &rel, &md)?);
            rels.push(rel);
        }
    }

    // The domain set can shrink between runs — drop deep-dives we wrote
    // previously but didn't write this run. Deletion is bounded by our
    // own written list, never by "any .md we don't recognize", so docs
    // the user added to the directory by hand are left alone.
    let dd_prefix = format!("{DEEP_DIVE_DIR}/");
    let cur: BTreeSet<&str> = rels.iter().map(String::as_str).collect();
    for rel in &prev {
        if rel.starts_with(&dd_prefix) && !cur.contains(rel.as_str()) {
            match std::fs::remove_file(out_dir.join(rel)) {
                Ok(()) => tracing::info!(path = rel, "removed stale doc"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(path = rel, "failed to remove stale doc: {e}"),
            }
        }
    }

    // Recorded last: a present, fresh written-docs file is the proof a
    // run finished writing — the incremental gate refuses to no-op
    // without it.
    if let Err(e) = save_written_docs(&pctx.config, &rels) {
        tracing::warn!("failed to record written doc list: {e}");
    }

    tracing::info!(files = written.len(), dir = %out_dir.display(), "docs written");
    Ok(written)
}

fn write_file(out_dir: &Path, rel: &str, content: &str) -> Result<PathBuf> {
    let path = out_dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    crate::util::write_atomic(&path, content.as_bytes())?;
    Ok(path)
}

/// Domain name → safe file name (spaces and slashes can't stay).
/// Shared with `status`, which maps file names back to domains.
pub(crate) fn sanitize_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect();
    s.trim().to_string()
}
