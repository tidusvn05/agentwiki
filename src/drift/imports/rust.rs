//! Rust import extraction: `use` trees (nested braces, `as`, `self`,
//! `*`), inline qualified paths (`crate::x::y` in expressions — real
//! edges may exist only in that form), `mod x;` declarations, and
//! `mod x { … }` inline-module tracking for `super::` resolution.
//!
//! Input is comment/string-sanitized source, so offsets are byte-exact.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::sanitize::blank;
use super::{EvidenceKind, ImportSpec, RawImport, RustRoot};

static RE_MOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*([{;])").unwrap());
static RE_CFG_TEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]").unwrap());
static RE_USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:pub(?:\s*\([^)]*\))?\s+)?use\s+").unwrap());
static RE_IDENT_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:::[A-Za-z_][A-Za-z0-9_]*)+").unwrap());

/// One `mod name { … }` inline region (byte range of the body).
#[derive(Debug)]
struct InlineMod {
    name: String,
    open: usize,
    close: usize,
}

/// Extract imports from sanitized Rust source.
///
/// `crate_name` recognizes `use <crate>::…` / `x = <crate>::…` self
/// references (integration tests import the lib by package name).
/// `cfg_test` marks `#[cfg(test)]` imports as test-only.
pub fn extract(
    src: &str,
    _rel_path: &Path,
    test_file: bool,
    crate_name: Option<&str>,
    cfg_test: bool,
) -> Vec<RawImport> {
    let b = src.as_bytes();
    let mut out: Vec<RawImport> = Vec::new();

    // `mod x;` decls + `mod x {` brace positions.
    let mut decls: Vec<(String, usize)> = Vec::new();
    let mut mod_braces: std::collections::BTreeMap<usize, String> =
        std::collections::BTreeMap::new();
    for m in RE_MOD.captures_iter(src) {
        let name = m.get(1).unwrap().as_str().to_string();
        if m.get(2).unwrap().as_str() == ";" {
            decls.push((name, m.get(0).unwrap().start()));
        } else {
            mod_braces.insert(m.get(0).unwrap().end() - 1, name);
        }
    }
    // Close each `mod {` body by brace matching.
    let mut mods: Vec<InlineMod> = Vec::new();
    let mut stack: Vec<(usize, Option<String>)> = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' => stack.push((i, mod_braces.remove(&i))),
            b'}' => {
                if let Some((open, Some(name))) = stack.pop() {
                    mods.push(InlineMod {
                        name,
                        open,
                        close: i,
                    });
                }
            }
            _ => {}
        }
    }
    mods.sort_by_key(|m| m.open);

    // `#[cfg(test)]` + ws/attrs + `mod … {` → that body is a test region.
    let mut test_regions: Vec<(usize, usize)> = Vec::new();
    for cm in RE_CFG_TEST.find_iter(src) {
        let mut j = cm.end();
        loop {
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b'#' {
                match src[j..].find(']') {
                    Some(end) => {
                        j += end + 1;
                        continue;
                    }
                    None => break,
                }
            }
            break;
        }
        if let Some(mm) = RE_MOD.captures(&src[j..])
            && mm.get(0).unwrap().start() == 0
            && mm.get(2).unwrap().as_str() == "{"
        {
            let open = j + mm.get(0).unwrap().end() - 1;
            if let Some(m) = mods.iter().find(|m| m.open == open) {
                test_regions.push((m.open, m.close));
            }
        }
    }

    let in_test = |pos: usize| cfg_test && test_regions.iter().any(|(s, e)| pos > *s && pos < *e);
    let inline_at = |pos: usize| -> Vec<String> {
        mods.iter()
            .filter(|m| pos > m.open && pos < m.close)
            .map(|m| m.name.clone())
            .collect()
    };
    let line_at = |pos: usize| -> u32 { (src[..pos].matches('\n').count() + 1) as u32 };
    // `#[cfg(test)] use …` — attr between the previous `;`/`{`/`}` and `use`.
    let preceded_by_cfg = |pos: usize| -> bool {
        if !cfg_test {
            return false;
        }
        let seg_start = src[..pos]
            .rfind([';', '{', '}'])
            .map(|i| i + 1)
            .unwrap_or(0);
        RE_CFG_TEST.is_match(&src[seg_start..pos])
    };

    // `use …;` statements.
    let mut use_ranges: Vec<(usize, usize)> = Vec::new();
    for m in RE_USE.find_iter(src) {
        let is_pub = m.as_str().trim_start().starts_with("pub");
        // Capture until `;` outside any bracket.
        let mut j = m.end();
        let mut depth = 0i32;
        while j < b.len() {
            match b[j] {
                b'{' | b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => depth -= 1,
                b';' if depth <= 0 => break,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            continue; // unterminated — bail
        }
        use_ranges.push((m.start(), j + 1));
        let kind = if is_pub {
            EvidenceKind::ReExport
        } else {
            EvidenceKind::Import
        };
        let test = test_file || in_test(m.start()) || preceded_by_cfg(m.start());
        let line = line_at(m.start());
        let inline = inline_at(m.start());
        for path in expand_use_tree(&src[m.end()..j]) {
            if let Some(spec) = path_spec(&path, &inline) {
                out.push(RawImport {
                    spec,
                    kind,
                    test_only: test,
                    line,
                });
            }
        }
    }

    // Blank `use` statements, then match inline `a::b::c` paths.
    let mut rest = b.to_vec();
    for (s, e) in &use_ranges {
        blank(&mut rest, *s, *e);
    }
    let rest = String::from_utf8_lossy(&rest).into_owned();
    let root_pat = match crate_name {
        Some(n) => format!("crate|self|super|{}", n.replace('-', "_")),
        None => "crate|self|super".to_string(),
    };
    let re_path = Regex::new(&format!("\\b(?:{root_pat}){}", RE_IDENT_PATH.as_str())).unwrap();
    for m in re_path.find_iter(&rest) {
        let segs: Vec<String> = m.as_str().split("::").map(|s| s.to_string()).collect();
        if let Some(spec) = path_spec(&segs, &inline_at(m.start())) {
            out.push(RawImport {
                spec,
                kind: EvidenceKind::InlinePath,
                test_only: test_file || in_test(m.start()),
                line: line_at(m.start()),
            });
        }
    }

    // `mod x;` — parent→child evidence only.
    for (name, pos) in decls {
        out.push(RawImport {
            spec: ImportSpec::Rust {
                root: RustRoot::SelfMod {
                    inline: inline_at(pos),
                },
                segs: vec![name],
            },
            kind: EvidenceKind::ModDecl,
            test_only: test_file || in_test(pos),
            line: line_at(pos),
        });
    }

    out
}

