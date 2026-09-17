//! Structured output types for research-phase agents.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::lenient::{
    any_to_string, de_bool, de_f64, de_opt_string, de_string, de_usize, de_vec_obj, de_vec_string,
};

// ============================ system_context ============================

/// Coarse project-type classification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum ProjectType {
    /// Browser-facing UI app.
    FrontendApp,
    /// Headless backend service.
    BackendService,
    /// Both ends in one repo.
    FullStackApp,
    /// Reusable component library.
    ComponentLibrary,
    /// Developer framework.
    Framework,
    /// Command-line tool.
    CLITool,
    /// Mobile application.
    MobileApp,
    /// Desktop application.
    DesktopApp,
    /// Anything else.
    #[default]
    Other,
}

impl ProjectType {
    /// Lenient raw-string → variant mapping.
    pub fn map_from_raw(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "frontendapp" | "frontend" | "frontend_app" | "web" => Self::FrontendApp,
            "backendservice" | "backend" | "backend_service" | "service" => Self::BackendService,
            "fullstackapp" | "fullstack" | "full_stack" => Self::FullStackApp,
            "componentlibrary" | "component_library" | "library" => Self::ComponentLibrary,
            "framework" => Self::Framework,
            "clitool" | "cli_tool" | "cli" => Self::CLITool,
            "mobileapp" | "mobile" | "mobile_app" => Self::MobileApp,
            "desktopapp" | "desktop" | "desktop_app" => Self::DesktopApp,
            _ => Self::Other,
        }
    }
}

fn de_project_type<'de, D>(deserializer: D) -> Result<ProjectType, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(ProjectType::map_from_raw(&any_to_string(value)))
}

/// A user persona.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct UserPersona {
    /// Persona name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Who they are.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// What they need from the system.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub needs: Vec<String>,
}

/// An external system the project interacts with.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ExternalSystem {
    /// System name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// What it is.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// How the project talks to it.
    #[serde(default, deserialize_with = "de_string")]
    pub interaction_type: String,
}

/// What is inside vs outside the system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct SystemBoundary {
    /// Scope description.
    #[serde(default, deserialize_with = "de_string")]
    pub scope: String,
    /// Components inside the boundary.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub included_components: Vec<String>,
    /// Components explicitly outside.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub excluded_components: Vec<String>,
}

fn de_system_boundary<'de, D>(deserializer: D) -> Result<SystemBoundary, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let from_map = |map: &serde_json::Map<String, serde_json::Value>| SystemBoundary {
        scope: map
            .get("scope")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default(),
        included_components: str_list(map.get("included_components")),
        excluded_components: str_list(map.get("excluded_components")),
    };
    match value {
        serde_json::Value::Object(map) => Ok(from_map(&map)),
        serde_json::Value::String(s) => {
            if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&s) {
                Ok(from_map(&map))
            } else {
                Err(serde::de::Error::custom(
                    "system_boundary string is not JSON",
                ))
            }
        }
        _ => Err(serde::de::Error::custom(
            "system_boundary must be an object or string",
        )),
    }
}

fn str_list(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

/// `system_context` agent output — C4 SystemContext-level facts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct SystemContextReport {
    /// Project name.
    #[serde(default, deserialize_with = "de_string")]
    pub project_name: String,
    /// What the project does.
    #[serde(default, deserialize_with = "de_string")]
    pub project_description: String,
    /// Coarse type.
    #[serde(default, deserialize_with = "de_project_type")]
    pub project_type: ProjectType,
    /// Why it exists.
    #[serde(default, deserialize_with = "de_string")]
    pub business_value: String,
    /// Who uses it.
    #[serde(default, deserialize_with = "de_vec_persona")]
    pub target_users: Vec<UserPersona>,
    /// External dependencies.
    #[serde(default, deserialize_with = "de_vec_external_system")]
    pub external_systems: Vec<ExternalSystem>,
    /// In/out of scope.
    #[serde(default, deserialize_with = "de_system_boundary")]
    pub system_boundary: SystemBoundary,
    /// 0–10 confidence.
    #[serde(default, deserialize_with = "de_f64")]
    pub confidence_score: f64,
}

