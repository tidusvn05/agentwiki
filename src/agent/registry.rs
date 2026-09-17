//! The task DAG — every research/compose node, its deps, fan-out axis,
//! materials, and output contract. Mirrors deepwiki-rs's orchestrator.

use crate::agent::reports;
use crate::agent::spec::{AgentSpec, ExecKind, FanOut, Material, Phase, schema_spec};
use crate::config::ModelTier;

use ExecKind::Llm;
use FanOut::{PerDir, PerDomain};
use Material::{CodeInsights, Custom, ProjectStructure, Readme, Relationships};
use ModelTier::{Efficient, Powerful};
use Phase::{Compose, Research};

/// All specs, in declaration order. Deps determine execution levels.
pub fn all_specs() -> Vec<AgentSpec> {
    let mut v = research_specs();
    v.extend(compose_specs());
    v
}

/// Phase-1 research agents.
pub fn research_specs() -> Vec<AgentSpec> {
    vec![
        AgentSpec {
            name: "dir_summary",
            prompt_tmpl: "dir_summary.md",
            schema: Some(schema_spec::<reports::DirectorySummaryResponse>()),
            tier: Efficient,
            deps: &[],
            fan_out: Some(PerDir),
            materials: &[Custom],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "relationships",
            prompt_tmpl: "relationships.md",
            schema: Some(schema_spec::<reports::RelationshipAnalysis>()),
            tier: Efficient,
            deps: &["dir_summary"],
            fan_out: None,
            materials: &[Custom],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "system_context",
            prompt_tmpl: "system_context.md",
            schema: Some(schema_spec::<reports::SystemContextReport>()),
            tier: Efficient,
            deps: &["dir_summary"],
            fan_out: None,
            materials: &[ProjectStructure, CodeInsights, Readme],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "domain_modules",
            prompt_tmpl: "domain_modules.md",
            schema: Some(schema_spec::<reports::DomainModulesReport>()),
            tier: Efficient,
            deps: &["dir_summary", "system_context", "relationships"],
            fan_out: None,
            materials: &[CodeInsights, ProjectStructure],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "database",
            prompt_tmpl: "database.md",
            schema: Some(schema_spec::<reports::DatabaseOverviewReport>()),
            tier: Efficient,
            deps: &["dir_summary"],
            fan_out: None,
            materials: &[ProjectStructure, Custom],
            phase: Research,
            exec: Llm,
        },
        // deepwiki-rs: architecture research output is free-form text,
        // not a JSON report.
        AgentSpec {
            name: "architecture",
            prompt_tmpl: "architecture.md",
            schema: None,
            tier: Powerful,
            deps: &["system_context", "domain_modules"],
            fan_out: None,
            materials: &[ProjectStructure, Relationships],
            phase: Research,
            exec: Llm,
        },
        // Workflow research output is free-form text in the reference.
        AgentSpec {
            name: "workflow",
            prompt_tmpl: "workflow.md",
            schema: None,
            tier: Powerful,
            deps: &["system_context", "domain_modules"],
            fan_out: None,
            materials: &[CodeInsights],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "key_module",
            prompt_tmpl: "key_module.md",
            // Per-domain fan-out; each instance returns one report.
            schema: Some(schema_spec::<reports::KeyModuleReport>()),
            tier: Efficient,
            deps: &["system_context", "domain_modules"],
            fan_out: Some(PerDomain),
            materials: &[Custom],
            phase: Research,
            exec: Llm,
        },
        AgentSpec {
            name: "boundary",
            prompt_tmpl: "boundary.md",
            schema: Some(schema_spec::<reports::BoundaryAnalysisReport>()),
            tier: Efficient,
            deps: &["system_context", "relationships"],
            fan_out: None,
            materials: &[ProjectStructure, Custom],
            phase: Research,
            exec: Llm,
        },
    ]
}

/// Phase-2 compose agents. `boundary_doc`/`database_doc` are deterministic
/// renderers — no CLI call.
pub fn compose_specs() -> Vec<AgentSpec> {
    vec![
        AgentSpec {
            name: "overview",
            prompt_tmpl: "editors/overview.md",
            schema: None,
            tier: Efficient,
            deps: &["system_context", "domain_modules"],
            fan_out: None,
            materials: &[Readme],
            phase: Compose,
            exec: Llm,
        },
        AgentSpec {
            name: "architecture_doc",
            prompt_tmpl: "editors/architecture_doc.md",
            schema: None,
            tier: Powerful,
            deps: &[
                "system_context",
                "domain_modules",
                "architecture",
                "workflow",
            ],
            fan_out: None,
            materials: &[],
            phase: Compose,
            exec: Llm,
        },
        AgentSpec {
            name: "workflow_doc",
            prompt_tmpl: "editors/workflow_doc.md",
            schema: None,
            tier: Powerful,
            deps: &["system_context", "domain_modules", "workflow"],
            fan_out: None,
            materials: &[CodeInsights],
            phase: Compose,
            exec: Llm,
        },
        AgentSpec {
            name: "boundary_doc",
            prompt_tmpl: "",
            schema: None,
            tier: Efficient,
            deps: &["boundary"],
            fan_out: None,
            materials: &[],
            phase: Compose,
            exec: ExecKind::Deterministic(crate::output::boundary_doc),
        },
        AgentSpec {
            name: "database_doc",
            prompt_tmpl: "",
            schema: None,
            tier: Efficient,
            deps: &["database"],
            fan_out: None,
            materials: &[],
            phase: Compose,
            exec: ExecKind::Deterministic(crate::output::database_doc),
        },
        AgentSpec {
            name: "deep_dive",
            prompt_tmpl: "editors/deep_dive.md",
            schema: None,
            tier: Powerful,
            deps: &[
                "system_context",
                "domain_modules",
                "architecture",
                "workflow",
                "key_module",
            ],
            fan_out: Some(PerDomain),
            materials: &[Custom],
            phase: Compose,
            exec: Llm,
        },
    ]
}

/// Display name used when a dep result is injected into a prompt.
pub fn display_name(spec_name: &str) -> &str {
    match spec_name {
        "dir_summary" => "Directory Summary Report",
        "relationships" => "Code Relationships Report",
        "system_context" => "System Context Research Report",
        "domain_modules" => "Domain Modules Research Report",
        "architecture" => "Architecture Research Report",
        "workflow" => "Workflow Research Report",
        "key_module" => "Key Module Code Insight",
        "boundary" => "Boundary Analysis Report",
        "database" => "Database Overview Report",
        other => other,
    }
}

/// Kahn level-ordering: `levels[i]` = specs whose deps are all in earlier
/// levels. Specs may appear in at most one level.
pub fn topo_levels(specs: &[AgentSpec]) -> Vec<Vec<usize>> {
    let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
    let mut levels: Vec<Vec<usize>> = Vec::new();
    let mut done: Vec<bool> = vec![false; specs.len()];
    loop {
        let level: Vec<usize> = (0..specs.len())
            .filter(|&i| {
                !done[i]
                    && specs[i].deps.iter().all(|d| {
                        !names.contains(d)
                            || specs
                                .iter()
                                .position(|s| s.name == *d)
                                .is_some_and(|p| done[p])
                    })
            })
            .collect();
        if level.is_empty() {
            break;
        }
        for &i in &level {
            done[i] = true;
        }
        levels.push(level);
    }
    debug_assert!(done.iter().all(|d| *d), "dependency cycle in spec DAG");
    levels
}