/// Normalize a raw path (`["super","super","x"]`) into an [`ImportSpec`]
/// — leading `super` segments fold into hop counts. `None` for
/// degenerate paths.
fn path_spec(segs: &[String], inline: &[String]) -> Option<ImportSpec> {
    let (first, rest) = segs.split_first()?;
    let root = match first.as_str() {
        "crate" => RustRoot::Crate,
        "self" => RustRoot::SelfMod {
            inline: inline.to_vec(),
        },
        "super" => {
            let extra = rest.iter().take_while(|s| s.as_str() == "super").count();
            return Some(ImportSpec::Rust {
                root: RustRoot::Super {
                    levels: 1 + extra,
                    inline: inline.to_vec(),
                },
                segs: rest[extra..].to_vec(),
            });
        }
        name => RustRoot::Named(name.to_string()),
    };
    Some(ImportSpec::Rust {
        root,
        segs: rest.to_vec(),
    })
}

/// Expand a `use` body (`a::{b, c::{d, e as f}, g::*}`) into plain
/// segment paths. Best-effort: malformed input yields partial paths.
pub fn expand_use_tree(body: &str) -> Vec<Vec<String>> {
    let b = body.as_bytes();
    let mut pos = 0usize;
    parse_path(b, &mut pos, &[])
}

fn skip_ws(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && (b[*pos] as char).is_whitespace() {
        *pos += 1;
    }
}

fn read_ident(b: &[u8], pos: &mut usize) -> Option<String> {
    skip_ws(b, pos);
    let start = *pos;
    while *pos < b.len() && (b[*pos].is_ascii_alphanumeric() || b[*pos] == b'_' || b[*pos] == b'*')
    {
        *pos += 1;
    }
    if *pos == start {
        return None;
    }
    Some(String::from_utf8_lossy(&b[start..*pos]).into_owned())
}

