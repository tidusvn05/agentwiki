//! Doc-tree writer: ctx → `1.Overview.md` … `4.Deep-Exploration/*`.

use std::path::{Path, PathBuf};

use serde_json::Value;

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

/// Write all compose-phase results to the output directory.
pub async fn write_docs(pctx: &PipelineCtx) -> Result<Vec<PathBuf>> {
    let out_dir = &pctx.config.output_path;
    std::fs::create_dir_all(out_dir).map_err(|e| Error::io(out_dir, e))?;
    let mut written = Vec::new();

    for (key, rel) in DOCS {
        match pctx.ctx.get(key).await {
            Some(Value::String(md)) => written.push(write_file(out_dir, rel, &md)?),
            Some(other) => written.push(write_file(
                out_dir,
                rel,
                &serde_json::to_string_pretty(&other).unwrap_or_default(),
            )?),
            None => {
                tracing::warn!(key, "no document produced, skipping");
            }
        }
    }

    // Per-domain deep dives → `4.Deep-Exploration/<Domain>.md`.
    if let Some(Value::Object(map)) = pctx.ctx.get("deep_dive").await {
        let mut kept = std::collections::BTreeSet::new();
        for (domain, v) in &map {
            let md = match v {
                Value::String(t) => t.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let name = sanitize_filename(domain);
            kept.insert(format!("{name}.md"));
            written.push(write_file(
                out_dir,
                &format!("4.Deep-Exploration/{name}.md"),
                &md,
            )?);
        }
        // The domain set can shrink between runs — drop deep-dive files
        // for domains that no longer exist instead of leaving stale docs.
        let dd = out_dir.join("4.Deep-Exploration");
        if let Ok(rd) = std::fs::read_dir(&dd) {
            for e in rd.flatten() {
                let p = e.path();
                let stale = p.extension().is_some_and(|x| x == "md")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| !kept.contains(n));
                if stale && let Err(err) = std::fs::remove_file(&p) {
                    tracing::warn!("failed to remove stale doc {}: {err}", p.display());
                }
            }
        }
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
fn sanitize_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect();
    s.trim().to_string()
}
