//! Comment/string blanking with offset preservation.
//!
//! Doc comments contain bait like `crate::config::Config` and string
//! literals contain code-looking text — both must be blanked before
//! import matching. Output is the same byte length: every non-code byte
//! becomes a space, except `\n` which survives so line numbers stay
//! meaningful.

use super::Lang;

/// Blank comments and string/char literals in `src` per `lang`.
pub fn sanitize(lang: Lang, src: &str) -> String {
    match lang {
        Lang::Rust => rust(src),
        Lang::Python => python(src),
        Lang::Js => js(src),
        _ => src.to_string(),
    }
}

/// Overwrite `out[i..j)` with spaces, keeping `\n`.
pub(crate) fn blank(out: &mut [u8], i: usize, j: usize) {
    for b in &mut out[i..j] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Rust: `//`, nested `/* */`, `"…"`/`b"…"`, `r"…"`/`r#"…"#`, `'c'`/`b'c'`
/// (lifetimes like `'a` left alone).
fn rust(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let n = b.len();
    let mut i = 0;
    while i < n {
        match b[i] {
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                let mut j = i;
                while j < n && b[j] != b'\n' {
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                let mut j = i + 2;
                let mut depth = 1;
                while j + 1 < n && depth > 0 {
                    if b[j] == b'/' && b[j + 1] == b'*' {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b[j + 1] == b'/' {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j.min(n));
                i = j;
            }
            b'"' => {
                let mut j = i + 1;
                while j < n && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                blank(&mut out, i, (j + 1).min(n));
                i = j + 1;
            }
            b'r' if i + 1 < n && (b[i + 1] == b'"' || b[i + 1] == b'#') => {
                // Raw string: count `#`s, then closing `"` + same `#`s.
                let mut k = i + 1;
                while k < n && b[k] == b'#' {
                    k += 1;
                }
                if k < n && b[k] == b'"' {
                    let hashes = k - (i + 1);
                    let mut j = k + 1;
                    while j < n {
                        if b[j] == b'"' {
                            let mut m = j + 1;
                            let mut ok = true;
                            for _ in 0..hashes {
                                if m >= n || b[m] != b'#' {
                                    ok = false;
                                    break;
                                }
                                m += 1;
                            }
                            if ok {
                                j = m;
                                break;
                            }
                        }
                        j += 1;
                    }
                    blank(&mut out, i, j.min(n));
                    i = j;
                } else {
                    i += 1;
                }
            }
            b'b' if i + 1 < n && b[i + 1] == b'"' => {
                i += 1; // the `"` arm handles the string next round
            }
            b'\'' => {
                // `'x'`, `'\n'`, `b'x'` = char literal; `'a` = lifetime.
                let is_char = if i + 1 < n && b[i + 1] == b'\\' {
                    true // escape char literal — find closing quote
                } else {
                    i + 2 < n && b[i + 2] == b'\''
                };
                if is_char {
                    let mut j = i + 1;
                    while j < n && b[j] != b'\'' {
                        if b[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    blank(&mut out, i, (j + 1).min(n));
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Python: `#` line comments; `'…'`/`"…"`/`'''…'''`/`"""…"""` strings
/// (raw/f/b prefixes fall out naturally — only the quotes matter).
fn python(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let n = b.len();
    let mut i = 0;
    while i < n {
        if b[i] == b'#' {
            let mut j = i;
            while j < n && b[j] != b'\n' {
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if b[i] == b'\'' || b[i] == b'"' {
            let q = b[i];
            let triple = i + 2 < n && b[i + 1] == q && b[i + 2] == q;
            let mut j = i + if triple { 3 } else { 1 };
            while j < n {
                if b[j] == b'\\' && !triple {
                    j += 2;
                    continue;
                }
                if triple {
                    if j + 2 < n && b[j] == q && b[j + 1] == q && b[j + 2] == q {
                        j += 3;
                        break;
                    }
                } else if b[j] == q {
                    j += 1;
                    break;
                }
                if !triple && b[j] == b'\n' {
                    break;
                }
                j += 1;
            }
            blank(&mut out, i, j.min(n));
            i = j;
        } else {
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// JS/TS: blank `//`, `/* */`, and `` `…` `` template literals — but keep
/// `'…'`/`"…"` contents: import specifiers ARE string literals, so
/// blanking them would erase the signal. A dynamic `import()` inside a
/// `${}` hole is missed; accepted.
fn js(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let n = b.len();
    let mut i = 0;
    while i < n {
        match b[i] {
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                let mut j = i;
                while j < n && b[j] != b'\n' {
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                let mut j = i + 2;
                while j + 1 < n && !(b[j] == b'*' && b[j + 1] == b'/') {
                    j += 1;
                }
                j = (j + 2).min(n);
                blank(&mut out, i, j);
                i = j;
            }
            b'`' => {
                let mut j = i + 1;
                while j < n && b[j] != b'`' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                blank(&mut out, i, (j + 1).min(n));
                i = j + 1;
            }
            // Skip over '…'/"…" without blanking — specifier contents must
            // survive for the extractor.
            b'\'' | b'"' => {
                let q = b[i];
                let mut j = i + 1;
                while j < n && b[j] != q {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                i = (j + 1).min(n);
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_blanks_keep_offsets() {
        let src = "//! see crate::config::Config\nuse crate::x;\nlet s = \"crate::y\";\n// z\nlet c = 'q'; let l: &'a str = s;";
        let out = rust(src);
        assert_eq!(out.len(), src.len());
        assert!(out.contains("use crate::x;"));
        assert!(!out.contains("crate::config::Config"));
        assert!(!out.contains("crate::y"));
        assert!(out.contains("'a")); // lifetime survives
        assert!(!out.contains("'q'")); // char literal blanked
        assert_eq!(out.lines().count(), src.lines().count());
    }

    #[test]
    fn rust_raw_and_nested() {
        let src = "let a = r#\"x /* y \"#;\n/* outer /* inner */ tail */\nuse crate::k;";
        let out = rust(src);
        assert!(out.contains("use crate::k;"));
        assert!(!out.contains("inner"));
        assert!(!out.contains("y"));
    }

    #[test]
    fn python_triple_and_comment() {
        let src = "SCHEMA = \"\"\"\nCREATE TABLE x\n\"\"\"\nfrom .m import T  # hi\n";
        let out = python(src);
        assert!(out.contains("from .m import T"));
        assert!(!out.contains("CREATE"));
        assert_eq!(out.lines().count(), src.lines().count());
    }

    #[test]
    fn js_strings_and_comments() {
        let src =
            "import a from './x'; // from './y'\nconst s = `tpl ${v} 'q'`;\nconst t = 'plain';\n";
        let out = js(src);
        assert!(out.contains("import a from"));
        assert!(out.contains("'./x'")); // specifier strings survive
        assert!(out.contains("'plain'"));
        assert!(!out.contains("'./y'")); // comment blanked
        assert!(!out.contains("'q'")); // inside template literal
    }
}