fn de_vec_persona<'de, D>(deserializer: D) -> Result<Vec<UserPersona>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(deserializer, |name| {
        Some(UserPersona {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_external_system<'de, D>(deserializer: D) -> Result<Vec<ExternalSystem>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(deserializer, |name| {
        Some(ExternalSystem {
            name,
            interaction_type: "unknown".to_string(),
            ..Default::default()
        })
    })
}

// ============================ domain_modules ============================

/// A specific implementation module inside a domain.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct SubModule {
    /// Sub-module name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Role and responsibilities.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Implementing file paths.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub code_paths: Vec<String>,
    /// Main functions/operations.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub key_functions: Vec<String>,
    /// 1–10 importance.
    #[serde(default, deserialize_with = "de_f64")]
    pub importance: f64,
}

/// A high-level functional/business domain.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DomainModule {
    /// Domain name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Responsibilities and role.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// e.g. "Core Business Domain", "Infrastructure Domain".
    #[serde(default, deserialize_with = "de_string")]
    pub domain_type: String,
    /// Sub-modules inside the domain.
    #[serde(default, deserialize_with = "de_vec_submodule")]
    pub sub_modules: Vec<SubModule>,
    /// Implementing file paths.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub code_paths: Vec<String>,
    /// 1–10 strategic importance.
    #[serde(default, deserialize_with = "de_f64")]
    pub importance: f64,
    /// 1–10 technical complexity.
    #[serde(default, deserialize_with = "de_f64")]
    pub complexity: f64,
}

/// Dependency/collaboration edge between two domains.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DomainRelation {
    /// Initiator domain.
    #[serde(default, deserialize_with = "de_string")]
    pub from_domain: String,
    /// Receiver domain.
    #[serde(default, deserialize_with = "de_string")]
    pub to_domain: String,
    /// e.g. "Data Dependency", "Service Call".
    #[serde(default, deserialize_with = "de_string")]
    pub relation_type: String,
    /// 1–10 coupling strength.
    #[serde(default, deserialize_with = "de_f64")]
    pub strength: f64,
    /// Interaction detail.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
}

/// One step in a business flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct BusinessFlowStep {
    /// 1-based order.
    #[serde(default, deserialize_with = "de_usize")]
    pub step: usize,
    /// Primary executing domain.
    #[serde(default, deserialize_with = "de_string")]
    pub domain_module: String,
    /// Optional specific sub-module.
    #[serde(default)]
    pub sub_module: Option<String>,
    /// What the step does.
    #[serde(default, deserialize_with = "de_string")]
    pub operation: String,
    /// Main code location/function.
    #[serde(default)]
    pub code_entry_point: Option<String>,
}

/// A key functional scenario / execution path.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct BusinessFlow {
    /// Flow name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Goal, trigger, expected result.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Ordered steps.
    #[serde(default, deserialize_with = "de_vec_flow_step")]
    pub steps: Vec<BusinessFlowStep>,
    /// How the flow starts.
    #[serde(default, deserialize_with = "de_string")]
    pub entry_point: String,
    /// 1–10 importance.
    #[serde(default, deserialize_with = "de_f64")]
    pub importance: f64,
    /// Domains spanned.
    #[serde(default, deserialize_with = "de_usize")]
    pub involved_domains_count: usize,
}

/// `domain_modules` agent output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DomainModulesReport {
    /// Identified domains.
    #[serde(default, deserialize_with = "de_vec_domain_module")]
    pub domain_modules: Vec<DomainModule>,
    /// Inter-domain relations.
    #[serde(default, deserialize_with = "de_vec_domain_relation")]
    pub domain_relations: Vec<DomainRelation>,
    /// Core business flows.
    #[serde(default, deserialize_with = "de_vec_business_flow")]
    pub business_flows: Vec<BusinessFlow>,
    /// Macro architecture summary.
    #[serde(default, deserialize_with = "de_string")]
    pub architecture_summary: String,
    /// 0–10 confidence.
    #[serde(default, deserialize_with = "de_f64")]
    pub confidence_score: f64,
}

