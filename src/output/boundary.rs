//! Deterministic `5.Boundary-Interfaces.md` renderer — ported from
//! deepwiki-rs `BoundaryEditor::generate_boundary_documentation`.

use std::fmt::Write as _;

use crate::agent::reports::{
    APIBoundary, BoundaryAnalysisReport, CLIBoundary, IntegrationSuggestion, RouterBoundary,
};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::scanner::ScanData;

/// [`DetFn`](crate::agent::spec::DetFn): render the boundary report as
/// markdown. `dep` is the `boundary` research result.
pub fn boundary_doc(_scan: &ScanData, _config: &Config, dep: &serde_json::Value) -> Result<String> {
    let report: BoundaryAnalysisReport =
        serde_json::from_value(dep.clone()).map_err(|e| Error::Parse {
            agent: "boundary_doc".to_string(),
            message: e.to_string(),
        })?;

    let mut s = String::from(
        "# System Boundary Interface Documentation\n\n\
         This document describes the system's external invocation interfaces, \
         including CLI commands, API endpoints, configuration parameters, and \
         other boundary mechanisms.\n\n",
    );
    if !report.cli_boundaries.is_empty() {
        s.push_str(&cli_section(&report.cli_boundaries));
    }
    if !report.api_boundaries.is_empty() {
        s.push_str(&api_section(&report.api_boundaries));
    }
    if !report.router_boundaries.is_empty() {
        s.push_str(&router_section(&report.router_boundaries));
    }
    if !report.integration_suggestions.is_empty() {
        s.push_str(&integration_section(&report.integration_suggestions));
    }
    let _ = write!(
        s,
        "\n---\n\n**Analysis Confidence**: {:.1}/10\n",
        report.confidence_score
    );
    Ok(s)
}

fn cli_section(cli_boundaries: &[CLIBoundary]) -> String {
    let mut s = String::from("## Command Line Interface (CLI)\n\n");
    for cli in cli_boundaries {
        let _ = writeln!(s, "### {}\n", cli.command);
        let _ = writeln!(s, "**Description**: {}\n", cli.description);
        let _ = writeln!(s, "**Source File**: `{}`\n", cli.source_location);
        if !cli.arguments.is_empty() {
            s.push_str("**Arguments**:\n\n");
            for arg in &cli.arguments {
                let req = if arg.required { "required" } else { "optional" };
                let default = arg
                    .default_value
                    .as_ref()
                    .map(|v| format!(" (default: `{v}`)"))
                    .unwrap_or_default();
                let _ = writeln!(
                    s,
                    "- `{}` ({}): {} - {}{}",
                    arg.name, arg.value_type, req, arg.description, default
                );
            }
            s.push('\n');
        }
        if !cli.options.is_empty() {
            s.push_str("**Options**:\n\n");
            for opt in &cli.options {
                let short = opt
                    .short_name
                    .as_ref()
                    .map(|v| format!(", {v}"))
                    .unwrap_or_default();
                let req = if opt.required { "required" } else { "optional" };
                let default = opt
                    .default_value
                    .as_ref()
                    .map(|v| format!(" (default: `{v}`)"))
                    .unwrap_or_default();
                let _ = writeln!(
                    s,
                    "- `{}{}`({}): {} - {}{}",
                    opt.name, short, opt.value_type, req, opt.description, default
                );
            }
            s.push('\n');
        }
        if !cli.examples.is_empty() {
            s.push_str("**Usage Examples**:\n\n");
            for ex in &cli.examples {
                let _ = writeln!(s, "```bash\n{ex}\n```\n");
            }
        }
    }
    s
}

fn api_section(api_boundaries: &[APIBoundary]) -> String {
    let mut s = String::from("## API Interfaces\n\n");
    for api in api_boundaries {
        let _ = writeln!(s, "### {} {}\n", api.method, api.endpoint);
        let _ = writeln!(s, "**Description**: {}\n", api.description);
        let _ = writeln!(s, "**Source File**: `{}`\n", api.source_location);
        if let Some(f) = &api.request_format {
            let _ = writeln!(s, "**Request Format**: {}\n", f);
        }
        if let Some(f) = &api.response_format {
            let _ = writeln!(s, "**Response Format**: {}\n", f);
        }
        if let Some(a) = &api.authentication {
            let _ = writeln!(s, "**Authentication**: {}\n", a);
        }
    }
    s
}

fn router_section(router_boundaries: &[RouterBoundary]) -> String {
    let mut s = String::from("## Router Routes\n\n");
    for r in router_boundaries {
        let _ = writeln!(s, "### {}\n", r.path);
        let _ = writeln!(s, "**Description**: {}\n", r.description);
        let _ = writeln!(s, "**Source File**: `{}`\n", r.source_location);
        if !r.params.is_empty() {
            s.push_str("**Parameters**:\n\n");
            for p in &r.params {
                let _ = writeln!(s, "- `{}` ({}): {}", p.key, p.value_type, p.description);
            }
            s.push('\n');
        }
    }
    s
}

fn integration_section(suggestions: &[IntegrationSuggestion]) -> String {
    let mut s = String::from("## Integration Suggestions\n\n");
    for i in suggestions {
        let _ = writeln!(s, "### {}\n", i.integration_type);
        let _ = writeln!(s, "{}\n", i.description);
        if !i.example_code.is_empty() {
            s.push_str("**Example Code**:\n\n");
            let _ = writeln!(s, "```\n{}\n```\n", i.example_code);
        }
        if !i.best_practices.is_empty() {
            s.push_str("**Best Practices**:\n\n");
            for p in &i.best_practices {
                let _ = writeln!(s, "- {p}");
            }
            s.push('\n');
        }
    }
    s
}
