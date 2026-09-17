//! Heuristic per-file code insights: interfaces, dependencies, metrics.
//!
//! Regex-based, language-aware extraction — a lightweight stand-in for
//! deepwiki-rs's `language_processors`. Good enough to seed `dir_summary`
//! prompts with symbol names without an AST.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Static metrics for one file.
#[derive(Debug, Clone, Default)]
pub struct FileMetrics {
    /// Non-empty line count.
    pub lines_of_code: usize,
    /// Detected function/method count.
    pub number_of_functions: usize,
    /// Detected type/class count.
    pub number_of_classes: usize,
    /// Branch-keyword count (if/for/while/match/case…).
    pub cyclomatic_complexity: f64,
}

/// One extracted symbol (function, class, …).
#[derive(Debug, Clone)]
pub struct ExtractedInterface {
    /// Symbol name.
    pub name: String,
    /// Kind label shown to the model.
    pub interface_type: String,
    /// `name(a: T, b: U) -> R` style signature, single line.
    pub signature: String,
}

/// One extracted dependency.
#[derive(Debug, Clone)]
pub struct ExtractedDependency {
    /// Module/package name.
    pub name: String,
    /// import | use | include | require.
    pub dependency_type: String,
    /// External to the repo (heuristic: not a relative/self path).
    pub is_external: bool,
}

/// Everything we can cheaply infer about one file.
#[derive(Debug, Clone, Default)]
pub struct FileStatics {
    /// Extracted symbols.
    pub interfaces: Vec<ExtractedInterface>,
    /// Extracted imports.
    pub dependencies: Vec<ExtractedDependency>,
    /// Metrics.
    pub metrics: FileMetrics,
}

struct LangRules {
    funcs: &'static [&'static str],
    types: &'static [&'static str],
    imports: &'static [&'static str],
    branches: &'static [&'static str],
}

fn rules_for(ext: &str) -> LangRules {
    match ext {
        "rs" => LangRules {
            funcs: &[r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)"],
            types: &[
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([A-Za-z0-9_]+)",
            ],
            imports: &[r"(?m)^\s*use\s+([A-Za-z0-9_:]+)"],
            branches: &["if ", "else if", "for ", "while ", "loop ", "match ", "?"],
        },
        "py" | "pyw" => LangRules {
            funcs: &[
                r"(?m)^\s*(?:async\s+)?def\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*([A-Za-z0-9_]+)\s*=\s*lambda",
            ],
            types: &[r"(?m)^\s*class\s+([A-Za-z0-9_]+)"],
            imports: &[r"(?m)^\s*(?:from\s+([A-Za-z0-9_.]+)\s+)?import\s+([A-Za-z0-9_.*, ]+)"],
            branches: &["if ", "elif ", "for ", "while ", "except", "with "],
        },
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => LangRules {
            funcs: &[
                r"(?m)^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z0-9_]+)\s*=\s*(?:async\s+)?(?:\(|function)",
                r"(?m)^\s*(?:public|private|protected|static|async|\s)*\s*([A-Za-z0-9_]+)\s*\([^)]*\)\s*[:\w\[\]<>| ]*\{",
            ],
            types: &[
                r"(?m)^\s*(?:export\s+)?(?:abstract\s+)?class\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*(?:export\s+)?interface\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*(?:export\s+)?(?:type|enum)\s+([A-Za-z0-9_]+)",
            ],
            imports: &[
                r#"(?m)import\s+.*?from\s+['"]([^'"]+)['"]"#,
                r#"(?m)require\(\s*['"]([^'"]+)['"]\s*\)"#,
            ],
            branches: &[
                "if (", "if(", "for (", "for(", "while ", "case ", "catch", "?",
            ],
        },
        "go" => LangRules {
            funcs: &[r"(?m)^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z0-9_]+)"],
            types: &[r"(?m)^\s*type\s+([A-Za-z0-9_]+)\s+(?:struct|interface)"],
            imports: &[
                r#"(?m)^\s*import\s+?(?:\(\s*)?["`]([^"`]+)["`]"#,
                r#"(?m)^\s*["`]([a-z0-9_./~-]+)["`]\s*$"#,
            ],
            branches: &["if ", "for ", "switch ", "case ", "select "],
        },
        "java" | "kt" | "scala" => LangRules {
            funcs: &[
                r"(?m)^\s*(?:public|private|protected|static|final|suspend|override|\s)*\s*(?:fun\s+)?([A-Za-z0-9_]+)\s*\(",
            ],
            types: &[
                r"(?m)^\s*(?:public|private|abstract|final|data|sealed|\s)*\s*(?:class|interface|enum|object)\s+([A-Za-z0-9_]+)",
            ],
            imports: &[r"(?m)^\s*import\s+([A-Za-z0-9_.*]+)"],
            branches: &["if (", "for (", "while (", "case ", "catch", "when ("],
        },
        "c" | "h" | "cpp" | "cc" | "hpp" | "cs" => LangRules {
            funcs: &[
                r"(?m)^\s*(?:public|private|protected|static|virtual|async|internal|inline|\s)*\s*[A-Za-z0-9_<>\[\],:*& ]+\s+([A-Za-z0-9_]+)\s*\(",
            ],
            types: &[
                r"(?m)^\s*(?:public|private|abstract|sealed|static|partial|\s)*\s*(?:class|struct|enum|interface|record)\s+([A-Za-z0-9_]+)",
            ],
            imports: &[
                r#"(?m)^\s*#include\s+[<"]([^>"]+)[>"]"#,
                r"(?m)^\s*using\s+([A-Za-z0-9_.]+)",
            ],
            branches: &["if (", "for (", "while (", "case ", "catch", "switch ("],
        },
        "rb" | "php" | "swift" | "dart" | "m" | "sh" | "sql" => LangRules {
            funcs: &[
                r"(?m)^\s*def\s+([A-Za-z0-9_!?]+)",
                r"(?m)^\s*(?:public\s+)?(?:static\s+)?function\s+([A-Za-z0-9_]+)",
                r"(?m)^\s*func\s+([A-Za-z0-9_]+)",
            ],
            types: &[
                r"(?m)^\s*(?:class|module|struct|enum|protocol|extension)\s+([A-Za-z0-9_]+)",
                r"(?im)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:TABLE|VIEW|PROCEDURE|FUNCTION)\s+([\w.\[\]]+)",
            ],
            imports: &[
                r#"(?m)^\s*(?:require|require_relative|include|use|using)\s+['"]?([A-Za-z0-9_./:]+)"#,
            ],
            branches: &["if ", "elsif", "for ", "while ", "case ", "when ", "rescue"],
        },
        _ => LangRules {
            funcs: &[],
            types: &[],
            imports: &[],
            branches: &["if ", "for ", "while ", "case "],
        },
    }
}