fn de_vec_submodule<'de, D>(d: D) -> Result<Vec<SubModule>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(SubModule {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_domain_module<'de, D>(d: D) -> Result<Vec<DomainModule>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DomainModule {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_domain_relation<'de, D>(d: D) -> Result<Vec<DomainRelation>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |_| None)
}

fn de_vec_flow_step<'de, D>(deserializer: D) -> Result<Vec<BusinessFlowStep>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for (idx, item) in items.into_iter().enumerate() {
                match item {
                    serde_json::Value::Object(map) => {
                        if let Ok(p) = serde_json::from_value::<BusinessFlowStep>(
                            serde_json::Value::Object(map),
                        ) {
                            out.push(p);
                        }
                    }
                    other => {
                        let operation = any_to_string(other);
                        if !operation.trim().is_empty() {
                            out.push(BusinessFlowStep {
                                step: idx + 1,
                                operation,
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

fn de_vec_business_flow<'de, D>(d: D) -> Result<Vec<BusinessFlow>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(BusinessFlow {
            name,
            ..Default::default()
        })
    })
}

// ============================ key_module ============================

/// `key_module` per-domain fan-out output.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct KeyModuleReport {
    /// Domain analyzed (filled by runner).
    #[serde(default, deserialize_with = "de_string")]
    pub domain_name: String,
    /// Module name.
    #[serde(default, deserialize_with = "de_string")]
    pub module_name: String,
    /// Current technical solution.
    #[serde(default, deserialize_with = "de_string")]
    pub module_description: String,
    /// Defined interfaces and interactions.
    #[serde(default, deserialize_with = "de_string")]
    pub interaction: String,
    /// Implementation detail.
    #[serde(default, deserialize_with = "de_string")]
    pub implementation: String,
    /// Related files.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub associated_files: Vec<String>,
    /// Flowchart source (mermaid, or empty).
    #[serde(default, deserialize_with = "de_string")]
    pub flowchart_mermaid: String,
    /// Sequence diagram source (mermaid, or empty).
    #[serde(default, deserialize_with = "de_string")]
    pub sequence_diagram_mermaid: String,
}

// ============================ boundary ============================

/// CLI argument definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct CLIArgument {
    /// Argument name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// What it does.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Required?
    #[serde(default, deserialize_with = "de_bool")]
    pub required: bool,
    /// Default if any.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub default_value: Option<String>,
    /// Value type.
    #[serde(default, deserialize_with = "de_string")]
    pub value_type: String,
}

/// CLI option/flag definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct CLIOption {
    /// Long name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Short name if any.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub short_name: Option<String>,
    /// What it does.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Required?
    #[serde(default, deserialize_with = "de_bool")]
    pub required: bool,
    /// Default if any.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub default_value: Option<String>,
    /// Value type.
    #[serde(default, deserialize_with = "de_string")]
    pub value_type: String,
}

/// One CLI command boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct CLIBoundary {
    /// Command name/path.
    #[serde(default, deserialize_with = "de_string")]
    pub command: String,
    /// What it does.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Positional arguments.
    #[serde(default, deserialize_with = "de_vec_cli_arg")]
    pub arguments: Vec<CLIArgument>,
    /// Flags/options.
    #[serde(default, deserialize_with = "de_vec_cli_option")]
    pub options: Vec<CLIOption>,
    /// Usage examples.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub examples: Vec<String>,
    /// Where it is defined.
    #[serde(default, deserialize_with = "de_string")]
    pub source_location: String,
}

/// One network API boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct APIBoundary {
    /// Endpoint path.
    #[serde(default, deserialize_with = "de_string")]
    pub endpoint: String,
    /// HTTP/RPC method.
    #[serde(default, deserialize_with = "de_string")]
    pub method: String,
    /// What it does.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Request shape.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub request_format: Option<String>,
    /// Response shape.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub response_format: Option<String>,
    /// Auth mechanism.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub authentication: Option<String>,
    /// Where it is defined.
    #[serde(default, deserialize_with = "de_string")]
    pub source_location: String,
}

/// Route parameter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct RouterParam {
    /// Parameter key.
    #[serde(default, deserialize_with = "de_string")]
    pub key: String,
    /// Value type.
    #[serde(default, deserialize_with = "de_string")]
    pub value_type: String,
    /// What it is for.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
}

