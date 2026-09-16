//! Formats scan data and dep results into `{{materials}}`/`{{custom}}`
//! prompt blocks — the DataFormatter port.

use std::fmt::Write as _;

use crate::agent::reports::{
    CodePurpose, DirectoryDossier, DirectorySummaryResponse, DomainModule, FileInsight,
    RelationshipAnalysis, classify_directory_purpose,
};
use crate::agent::spec::Material;
use crate::scanner::{DirectoryInfo, ScanData, structure};

/// Max README chars injected into prompts.
const README_CAP: usize = 16_384;
/// Truncated `source_summary` for database material.
const DB_SUMMARY_CAP: usize = 10_240;
/// Cap on filtered key-module insights.
const KEY_MODULE_INSIGHTS: usize = 50;
/// Cap on filtered database insights.
const DB_INSIGHTS: usize = 50;

/// A scan-material block rendered for `{{materials}}`. `CodeInsights`,
/// `Relationships` and `Custom` need ctx — the runner builds them.
pub fn render_material(m: Material, scan: &ScanData) -> Option<String> {
    match m {
        Material::ProjectStructure => Some(project_structure(scan)),
        Material::Readme => readme_block(scan),
        Material::CodeInsights | Material::Relationships | Material::Custom => None,
    }
}

/// `### Project Structure Overview` (dirs-only tree when huge).
pub fn project_structure(scan: &ScanData) -> String {
    if scan.files.len() > 100 {
        structure::format_as_directory_tree(scan)
    } else {
        structure::format_as_tree(scan)
    }
}

/// README block from the scan, if a README exists.
fn readme_block(scan: &ScanData) -> Option<String> {
    let text = scan.readme.as_ref()?;
    let text: String = text.chars().take(README_CAP).collect();
    Some(format!(
        "### Previous README Content for Style Reference\n```markdown\n{text}\n```\n"
    ))
}