/// `seg(::seg)*` where a seg may be `{list}` — extends `prefix`.
fn parse_path(b: &[u8], pos: &mut usize, prefix: &[String]) -> Vec<Vec<String>> {
    let mut cur = prefix.to_vec();
    loop {
        skip_ws(b, pos);
        match b.get(*pos) {
            Some(b'{') => {
                *pos += 1;
                let mut out = Vec::new();
                for item in parse_list(b, pos) {
                    let mut p = cur.clone();
                    p.extend(item);
                    out.push(p);
                }
                return out;
            }
            Some(_) => {
                let Some(ident) = read_ident(b, pos) else {
                    return if cur.len() > prefix.len() {
                        vec![cur]
                    } else {
                        vec![]
                    };
                };
                cur.push(ident);
                skip_ws(b, pos);
                if b[*pos..].starts_with(b"as")
                    && b.get(*pos + 2)
                        .is_some_and(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
                {
                    *pos += 2;
                    let _ = read_ident(b, pos);
                    skip_ws(b, pos);
                }
                if b[*pos..].starts_with(b"::") {
                    *pos += 2;
                    continue;
                }
                return vec![cur];
            }
            None => return vec![cur],
        }
    }
}

/// Comma-separated trees inside `{ … }`.
fn parse_list(b: &[u8], pos: &mut usize) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    loop {
        skip_ws(b, pos);
        match b.get(*pos) {
            None => return out,
            Some(b'}') => {
                *pos += 1;
                return out;
            }
            Some(b',') => *pos += 1,
            Some(_) => {
                out.extend(parse_path(b, pos, &[]));
                skip_ws(b, pos);
                if b.get(*pos) == Some(&b',') {
                    *pos += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drift::imports::sanitize;

    fn specs(src: &str, cfg_test: bool) -> Vec<RawImport> {
        extract(
            &sanitize::sanitize(super::super::Lang::Rust, src),
            Path::new("src/x.rs"),
            false,
            Some("mycrate"),
            cfg_test,
        )
    }

    #[test]
    fn expands_nested_use_trees() {
        let paths = expand_use_tree("crate::{a, b::{c, d as e}, f::*}");
        assert_eq!(
            paths,
            vec![
                vec!["crate", "a"],
                vec!["crate", "b", "c"],
                vec!["crate", "b", "d"],
                vec!["crate", "f", "*"],
            ]
        );
    }

    #[test]
    fn use_kinds_and_roots() {
        let src =
            "use crate::a::b;\npub use self::c;\nuse super::d::e;\nuse mycrate::f;\nuse std::g;\n";
        let out = specs(src, true);
        let kinds: Vec<_> = out.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            [
                EvidenceKind::Import,
                EvidenceKind::ReExport,
                EvidenceKind::Import,
                EvidenceKind::Import,
                EvidenceKind::Import
            ]
        );
        assert!(matches!(
            out[0].spec,
            ImportSpec::Rust {
                root: RustRoot::Crate,
                ..
            }
        ));
        assert!(matches!(
            out[1].spec,
            ImportSpec::Rust {
                root: RustRoot::SelfMod { .. },
                ..
            }
        ));
        assert!(matches!(
            out[2].spec,
            ImportSpec::Rust {
                root: RustRoot::Super { levels: 1, .. },
                ..
            }
        ));
        assert!(matches!(
            out[3].spec,
            ImportSpec::Rust { root: RustRoot::Named(ref n), .. } if n == "mycrate"
        ));
        assert!(matches!(
            out[4].spec,
            ImportSpec::Rust { root: RustRoot::Named(ref n), .. } if n == "std"
        ));
    }

    #[test]
    fn inline_paths_and_mod_decls() {
        let src = "mod helper;\nfn f() { let _ = crate::deep::inner::probe(); }\n";
        let out = specs(src, true);
        assert!(out.iter().any(|i| i.kind == EvidenceKind::ModDecl));
        assert!(out.iter().any(|i| i.kind == EvidenceKind::InlinePath));
    }

    #[test]
    fn cfg_test_marks_test_only() {
        let src = "use crate::a;\n#[cfg(test)]\nmod tests {\n    use super::b;\n}\n#[cfg(test)]\nuse crate::c;\n";
        let on = specs(src, true);
        // crate::a real; super::b inside cfg mod → test; crate::c attr'd → test.
        let flags: Vec<bool> = on.iter().map(|i| i.test_only).collect();
        assert_eq!(flags, [false, true, true]);
        let off = specs(src, false);
        assert!(off.iter().all(|i| !i.test_only));
    }

    #[test]
    fn inline_mod_tracks_super_context() {
        let src = "mod inner {\n    use super::sib::x;\n}\n";
        let out = specs(src, true);
        match &out[0].spec {
            ImportSpec::Rust {
                root: RustRoot::Super { levels, inline },
                ..
            } => {
                assert_eq!(*levels, 1);
                assert_eq!(inline, &["inner".to_string()]);
            }
            other => panic!("expected super spec, got {other:?}"),
        }
    }
}
