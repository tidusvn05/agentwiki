//! Code-insight types: what the `dir_summary` fan-out produces per directory.

use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lenient::{self, de_bool, de_f64, de_string, de_vec_obj, de_vec_string};

/// Code functionality classification, ported from deepwiki-rs `CodePurpose`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum CodePurpose {
    /// Project execution entry point.
    Entry,
    /// Intelligent agent.
    Agent,
    /// Frontend UI page.
    Page,
    /// Frontend UI component.
    Widget,
    /// Module implementing specific logical functionality.
    SpecificFeature,
    /// Data type or model.
    Model,
    /// Program-internal interface definitions.
    Types,
    /// Functional tool code for specific scenarios.
    Tool,
    /// Common low-level utility unrelated to business logic.
    Util,
    /// Configuration.
    Config,
    /// Middleware.
    Middleware,
    /// Plugin.
    Plugin,
    /// Router in a frontend or backend system.
    Router,
    /// Database component.
    Database,
    /// Service API for external calls (HTTP, RPC, IPC…).
    Api,
    /// MVC controller.
    Controller,
    /// MVC service / business rules.
    Service,
    /// Collection of related code with clear boundaries.
    Module,
    /// Dependency library.
    Lib,
    /// Test component.
    Test,
    /// Documentation component.
    Doc,
    /// Data access layer.
    Dao,
    /// Context component.
    Context,
    /// CLI command or message/request handler.
    Command,
    /// Uncategorized or unknown.
    #[default]
    Other,
}

impl CodePurpose {
    /// Map an arbitrary raw label onto the closest variant.
    pub fn map_from_raw(raw: &str) -> CodePurpose {
        let normalized = raw
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if normalized.is_empty() {
            return CodePurpose::Other;
        }
        if normalized.contains("specificfeature") || normalized == "feature" {
            return CodePurpose::SpecificFeature;
        }
        if normalized.contains("frontenduicomponent") || normalized == "widget" {
            return CodePurpose::Widget;
        }
        if normalized.contains("frontenduipage") || normalized == "page" {
            return CodePurpose::Page;
        }
        if normalized.contains("agent") {
            return CodePurpose::Agent;
        }
        if normalized.contains("entry") || normalized == "main" || normalized == "cli" {
            return CodePurpose::Entry;
        }
        if normalized.contains("model") || normalized.contains("entity") {
            return CodePurpose::Model;
        }
        if normalized.contains("type") || normalized.contains("interface") {
            return CodePurpose::Types;
        }
        if normalized.contains("tool") {
            return CodePurpose::Tool;
        }
        if normalized.contains("util") || normalized.contains("helper") {
            return CodePurpose::Util;
        }
        if normalized.contains("config") || normalized.contains("setting") {
            return CodePurpose::Config;
        }
        if normalized.contains("middleware") {
            return CodePurpose::Middleware;
        }
        if normalized.contains("plugin") || normalized.contains("extension") {
            return CodePurpose::Plugin;
        }
        if normalized.contains("router") || normalized.contains("route") {
            return CodePurpose::Router;
        }
        if normalized.contains("database") || normalized == "db" || normalized.contains("sql") {
            return CodePurpose::Database;
        }
        if normalized.contains("dao") || normalized.contains("repository") {
            return CodePurpose::Dao;
        }
        if normalized.contains("api") || normalized.contains("endpoint") {
            return CodePurpose::Api;
        }
        if normalized.contains("controller") {
            return CodePurpose::Controller;
        }
        if normalized.contains("service") {
            return CodePurpose::Service;
        }
        if normalized.contains("command") || normalized.contains("handler") {
            return CodePurpose::Command;
        }
        if normalized.contains("module") {
            return CodePurpose::Module;
        }
        if normalized.contains("lib") || normalized.contains("package") {
            return CodePurpose::Lib;
        }
        if normalized.contains("test") || normalized.contains("spec") {
            return CodePurpose::Test;
        }
        if normalized.contains("doc") {
            return CodePurpose::Doc;
        }
        if normalized.contains("context") {
            return CodePurpose::Context;
        }
        CodePurpose::Other
    }
}

