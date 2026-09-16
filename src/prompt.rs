//! Prompt templates: `{{key}}` rendering + disk-override loading.
//!
//! Template files live under `prompts/`. A file in `config.prompts_dir`
//! overrides the embedded copy; otherwise `include_str!` provides the
//! default so the binary is self-contained.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{Error, Result};

/// Render `{{key}}` placeholders. Unknown placeholders are left in place so
/// JSON examples inside templates (`{{"a": 1}}`-style braces) survive.
pub fn render(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

/// Loads prompt templates, preferring `prompts_dir` on disk.
pub struct PromptLoader {
    dir: Option<PathBuf>,
}

macro_rules! embedded {
    ($($name:literal => $path:literal),* $(,)?) => {
        fn embedded_template(name: &str) -> Option<&'static str> {
            match name {
                $($name => Some(include_str!($path)),)*
                _ => None,
            }
        }
    };
}

embedded! {
    "dir_summary.md" => "../prompts/dir_summary.md",
    "relationships.md" => "../prompts/relationships.md",
    "system_context.md" => "../prompts/system_context.md",
    "domain_modules.md" => "../prompts/domain_modules.md",
    "architecture.md" => "../prompts/architecture.md",
    "workflow.md" => "../prompts/workflow.md",
    "key_module.md" => "../prompts/key_module.md",
    "boundary.md" => "../prompts/boundary.md",
    "database.md" => "../prompts/database.md",
    "editors/overview.md" => "../prompts/editors/overview.md",
    "editors/architecture_doc.md" => "../prompts/editors/architecture_doc.md",
    "editors/workflow_doc.md" => "../prompts/editors/workflow_doc.md",
    "editors/deep_dive.md" => "../prompts/editors/deep_dive.md",
}

impl PromptLoader {
    /// `dir` = `config.prompts_dir` (disk overrides), may be `None`.
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    /// Load a template by relative name (`"system_context.md"`).
    pub fn load(&self, name: &str) -> Result<String> {
        if let Some(dir) = &self.dir {
            let path = dir.join(name);
            if path.is_file() {
                return std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e));
            }
        }
        embedded_template(name)
            .map(str::to_string)
            .ok_or_else(|| Error::Prompt {
                name: name.to_string(),
                message: "unknown template".to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_replaces_known_leaves_unknown() {
        let mut vars = HashMap::new();
        vars.insert("a", "X".to_string());
        let out = render("{{a}} and {{b}} and {{\"q\":1}}", &vars);
        assert_eq!(out, "X and {{b}} and {{\"q\":1}}");
    }
}
