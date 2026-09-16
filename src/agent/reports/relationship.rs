//! Relationship-analysis report types (the `relationships` agent output).

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::lenient::{self, de_opt_string, de_string, de_u8, de_vec_string};

fn de_dependency_type<'de, D>(deserializer: D) -> Result<DependencyType, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(DependencyType::map_from_raw(&lenient::any_to_string(value)))
}

/// Dependency type between two components.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
pub enum DependencyType {
    /// `use` / `import` statements.
    Import,
    /// Function-call dependency.
    FunctionCall,
    /// Inheritance.
    Inheritance,
    /// Composition.
    Composition,
    /// Data-flow dependency.
    DataFlow,
    /// Generic module dependency.
    #[default]
    Module,
}

impl DependencyType {
    /// Lenient raw-string → variant mapping.
    pub fn map_from_raw(raw: &str) -> Self {
        let normalized = raw.trim().to_lowercase();
        if normalized.contains("import") || normalized == "use" {
            return Self::Import;
        }
        if normalized.contains("function") || normalized.contains("call") {
            return Self::FunctionCall;
        }
        if normalized.contains("inherit") || normalized.contains("extend") {
            return Self::Inheritance;
        }
        if normalized.contains("composition") || normalized.contains("compose") {
            return Self::Composition;
        }
        if normalized.contains("data") && normalized.contains("flow") {
            return Self::DataFlow;
        }
        Self::Module
    }

    /// Stable lowercase label used in prompts and reports.
    pub fn as_str(&self) -> &'static str {
        match self {
            DependencyType::Import => "import",
            DependencyType::FunctionCall => "function_call",
            DependencyType::Inheritance => "inheritance",
            DependencyType::Composition => "composition",
            DependencyType::DataFlow => "data_flow",
            DependencyType::Module => "module",
        }
    }
}

/// One directed dependency edge.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
#[serde(default)]
pub struct CoreDependency {
    /// Source component.
    #[serde(default, deserialize_with = "de_string")]
    pub from: String,
    /// Target component.
    #[serde(default, deserialize_with = "de_string")]
    pub to: String,
    /// Edge kind.
    #[serde(default, deserialize_with = "de_dependency_type")]
    pub dependency_type: DependencyType,
    /// 1–5 importance.
    #[serde(default, deserialize_with = "de_u8")]
    pub importance: u8,
    /// Optional description.
    #[serde(default, deserialize_with = "de_opt_string")]
    pub description: Option<String>,
}

/// One architecture layer.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
#[serde(default)]
pub struct ArchitectureLayer {
    /// Layer name.
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    /// Components in the layer.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub components: Vec<String>,
    /// Level (lower = deeper in the stack).
    #[serde(default, deserialize_with = "de_u8")]
    pub level: u8,
}

fn de_vec_core_dep<'de, D>(deserializer: D) -> Result<Vec<CoreDependency>, D::Error>
where
    D: Deserializer<'de>,
{
    lenient::de_vec_obj(deserializer, |_| None)
}

fn de_vec_layer<'de, D>(deserializer: D) -> Result<Vec<ArchitectureLayer>, D::Error>
where
    D: Deserializer<'de>,
{
    lenient::de_vec_obj(deserializer, |_| None)
}

/// Relationship analysis result (project-level dependency graph).
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema, Default)]
#[serde(default)]
pub struct RelationshipAnalysis {
    /// Important dependency edges.
    #[serde(default, deserialize_with = "de_vec_core_dep")]
    pub core_dependencies: Vec<CoreDependency>,
    /// Architecture layers, low → high.
    #[serde(default, deserialize_with = "de_vec_layer")]
    pub architecture_layers: Vec<ArchitectureLayer>,
    /// Free-form key insights / issues.
    #[serde(default, deserialize_with = "de_vec_string")]
    pub key_insights: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_parse_mixed_types() {
        let payload = serde_json::json!({
            "core_dependencies": [
                {"from": {"module": "reader"}, "to": "cache", "dependency_type": "function call", "importance": "4"},
                "invalid-entry"
            ],
            "architecture_layers": [{"name": 101, "components": "reader", "level": "2"}],
            "key_insights": ["good", {"note": "check cycle"}]
        });
        let parsed: RelationshipAnalysis = serde_json::from_value(payload).unwrap();
        assert_eq!(parsed.core_dependencies.len(), 1);
        assert_eq!(parsed.core_dependencies[0].dependency_type.as_str(), "function_call");
        assert_eq!(parsed.architecture_layers[0].level, 2);
        assert_eq!(parsed.key_insights.len(), 2);
    }
}