impl Display for CodePurpose {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

fn de_code_purpose<'de, D>(deserializer: D) -> Result<CodePurpose, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(CodePurpose::map_from_raw(&lenient::json_value_to_string(value)))
}

/// Parameter of an interface/function.
#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
#[serde(default)]
pub struct ParameterInfo {
    /// Parameter name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Parameter type annotation.
    #[serde(default, deserialize_with = "de_string")]
    pub param_type: String,
}

/// Interface / function exposed by a file.
#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
#[serde(default)]
pub struct InterfaceInfo {
    /// Symbol name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Kind: function, method, class, trait…
    #[serde(default, deserialize_with = "de_string")]
    pub interface_type: String,
    /// Parameters.
    #[serde(default, deserialize_with = "de_vec_parameter_info")]
    pub parameters: Vec<ParameterInfo>,
    /// Return type if known.
    #[serde(default, deserialize_with = "lenient::de_opt_string")]
    pub return_type: Option<String>,
}

fn de_vec_parameter_info<'de, D>(deserializer: D) -> Result<Vec<ParameterInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|v| {
                let obj = v.as_object()?;
                Some(ParameterInfo {
                    name: obj
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    param_type: obj
                        .get("param_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

/// Dependency of a file.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct Dependency {
    /// Module/crate/package name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Whether it is external to the repo.
    #[serde(default, deserialize_with = "de_bool")]
    pub is_external: bool,
    /// import | use | include | require …
    #[serde(default, deserialize_with = "de_string")]
    pub dependency_type: String,
}

fn de_vec_dependency<'de, D>(deserializer: D) -> Result<Vec<Dependency>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    de_vec_obj(deserializer, |name| {
        Some(Dependency {
            name,
            ..Default::default()
        })
    })
}

fn de_vec_interface<'de, D>(deserializer: D) -> Result<Vec<InterfaceInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    de_vec_obj(deserializer, |name| {
        Some(InterfaceInfo {
            name,
            ..Default::default()
        })
    })
}

/// Per-file insight (LLM-produced, inside a directory dossier).
#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
#[serde(default)]
pub struct FileInsight {
    /// File name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Repo-relative path (filled in by the runner, not the model).
    pub file_path: PathBuf,
    /// 1–2 sentence description.
    #[serde(default, deserialize_with = "de_string")]
    pub summary: String,
    /// Coarse purpose classification.
    #[serde(default, deserialize_with = "de_code_purpose")]
    pub code_purpose: CodePurpose,
    /// Short summary of the source itself.
    #[serde(default, deserialize_with = "de_string")]
    pub source_summary: String,
    /// Longer description of the file's role.
    #[serde(default, deserialize_with = "de_string")]
    pub detailed_description: String,
    /// Key responsibilities.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub responsibilities: Vec<String>,
    /// Exposed interfaces/functions.
    #[serde(default, deserialize_with = "de_vec_interface")]
    pub interfaces: Vec<InterfaceInfo>,
    /// Key dependencies.
    #[serde(default, deserialize_with = "de_vec_dependency")]
    pub dependencies: Vec<Dependency>,
    /// 0.0–1.0 importance.
    #[serde(default, deserialize_with = "de_f64")]
    pub importance_score: f64,
}

/// Directory purpose classification (deterministic, from name).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DirectoryPurpose {
    /// Core business logic.
    Core,
    /// Configuration.
    Config,
    /// API/interface layer.
    Api,
    /// Data layer.
    Database,
    /// Frontend.
    Frontend,
    /// Tests.
    Test,
    /// Tooling/scripts.
    Tool,
    /// Documentation.
    Docs,
    /// Anything else.
    #[default]
    Other,
}