/// One page/URL route boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct RouterBoundary {
    /// Route path.
    #[serde(default, deserialize_with = "de_string")]
    pub path: String,
    /// What it serves.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Where it is defined.
    #[serde(default, deserialize_with = "de_string")]
    pub source_location: String,
    /// Route parameters.
    #[serde(default, deserialize_with = "de_vec_router_param")]
    pub params: Vec<RouterParam>,
}

/// How to integrate with the system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct IntegrationSuggestion {
    /// e.g. "REST client", "CLI invocation".
    #[serde(default, deserialize_with = "de_string")]
    pub integration_type: String,
    /// Description.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Example code.
    #[serde(default, deserialize_with = "de_string")]
    pub example_code: String,
    /// Recommended practices.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub best_practices: Vec<String>,
}

/// `boundary` agent output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct BoundaryAnalysisReport {
    /// CLI boundaries.
    #[serde(default, deserialize_with = "de_vec_cli_boundary")]
    pub cli_boundaries: Vec<CLIBoundary>,
    /// Network API boundaries.
    #[serde(default, deserialize_with = "de_vec_api_boundary")]
    pub api_boundaries: Vec<APIBoundary>,
    /// Page routes.
    #[serde(default, deserialize_with = "de_vec_router_boundary")]
    pub router_boundaries: Vec<RouterBoundary>,
    /// Integration suggestions.
    #[serde(default, deserialize_with = "de_vec_integration")]
    pub integration_suggestions: Vec<IntegrationSuggestion>,
    /// 0–10 confidence.
    #[serde(default, deserialize_with = "de_f64")]
    pub confidence_score: f64,
}

fn de_vec_cli_arg<'de, D>(d: D) -> Result<Vec<CLIArgument>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(CLIArgument {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_cli_option<'de, D>(d: D) -> Result<Vec<CLIOption>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(CLIOption {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_cli_boundary<'de, D>(d: D) -> Result<Vec<CLIBoundary>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |command| {
        Some(CLIBoundary {
            command,
            ..Default::default()
        })
    })
}

fn de_vec_api_boundary<'de, D>(d: D) -> Result<Vec<APIBoundary>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |endpoint| {
        Some(APIBoundary {
            endpoint,
            ..Default::default()
        })
    })
}

fn de_vec_router_param<'de, D>(d: D) -> Result<Vec<RouterParam>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |key| {
        Some(RouterParam {
            key,
            ..Default::default()
        })
    })
}

fn de_vec_router_boundary<'de, D>(d: D) -> Result<Vec<RouterBoundary>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |path| {
        Some(RouterBoundary {
            path,
            ..Default::default()
        })
    })
}

fn de_vec_integration<'de, D>(d: D) -> Result<Vec<IntegrationSuggestion>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |description| {
        Some(IntegrationSuggestion {
            description,
            ..Default::default()
        })
    })
}

// ============================ database ============================

/// A database project (e.g. `.sqlproj`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DatabaseProject {
    /// Project name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Project file path.
    #[serde(default, deserialize_with = "de_string")]
    pub project_path: String,
    /// Target platform (SQL Server etc.).
    #[serde(default, deserialize_with = "de_opt_string")]
    pub target_platform: Option<String>,
    /// Table count.
    #[serde(default, deserialize_with = "de_usize")]
    pub table_count: usize,
    /// View count.
    #[serde(default, deserialize_with = "de_usize")]
    pub view_count: usize,
    /// Procedure count.
    #[serde(default, deserialize_with = "de_usize")]
    pub procedure_count: usize,
    /// Function count.
    #[serde(default, deserialize_with = "de_usize")]
    pub function_count: usize,
    /// Referenced projects/DACPACs.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub references: Vec<String>,
}

/// A table column.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct TableColumn {
    /// Column name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Data type.
    #[serde(default, deserialize_with = "de_string")]
    pub data_type: String,
    /// Allows NULL.
    #[serde(default, deserialize_with = "de_bool")]
    pub nullable: bool,
    /// Identity/auto-increment.
    #[serde(default, deserialize_with = "de_bool")]
    pub is_identity: bool,
    /// Default value.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub default_value: Option<String>,
}