/// `### Source Code Insights Summary` — top-N dossier insights.
pub fn code_insights_block(dossiers: &[DirectoryDossier], limit: usize) -> String {
    let mut all: Vec<&FileInsight> = dossiers
        .iter()
        .flat_map(|d| d.file_insights.iter())
        .collect();
    all.sort_by(|a, b| {
        b.importance_score
            .partial_cmp(&a.importance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut s = String::from("### Source Code Insights Summary\n```markdown\n");
    for i in all.iter().take(limit) {
        let _ = writeln!(
            s,
            "- **{}** (importance {:.1}, {:?}): {}",
            i.file_path.display(),
            i.importance_score,
            i.code_purpose,
            i.source_summary
        );
    }
    s.push_str("```\n");
    s
}

/// `### Dependency Relationship Analysis` — edge list.
pub fn relationships_block(report: Option<&RelationshipAnalysis>) -> String {
    let mut s = String::from("### Dependency Relationship Analysis\n```markdown\n");
    if let Some(r) = report {
        for rel in &r.core_dependencies {
            let _ = writeln!(
                s,
                "- {} → {} [{}] (importance {}){}",
                rel.from,
                rel.to,
                rel.dependency_type.as_str(),
                rel.importance,
                rel.description
                    .as_deref()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default(),
            );
        }
    }
    s.push_str("```\n");
    s
}

/// A dep result rendered as `#### Display Name` + fenced content.
pub fn dep_block(display: &str, value: &serde_json::Value) -> String {
    let body = match value {
        serde_json::Value::String(t) => t.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    format!("#### {display}\n```json\n{body}\n```\n")
}

/// Per-instance `{{custom}}` block for `dir_summary@<dir>`: directory
/// metadata + per-file metrics/interfaces/source preview.
pub fn dir_summary_custom(dir: &DirectoryInfo, file_source_chars: usize) -> String {
    let mut s = format!(
        "**Directory**: `{}`\n**Path**: `{}`\n**Direct files**: {}\n**Direct subdirectories**: {}\n\n**File details**:\n\n",
        dir.name,
        dir.rel_path.display(),
        dir.files.len(),
        dir.subdirectory_count,
    );
    for entry in &dir.files {
        let content = std::fs::read_to_string(&entry.abs_path).unwrap_or_default();
        let st = crate::scanner::extract(&entry.abs_path, &content);
        let interfaces: Vec<String> = st
            .interfaces
            .iter()
            .map(|i| format!("{} ({})", i.name, i.interface_type))
            .collect();
        let deps: Vec<String> = st.dependencies.iter().map(|d| d.name.clone()).collect();
        let _ = writeln!(
            s,
            "- File: `{}`\n  - Metrics: {} lines, {} functions, {} classes, importance {:.2}\n  - Interfaces: {}\n  - Dependencies: {}",
            entry.rel_path.display(),
            st.metrics.lines_of_code,
            st.metrics.number_of_functions,
            st.metrics.number_of_classes,
            entry.importance_score,
            join_or_none(&interfaces),
            join_or_none(&deps),
        );
        let preview: String = content.chars().take(file_source_chars).collect();
        if !preview.trim().is_empty() {
            let _ = writeln!(s, "  - Source preview:\n```\n{preview}\n```");
        }
    }
    s
}

/// `{{custom}}` for `relationships` — dossier summaries to reason over.
pub fn relationships_custom(dossiers: &[DirectoryDossier]) -> String {
    let mut s = String::from("**Directory dossiers**:\n\n```markdown\n");
    for d in dossiers {
        let _ = writeln!(
            s,
            "## `{}` — {} files, importance {:.2}\n{}\nKey files: {}\nInterfaces: {}\n",
            d.path.display(),
            d.file_count,
            d.importance_score,
            d.summary,
            d.key_files.join(", "),
            d.file_insights
                .iter()
                .flat_map(|i| i.interfaces.iter().map(|x| x.name.clone()))
                .take(10)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    s.push_str("```\n");
    s
}

/// `{{custom}}` for `key_module@<domain>` — domain detail + filtered
/// insights (files whose path intersects the domain's code_paths).
pub fn key_module_custom(domain: &DomainModule, dossiers: &[DirectoryDossier]) -> String {
    let submodule_names: Vec<String> = domain
        .sub_modules
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let mut s = format!(
        "**Domain**: {}\n**Type**: {}\n**Description**: {}\n**Code paths**: {}\n**Submodules**: {}\n\n",
        domain.name,
        domain.domain_type,
        domain.description,
        domain.code_paths.join(", "),
        submodule_names.join(", "),
    );
    s.push_str("**Relevant code insights**:\n```markdown\n");
    for i in filtered_insights(dossiers, &domain.code_paths, KEY_MODULE_INSIGHTS) {
        let _ = writeln!(
            s,
            "- **{}** [{:?}]: {}",
            i.file_path.display(),
            i.code_purpose,
            i.source_summary
        );
    }
    s.push_str("```\n");
    s
}

/// `{{custom}}` for `boundary` — entry/API/config/router insights.
pub fn boundary_custom(dossiers: &[DirectoryDossier], limit: usize) -> String {
    use CodePurpose::*;
    let wanted = [Entry, Api, Config, Router, Controller, Command];
    let mut matched: Vec<&FileInsight> = dossiers
        .iter()
        .flat_map(|d| d.file_insights.iter())
        .filter(|i| wanted.contains(&i.code_purpose))
        .collect();
    sort_insights(&mut matched);
    let mut s =
        String::from("**Boundary-related code insights** (entry/api/config):\n```markdown\n");
    for i in matched.iter().take(limit) {
        let _ = writeln!(
            s,
            "- **{}** [{:?}]: {}",
            i.file_path.display(),
            i.code_purpose,
            i.source_summary
        );
    }
    s.push_str("```\n");
    s
}

/// `{{custom}}` for `database` — DB/DAO/sql insights.
pub fn database_custom(dossiers: &[DirectoryDossier]) -> String {
    use CodePurpose::*;
    let mut matched: Vec<&FileInsight> = dossiers
        .iter()
        .flat_map(|d| d.file_insights.iter())
        .filter(|i| {
            matches!(i.code_purpose, Database | Dao)
                || i.file_path.ends_with(".sql")
                || i.file_path.ends_with(".sqlproj")
        })
        .collect();
    sort_insights(&mut matched);
    let mut s = String::from("**Database-related code insights**:\n```markdown\n");
    for i in matched.iter().take(DB_INSIGHTS) {
        let summary: String = i.source_summary.chars().take(DB_SUMMARY_CAP).collect();
        let _ = writeln!(
            s,
            "- **{}** [{:?}]: {}",
            i.file_path.display(),
            i.code_purpose,
            summary
        );
    }
    s.push_str("```\n");
    s
}

/// Merge a dir_summary response with scanner metadata → dossier.
/// `file_path` is filled in here — the model only reports file names.
pub fn dossier_from(dir: &DirectoryInfo, resp: &DirectorySummaryResponse) -> DirectoryDossier {
    let mut dossier = DirectoryDossier {
        path: dir.path.clone(),
        name: dir.name.clone(),
        purpose: classify_directory_purpose(&dir.name),
        file_count: dir.files.len(),
        subdirectory_count: dir.subdirectory_count,
        importance_score: resp.importance_score,
        summary: resp.summary.clone(),
        key_files: resp.key_files.clone(),
        file_insights: resp.file_insights.clone(),
    };
    for fi in &mut dossier.file_insights {
        if fi.file_path.as_os_str().is_empty() {
            fi.file_path = dir.rel_path.join(&fi.name);
        }
    }
    dossier
}

/// Insights whose file path intersects any of `paths` (either direction).
fn filtered_insights<'a>(
    dossiers: &'a [DirectoryDossier],
    paths: &[String],
    limit: usize,
) -> Vec<&'a FileInsight> {
    let mut v: Vec<&FileInsight> = dossiers
        .iter()
        .flat_map(|d| d.file_insights.iter())
        .filter(|i| {
            let fp = i.file_path.to_string_lossy();
            paths
                .iter()
                .any(|p| fp.contains(p.as_str()) || p.contains(fp.as_ref()))
        })
        .collect();
    sort_insights(&mut v);
    v.truncate(limit);
    v
}

fn sort_insights(v: &mut [&FileInsight]) {
    v.sort_by(|a, b| {
        b.importance_score
            .partial_cmp(&a.importance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn join_or_none(v: &[String]) -> String {
    if v.is_empty() {
        "none".to_string()
    } else {
        v.join(", ")
    }
}