static CAP_CACHE: LazyLock<std::sync::Mutex<std::collections::HashMap<String, Regex>>> =
    LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn cached_regex(pat: &str) -> Option<Regex> {
    let mut cache = CAP_CACHE.lock().ok()?;
    if let Some(r) = cache.get(pat) {
        return Some(r.clone());
    }
    let r = Regex::new(pat).ok()?;
    cache.insert(pat.to_string(), r.clone());
    Some(r)
}

/// Extract statics for one file's content.
pub fn extract(path: &Path, content: &str) -> FileStatics {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let rules = rules_for(&ext);

    let mut out = FileStatics::default();
    out.metrics.lines_of_code = content.lines().filter(|l| !l.trim().is_empty()).count();

    let mut seen = std::collections::HashSet::new();
    for pat in rules.funcs {
        if let Some(re) = cached_regex(pat) {
            for cap in re.captures_iter(content).take(60) {
                if let Some(m) = cap.get(1).or_else(|| cap.get(0)) {
                    let name = m.as_str().to_string();
                    if seen.insert(format!("f:{name}")) && name.len() > 1 {
                        out.metrics.number_of_functions += 1;
                        out.interfaces.push(ExtractedInterface {
                            signature: signature_line(content, m.start()),
                            name,
                            interface_type: "function".to_string(),
                        });
                    }
                }
            }
        }
    }
    for pat in rules.types {
        if let Some(re) = cached_regex(pat) {
            for cap in re.captures_iter(content).take(40) {
                if let Some(m) = cap.get(1) {
                    let name = m.as_str().to_string();
                    if seen.insert(format!("t:{name}")) {
                        out.metrics.number_of_classes += 1;
                        out.interfaces.push(ExtractedInterface {
                            signature: signature_line(content, m.start()),
                            name,
                            interface_type: "type".to_string(),
                        });
                    }
                }
            }
        }
    }
    for pat in rules.imports {
        if let Some(re) = cached_regex(pat) {
            for cap in re.captures_iter(content).take(40) {
                // `from X import a, b` → module is group 1; plain `import X` → group 1 or 2.
                let name = cap
                    .get(1)
                    .or_else(|| cap.get(2))
                    .map(|m| m.as_str().to_string());
                if let Some(name) = name {
                    let name = name.trim_end_matches(".*").to_string();
                    if !name.is_empty() && seen.insert(format!("d:{name}")) {
                        let is_external = !(name.starts_with('.')
                            || name.starts_with("crate")
                            || name.starts_with("self")
                            || name.starts_with("super"));
                        out.dependencies.push(ExtractedDependency {
                            name,
                            dependency_type: "import".to_string(),
                            is_external,
                        });
                    }
                }
            }
        }
    }

    out.metrics.cyclomatic_complexity = rules
        .branches
        .iter()
        .map(|b| content.matches(b).count() as f64)
        .sum();

    out
}

/// Extract the first line containing byte offset `pos` (best effort).
fn signature_line(content: &str, pos: usize) -> String {
    let start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = content[pos..]
        .find('\n')
        .map(|i| pos + i)
        .unwrap_or(content.len());
    content[start..end].trim().chars().take(160).collect()
}
