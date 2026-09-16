//! Summary report: `__AgentWiki_Summary__.md` in the output dir +
//! `summary.json` in the internal dir.

use std::fmt::Write as _;
use std::time::Duration;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::pipeline::PipelineCtx;
use crate::quota::now_rfc3339;

use super::VerifyReport;

/// Machine-readable run summary (`<internal>/summary.json`).
#[derive(Debug, Serialize)]
pub struct SummaryJson {
    /// RFC3339 timestamp.
    pub generated_at: String,
    /// Project scanned.
    pub project: String,
    /// Output directory.
    pub output: String,
    /// Wall seconds for the whole pipeline.
    pub total_secs: f64,
    /// Real CLI calls made.
    pub cli_calls: u32,
    /// Cache hits.
    pub cache_hits: u32,
    /// Calls used today (quota).
    pub quota_used_today: u32,
    /// Per-spec timings `(name, secs)`.
    pub spec_timings: Vec<(String, f64)>,
    /// Verify report.
    pub verify: VerifyReport,
}

/// Write the human + machine summaries.
pub async fn write_summary(
    pctx: &PipelineCtx,
    verify: &VerifyReport,
    total: Duration,
) -> Result<()> {
    let stats = pctx.stats.lock().await;
    let quota_used = pctx.quota.today_count().await;

    let json = SummaryJson {
        generated_at: now_rfc3339(),
        project: pctx.scan.root.display().to_string(),
        output: pctx.config.output_path.display().to_string(),
        total_secs: total.as_secs_f64(),
        cli_calls: stats.cli_calls,
        cache_hits: stats.cache_hits,
        quota_used_today: quota_used,
        spec_timings: stats.timings.clone(),
        verify: VerifyReport {
            missing: verify.missing.clone(),
            empty: verify.empty.clone(),
            mermaid_blocks: verify.mermaid_blocks,
            mermaid_issues: verify.mermaid_issues.clone(),
            fixer_available: verify.fixer_available,
            fixer_output: verify.fixer_output.clone(),
        },
    };
    drop(stats);

    let internal = &pctx.config.internal_path;
    std::fs::create_dir_all(internal).map_err(|e| Error::io(internal, e))?;
    let json_path = internal.join("summary.json");
    std::fs::write(&json_path, serde_json::to_string_pretty(&json).unwrap_or_default())
        .map_err(|e| Error::io(&json_path, e))?;

    // Human summary next to the docs.
    let mut md = String::from("# AgentWiki Run Summary\n\n");
    let _ = writeln!(md, "- Generated: {}", json.generated_at);
    let _ = writeln!(md, "- Project: `{}`", json.project);
    let _ = writeln!(md, "- Total: {:.1}s", json.total_secs);
    let _ = writeln!(md, "- CLI calls: {}", json.cli_calls);
    let _ = writeln!(md, "- Cache hits: {}", json.cache_hits);
    let _ = writeln!(
        md,
        "- Quota used today: {}/{}",
        json.quota_used_today, pctx.config.limits.daily_cap
    );
    let _ = writeln!(
        md,
        "- Mermaid blocks: {}, issues: {}",
        verify.mermaid_blocks,
        verify.mermaid_issues.len()
    );
    if !verify.missing.is_empty() {
        let _ = writeln!(md, "- Missing docs: {}", verify.missing.join(", "));
    }
    if !verify.mermaid_issues.is_empty() {
        md.push_str("\n## Mermaid issues\n\n");
        for i in &verify.mermaid_issues {
            let _ = writeln!(md, "- {i}");
        }
    }
    md.push_str("\n## Spec timings\n\n| Spec | Seconds |\n|------|---------|\n");
    for (name, secs) in &json.spec_timings {
        let _ = writeln!(md, "| {name} | {secs:.1} |");
    }

    let md_path = pctx.config.output_path.join("__AgentWiki_Summary__.md");
    std::fs::write(&md_path, md).map_err(|e| Error::io(&md_path, e))
}
