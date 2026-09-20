//! Python import extraction: `from .x import y`, `from ..p import z`,
//! `from . import x`, plain `import a.b`, parenthesized name lists.
//!
//! `from`-imports inside `__init__.py` are re-exports (facade pattern).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::{EvidenceKind, ImportSpec, RawImport};

static RE_FROM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*from\s+(\.*)([A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)*)\s+import\s+").unwrap()
});
static RE_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*import\s+([A-Za-z0-9_.]+(?:\s*,\s*[A-Za-z0-9_.]+)*)").unwrap()
});

/// Extract raw imports from sanitized Python source.
pub fn extract(src: &str, rel_path: &Path, test_file: bool) -> Vec<RawImport> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let is_init = rel_path.file_name().is_some_and(|n| n == "__init__.py");
    let line_at = |pos: usize| -> u32 { (src[..pos].matches('\n').count() + 1) as u32 };
    let kind = if is_init {
        EvidenceKind::ReExport
    } else {
        EvidenceKind::Import
    };

    // `from <dots><module> import <names>` — names may be a parenthesized
    // multi-line list.
    for m in RE_FROM.captures_iter(src) {
        let level = m.get(1).unwrap().as_str().len() as u32;
        let module: Vec<String> = m
            .get(2)
            .map(|g| {
                g.as_str()
                    .split('.')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        // Names: up to EOL, or the `(...)` group.
        let mut j = m.get(0).unwrap().end();
        let names_text = if j < b.len() && b[j] == b'(' {
            let mut depth = 0i32;
            let start = j;
            while j < b.len() {
                match b[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            &src[start + 1..j.min(b.len())]
        } else {
            let start = j;
            while j < b.len() && b[j] != b'\n' && b[j] != b'\\' {
                j += 1;
            }
            if j < b.len() && b[j] == b'\\' {
                j += 2; // line continuation — keep scanning one more line
                while j < b.len() && b[j] != b'\n' {
                    j += 1;
                }
            }
            &src[start..j]
        };
        let names: Vec<String> = names_text
            .split(',')
            .map(|n| n.split_whitespace().next().unwrap_or(""))
            .filter(|n| {
                !n.is_empty()
                    && n.chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic() || c == '_')
            })
            .map(|n| n.to_string())
            .collect();
        out.push(RawImport {
            spec: ImportSpec::Python {
                level,
                module,
                names,
            },
            kind,
            test_only: test_file,
            line: line_at(m.get(0).unwrap().start()),
        });
    }

    // `import a.b[, c.d]` — absolute only.
    for m in RE_IMPORT.captures_iter(src) {
        for name in m.get(1).unwrap().as_str().split(',') {
            let module: Vec<String> = name
                .trim()
                .split('.')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if module.is_empty() {
                continue;
            }
            out.push(RawImport {
                spec: ImportSpec::Python {
                    level: 0,
                    module,
                    names: vec![],
                },
                kind: EvidenceKind::Import,
                test_only: test_file,
                line: line_at(m.get(0).unwrap().start()),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(src: &str, init: bool) -> Vec<RawImport> {
        let name = if init { "__init__.py" } else { "m.py" };
        extract(
            &crate::drift::imports::sanitize::sanitize(super::super::Lang::Python, src),
            Path::new(name),
            false,
        )
    }

    #[test]
    fn relative_and_absolute() {
        let out = specs(
            "from .models import Task\nfrom ..pkg import x\nfrom . import helper\nimport os.path\n",
            false,
        );
        match &out[0].spec {
            ImportSpec::Python {
                level,
                module,
                names,
            } => {
                assert_eq!(*level, 1);
                assert_eq!(module, &["models"]);
                assert_eq!(names, &["Task"]);
            }
            other => panic!("{other:?}"),
        }
        match &out[1].spec {
            ImportSpec::Python { level, module, .. } => {
                assert_eq!(*level, 2);
                assert_eq!(module, &["pkg"]);
            }
            other => panic!("{other:?}"),
        }
        match &out[2].spec {
            ImportSpec::Python {
                level,
                module,
                names,
            } => {
                assert_eq!(*level, 1);
                assert!(module.is_empty());
                assert_eq!(names, &["helper"]);
            }
            other => panic!("{other:?}"),
        }
        match &out[3].spec {
            ImportSpec::Python { level, module, .. } => {
                assert_eq!(*level, 0);
                assert_eq!(module, &["os", "path"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parenthesized_names_and_as() {
        let out = specs(
            "from pkg.sub import (\n    Alpha,\n    Beta as B,\n)\n",
            false,
        );
        match &out[0].spec {
            ImportSpec::Python { module, names, .. } => {
                assert_eq!(module, &["pkg", "sub"]);
                assert_eq!(names, &["Alpha", "Beta"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn init_py_is_reexport() {
        let out = specs("from .a import b\n", true);
        assert_eq!(out[0].kind, EvidenceKind::ReExport);
    }
}