/// A database table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DatabaseTable {
    /// Schema (e.g. dbo).
    #[serde(default, deserialize_with = "de_string")]
    pub schema: String,
    /// Table name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Columns.
    #[serde(default, deserialize_with = "de_vec_table_column")]
    pub columns: Vec<TableColumn>,
    /// Primary-key columns.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub primary_key: Vec<String>,
    /// Purpose.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Source file.
    #[serde(default, deserialize_with = "de_string")]
    pub source_path: String,
}

/// A database view.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DatabaseView {
    /// Schema.
    #[serde(default, deserialize_with = "de_string")]
    pub schema: String,
    /// View name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// What it shows.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Referenced tables.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub referenced_tables: Vec<String>,
    /// Source file.
    #[serde(default, deserialize_with = "de_string")]
    pub source_path: String,
}

/// Stored-procedure / function parameter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ProcedureParameter {
    /// Name (with @ where applicable).
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Data type.
    #[serde(default, deserialize_with = "de_string")]
    pub data_type: String,
    /// Optional?
    #[serde(default, deserialize_with = "de_bool")]
    pub is_optional: bool,
    /// INPUT | OUTPUT | INOUT.
    #[serde(default, deserialize_with = "de_string")]
    pub direction: String,
}

/// A stored procedure.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct StoredProcedure {
    /// Schema.
    #[serde(default, deserialize_with = "de_string")]
    pub schema: String,
    /// Procedure name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Parameters.
    #[serde(default, deserialize_with = "de_vec_proc_param")]
    pub parameters: Vec<ProcedureParameter>,
    /// What it does.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Tables touched.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub referenced_tables: Vec<String>,
    /// Source file.
    #[serde(default, deserialize_with = "de_string")]
    pub source_path: String,
}

/// A database function.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DatabaseFunction {
    /// Schema.
    #[serde(default, deserialize_with = "de_string")]
    pub schema: String,
    /// Function name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Scalar, table-valued…
    #[serde(default, deserialize_with = "de_string")]
    pub function_type: String,
    /// Parameters.
    #[serde(default, deserialize_with = "de_vec_proc_param")]
    pub parameters: Vec<ProcedureParameter>,
    /// Return type.
    #[serde(default, deserialize_with = "de_string")]
    pub return_type: String,
    /// What it computes.
    #[serde(default, deserialize_with = "de_string")]
    pub description: String,
    /// Source file.
    #[serde(default, deserialize_with = "de_string")]
    pub source_path: String,
}

/// Table relationship (FK, reference, implicit).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct TableRelationship {
    /// Source table (schema.table).
    #[serde(default, deserialize_with = "de_string")]
    pub from_table: String,
    /// Source columns.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub from_columns: Vec<String>,
    /// Target table.
    #[serde(default, deserialize_with = "de_string")]
    pub to_table: String,
    /// Target columns.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub to_columns: Vec<String>,
    /// ForeignKey | Reference | Implicit.
    #[serde(default, deserialize_with = "de_string")]
    pub relationship_type: String,
    /// FK constraint name if explicit.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub constraint_name: Option<String>,
}

/// A data movement pattern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DataFlow {
    /// Flow name/description.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Source (table, system, procedure).
    #[serde(default, deserialize_with = "de_string")]
    pub source: String,
    /// Destination.
    #[serde(default, deserialize_with = "de_string")]
    pub destination: String,
    /// INSERT/UPDATE/MERGE…
    #[serde(default, deserialize_with = "de_vec_string")]
    pub operations: Vec<String>,
    /// Procedures involved.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub procedures_involved: Vec<String>,
}