/// Classify a directory's purpose from its name (ported heuristic).
pub fn classify_directory_purpose(name: &str) -> DirectoryPurpose {
    let n = name.to_lowercase();
    if n == "src"
        || n == "lib"
        || n == "internal"
        || n == "core"
        || n == "pkg"
        || n == "cmd"
        || n == "main"
        || n == "entry"
    {
        DirectoryPurpose::Core
    } else if n.contains("config") || n == "conf" || n == "cfg" {
        DirectoryPurpose::Config
    } else if n.contains("api") || n == "rpc" || n == "grpc" {
        DirectoryPurpose::Api
    } else if n.contains("database")
        || n == "db"
        || n.contains("migrations")
        || n.contains("schema")
    {
        DirectoryPurpose::Database
    } else if n == "frontend" || n == "ui" || n == "web" || n == "client" || n == "views" {
        DirectoryPurpose::Frontend
    } else if n == "test" || n == "tests" || n == "spec" || n == "__tests__" {
        DirectoryPurpose::Test
    } else if n == "scripts" || n == "tools" || n == "bin" {
        DirectoryPurpose::Tool
    } else if n == "docs" || n == "doc" || n == "documentation" {
        DirectoryPurpose::Docs
    } else {
        DirectoryPurpose::Other
    }
}

/// LLM output of `dir_summary` for one directory.
#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
#[serde(default)]
pub struct DirectorySummaryResponse {
    /// 2–3 sentence description of the directory's role.
    #[serde(default, deserialize_with = "de_string")]
    pub summary: String,
    /// 0.0–1.0 directory importance.
    #[serde(default, deserialize_with = "de_f64")]
    pub importance_score: f64,
    /// Up-to-5 most important file names.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub key_files: Vec<String>,
    /// Per-file insights.
    #[serde(default, deserialize_with = "de_vec_file_insight")]
    pub file_insights: Vec<FileInsight>,
}

fn de_vec_file_insight<'de, D>(deserializer: D) -> Result<Vec<FileInsight>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    de_vec_obj(deserializer, |name| {
        Some(FileInsight {
            name,
            ..Default::default()
        })
    })
}

/// A directory's dossier = LLM summary + deterministic metadata.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct DirectoryDossier {
    /// Absolute path.
    pub path: PathBuf,
    /// Directory name.
    pub name: String,
    /// Purpose classification.
    pub purpose: DirectoryPurpose,
    /// Files directly inside.
    pub file_count: usize,
    /// Direct subdirectories.
    pub subdirectory_count: usize,
    /// LLM-assigned importance 0.0–1.0.
    pub importance_score: f64,
    /// LLM summary.
    pub summary: String,
    /// LLM-chosen key file names.
    pub key_files: Vec<String>,
    /// Per-file insights.
    pub file_insights: Vec<FileInsight>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_purpose_maps_messy_labels() {
        assert_eq!(CodePurpose::map_from_raw("Entry"), CodePurpose::Entry);
        assert_eq!(CodePurpose::map_from_raw("entry point"), CodePurpose::Entry);
        assert_eq!(CodePurpose::map_from_raw("SpecificFeature"), CodePurpose::SpecificFeature);
        assert_eq!(CodePurpose::map_from_raw("???"), CodePurpose::Other);
    }

    #[test]
    fn dir_summary_lenient_parse() {
        let v = serde_json::json!({
            "summary": {"text": "core logic"},
            "importance_score": "0.8",
            "key_files": "main.rs",
            "file_insights": [{"name": "a.rs", "code_purpose": "entry", "importance_score": "1"}]
        });
        let r: DirectorySummaryResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.summary, "core logic");
        assert!((r.importance_score - 0.8).abs() < 1e-9);
        assert_eq!(r.key_files, vec!["main.rs"]);
        assert_eq!(r.file_insights[0].code_purpose, CodePurpose::Entry);
    }
}
