//! Deterministic `6.Database-Overview.md` renderer — ported from
//! deepwiki-rs `DatabaseEditor::generate_database_documentation`.

use std::fmt::Write as _;

use crate::agent::reports::{
    DataFlow, DatabaseFunction, DatabaseOverviewReport, DatabaseProject, DatabaseTable,
    DatabaseView, StoredProcedure,
};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::scanner::ScanData;

/// [`DetFn`](crate::agent::spec::DetFn): render the database report as
/// markdown. `dep` is the `database` research result.
pub fn database_doc(_scan: &ScanData, _config: &Config, dep: &serde_json::Value) -> Result<String> {
    let report: DatabaseOverviewReport =
        serde_json::from_value(dep.clone()).map_err(|e| Error::Parse {
            agent: "database_doc".to_string(),
            message: e.to_string(),
        })?;

    let mut s = String::from("## Database Overview\n\n### Summary\n\n");
    s.push_str("| Metric | Count |\n|--------|-------|\n");
    let _ = writeln!(
        s,
        "| Database Projects | {} |",
        report.database_projects.len()
    );
    let _ = writeln!(s, "| Tables | {} |", report.tables.len());
    let _ = writeln!(s, "| Views | {} |", report.views.len());
    let _ = writeln!(
        s,
        "| Stored Procedures | {} |",
        report.stored_procedures.len()
    );
    let _ = writeln!(s, "| Functions | {} |", report.database_functions.len());
    let _ = writeln!(
        s,
        "| Relationships | {} |",
        report.table_relationships.len()
    );
    s.push('\n');

    if !report.database_projects.is_empty() {
        s.push_str("### Database Projects\n\n");
        for p in &report.database_projects {
            project(&mut s, p);
        }
    }
    if !report.tables.is_empty() {
        s.push_str("### Tables\n\n");
        for t in &report.tables {
            table(&mut s, t);
        }
    }
    if !report.views.is_empty() {
        s.push_str("### Views\n\n");
        for v in &report.views {
            view(&mut s, v);
        }
    }
    if !report.stored_procedures.is_empty() {
        s.push_str("### Stored Procedures\n\n");
        for p in &report.stored_procedures {
            proc(&mut s, p);
        }
    }
    if !report.database_functions.is_empty() {
        s.push_str("### Functions\n\n");
        for f in &report.database_functions {
            func(&mut s, f);
        }
    }
    if !report.table_relationships.is_empty() {
        s.push_str("### Table Relationships\n\n```mermaid\nerDiagram\n");
        for r in &report.table_relationships {
            let _ = writeln!(
                s,
                "    {} ||--o{{ {} : \"{}\"",
                mermaid_id(&r.from_table),
                mermaid_id(&r.to_table),
                r.relationship_type
            );
        }
        s.push_str("```\n\n");
        s.push_str("| From Table | From Columns | To Table | To Columns | Type |\n");
        s.push_str("|------------|--------------|----------|------------|------|\n");
        for r in &report.table_relationships {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                r.from_table,
                r.from_columns.join(", "),
                r.to_table,
                r.to_columns.join(", "),
                r.relationship_type
            );
        }
        s.push('\n');
    }
    if !report.data_flows.is_empty() {
        s.push_str("### Data Flows\n\n");
        for f in &report.data_flows {
            flow(&mut s, f);
        }
    }
    let _ = write!(
        s,
        "\n---\n\n**Analysis Confidence**: {:.1}/10\n",
        report.confidence_score
    );
    Ok(s)
}

fn project(s: &mut String, p: &DatabaseProject) {
    let _ = writeln!(s, "#### `{}`\n", p.name);
    let _ = writeln!(s, "- Path: `{}`", p.project_path);
    if let Some(t) = &p.target_platform {
        let _ = writeln!(s, "- Target platform: {t}");
    }
    let _ = writeln!(
        s,
        "- Objects: {} tables, {} views, {} procedures, {} functions\n",
        p.table_count, p.view_count, p.procedure_count, p.function_count
    );
    if !p.references.is_empty() {
        let _ = writeln!(s, "- References: {}", p.references.join(", "));
    }
    s.push('\n');
}

fn table(s: &mut String, t: &DatabaseTable) {
    let _ = writeln!(s, "#### `{}.{}`\n", t.schema, t.name);
    if !t.description.is_empty() {
        let _ = writeln!(s, "{}\n", t.description);
    }
    if !t.columns.is_empty() {
        s.push_str("| Column | Type | Nullable | Identity | Default |\n");
        s.push_str("|--------|------|----------|----------|---------|\n");
        for c in &t.columns {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                c.name,
                c.data_type,
                c.nullable,
                c.is_identity,
                c.default_value.as_deref().unwrap_or("")
            );
        }
        s.push('\n');
    }
    if !t.primary_key.is_empty() {
        let _ = writeln!(s, "**Primary key**: {}\n", t.primary_key.join(", "));
    }
    if !t.source_path.is_empty() {
        let _ = writeln!(s, "**Source**: `{}`\n", t.source_path);
    }
}

fn view(s: &mut String, v: &DatabaseView) {
    let _ = writeln!(s, "#### `{}.{}`\n", v.schema, v.name);
    if !v.description.is_empty() {
        let _ = writeln!(s, "{}\n", v.description);
    }
    if !v.referenced_tables.is_empty() {
        let _ = writeln!(s, "**References**: {}\n", v.referenced_tables.join(", "));
    }
    if !v.source_path.is_empty() {
        let _ = writeln!(s, "**Source**: `{}`\n", v.source_path);
    }
}

fn proc(s: &mut String, p: &StoredProcedure) {
    let _ = writeln!(s, "#### `{}.{}`\n", p.schema, p.name);
    if !p.description.is_empty() {
        let _ = writeln!(s, "{}\n", p.description);
    }
    if !p.parameters.is_empty() {
        s.push_str("**Parameters**:\n\n");
        for param in &p.parameters {
            let _ = writeln!(
                s,
                "- `{}` {} ({}{})",
                param.name,
                param.data_type,
                param.direction,
                if param.is_optional { ", optional" } else { "" }
            );
        }
        s.push('\n');
    }
    if !p.referenced_tables.is_empty() {
        let _ = writeln!(s, "**Tables**: {}\n", p.referenced_tables.join(", "));
    }
}

fn func(s: &mut String, f: &DatabaseFunction) {
    let _ = writeln!(s, "#### `{}.{}`\n", f.schema, f.name);
    if !f.function_type.is_empty() {
        let _ = writeln!(s, "Type: {}\n", f.function_type);
    }
    if !f.description.is_empty() {
        let _ = writeln!(s, "{}\n", f.description);
    }
    if !f.return_type.is_empty() {
        let _ = writeln!(s, "**Returns**: `{}`\n", f.return_type);
    }
    if !f.source_path.is_empty() {
        let _ = writeln!(s, "**Source**: `{}`\n", f.source_path);
    }
}

fn flow(s: &mut String, f: &DataFlow) {
    let _ = writeln!(s, "#### {}\n", f.name);
    let _ = writeln!(s, "- {} → {}", f.source, f.destination);
    if !f.operations.is_empty() {
        let _ = writeln!(s, "- Operations: {}", f.operations.join(", "));
    }
    if !f.procedures_involved.is_empty() {
        let _ = writeln!(s, "- Procedures: {}", f.procedures_involved.join(", "));
    }
    s.push('\n');
}

/// ASCII-safe mermaid node id (schema/table names may contain dots).
fn mermaid_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