/// `database` agent output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct DatabaseOverviewReport {
    /// `.sqlproj`-style projects found.
    #[serde(default, deserialize_with = "de_vec_db_project")]
    pub database_projects: Vec<DatabaseProject>,
    /// All discovered tables.
    #[serde(default, deserialize_with = "de_vec_db_table")]
    pub tables: Vec<DatabaseTable>,
    /// All discovered views.
    #[serde(default, deserialize_with = "de_vec_db_view")]
    pub views: Vec<DatabaseView>,
    /// All discovered stored procedures.
    #[serde(default, deserialize_with = "de_vec_stored_proc")]
    pub stored_procedures: Vec<StoredProcedure>,
    /// All discovered functions.
    #[serde(default, deserialize_with = "de_vec_db_function")]
    pub database_functions: Vec<DatabaseFunction>,
    /// Table relationships.
    #[serde(default, deserialize_with = "de_vec_table_rel")]
    pub table_relationships: Vec<TableRelationship>,
    /// Data flows.
    #[serde(default, deserialize_with = "de_vec_data_flow")]
    pub data_flows: Vec<DataFlow>,
    /// 0–10 confidence.
    #[serde(default, deserialize_with = "de_f64")]
    pub confidence_score: f64,
}

fn de_vec_db_project<'de, D>(d: D) -> Result<Vec<DatabaseProject>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DatabaseProject {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_table_column<'de, D>(d: D) -> Result<Vec<TableColumn>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(TableColumn {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_db_table<'de, D>(d: D) -> Result<Vec<DatabaseTable>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DatabaseTable {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_db_view<'de, D>(d: D) -> Result<Vec<DatabaseView>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DatabaseView {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_proc_param<'de, D>(d: D) -> Result<Vec<ProcedureParameter>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(ProcedureParameter {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_stored_proc<'de, D>(d: D) -> Result<Vec<StoredProcedure>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(StoredProcedure {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_db_function<'de, D>(d: D) -> Result<Vec<DatabaseFunction>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DatabaseFunction {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_table_rel<'de, D>(d: D) -> Result<Vec<TableRelationship>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |_| None)
}

fn de_vec_data_flow<'de, D>(d: D) -> Result<Vec<DataFlow>, D::Error>
where
    D: Deserializer<'de>,
{
    de_vec_obj(d, |name| {
        Some(DataFlow {
            name,
            ..Default::default()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_context_lenient() {
        let payload = serde_json::json!({
            "project_name": "telemetry-processor",
            "project_type": "BackendService",
            "target_users": [{"name": "Platform engineer"}, "Operations team"],
            "external_systems": ["GeoIP db", {"name": "Redis"}],
            "system_boundary": "{\"scope\":\"ingest\",\"included_components\":\"reader\",\"excluded_components\":[\"ui\"]}",
            "confidence_score": "8.4"
        });
        let r: SystemContextReport = serde_json::from_value(payload).unwrap();
        assert_eq!(r.project_name, "telemetry-processor");
        assert_eq!(r.target_users.len(), 2);
        assert_eq!(r.external_systems.len(), 2);
        assert_eq!(r.system_boundary.included_components, vec!["reader"]);
        assert!((r.confidence_score - 8.4).abs() < 1e-9);
    }

    #[test]
    fn domain_modules_lenient_steps() {
        let payload = serde_json::json!({
            "domain_modules": [{"name": "Ingestion", "sub_modules": [{"name": "Reader"}]}],
            "business_flows": [{"name": "Tick", "steps": ["Load Config", {"step": "2", "operation": "Process"}]}],
            "confidence_score": "7.2"
        });
        let r: DomainModulesReport = serde_json::from_value(payload).unwrap();
        assert_eq!(r.domain_modules.len(), 1);
        assert_eq!(r.business_flows[0].steps.len(), 2);
        assert_eq!(r.business_flows[0].steps[1].step, 2);
    }

    #[test]
    fn boundary_lenient_shapes() {
        let payload = serde_json::json!({
            "cli_boundaries": [{"description": "main cli"}, "telemetry-processor"],
            "api_boundaries": [{"method": "GET"}, "/health"],
            "router_boundaries": [{"path": "/"}, "/status"],
            "integration_suggestions": ["Prefer idempotent operations"],
            "confidence_score": "7.0"
        });
        let r: BoundaryAnalysisReport = serde_json::from_value(payload).unwrap();
        assert_eq!(r.cli_boundaries.len(), 2);
        assert_eq!(r.api_boundaries.len(), 2);
        assert_eq!(r.router_boundaries.len(), 2);
        assert_eq!(r.integration_suggestions.len(), 1);
    }
}
