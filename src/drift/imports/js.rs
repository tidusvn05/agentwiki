//! JS/TS import extraction: `import … from`, `import 'x'`,
//! `export … from` (re-export), `require('x')`, `import('x')`.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::{EvidenceKind, ImportSpec, RawImport};

static RE_PATTERNS: LazyLock<Vec<(Regex, EvidenceKind)>> = LazyLock::new(|| {
    [
        // `import x, {y} from '…'` and bare `import '…'`.
        (
            Regex::new(r#"\bimport\s+(?:[A-Za-z0-9_{}*,\s]+\s+from\s+)?['"]([^'"]+)['"]"#).unwrap(),
            EvidenceKind::Import,
        ),
        // `export {x} from '…'` / `export * from '…'`.
        (
            Regex::new(r#"\bexport\s+[A-Za-z0-9_{}*,\s]+\s+from\s+['"]([^'"]+)['"]"#).unwrap(),
            EvidenceKind::ReExport,
        ),
        // `require('…')`.
        (
            Regex::new(r#"\brequire\s*\(\s*['"]([^'"]+)['"]"#).unwrap(),
            EvidenceKind::Import,
        ),
        // `import('…')` dynamic.
        (
            Regex::new(r#"\bimport\s*\(\s*['"]([^'"]+)['"]"#).unwrap(),
            EvidenceKind::Import,
        ),
    ]
    .into_iter()
    .collect()
});

/// Extract raw imports from sanitized JS/TS source.
pub fn extract(src: &str, _rel_path: &Path, test_file: bool) -> Vec<RawImport> {
    let line_at = |pos: usize| -> u32 { (src[..pos].matches('\n').count() + 1) as u32 };
    let mut out = Vec::new();
    for (re, kind) in RE_PATTERNS.iter() {
        for m in re.captures_iter(src) {
            out.push(RawImport {
                spec: ImportSpec::Js {
                    specifier: m.get(1).unwrap().as_str().to_string(),
                },
                kind: *kind,
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

    fn specs(src: &str) -> Vec<RawImport> {
        extract(
            &crate::drift::imports::sanitize::sanitize(super::super::Lang::Js, src),
            Path::new("a.ts"),
            false,
        )
    }

    #[test]
    fn all_forms() {
        let out = specs(
            "import x from './a';\nimport './side';\nexport { y } from './b';\nconst r = require('./c');\nconst d = import('./d');\n",
        );
        let got: Vec<(&str, EvidenceKind)> = out
            .iter()
            .map(|i| {
                (
                    match &i.spec {
                        ImportSpec::Js { specifier } => specifier.as_str(),
                        _ => "?",
                    },
                    i.kind,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("./a", EvidenceKind::Import),
                ("./side", EvidenceKind::Import),
                ("./b", EvidenceKind::ReExport),
                ("./c", EvidenceKind::Import),
                ("./d", EvidenceKind::Import),
            ]
        );
    }
}
