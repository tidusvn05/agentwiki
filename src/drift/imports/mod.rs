//! Import extraction: language detection, raw import records, and the
//! per-language `extract_imports` dispatcher.
//!
//! This is a separate extractor from `scanner::insights::extract` — that
//! one is tuned for prompt materials (regex-only, capped, feeds
//! `dir_summary` prompts) and cannot be reused without breaking prompt
//! caches.

pub mod js;
pub mod python;
pub mod rust;
pub mod sanitize;

use std::path::Path;

/// Language bucket for a scanned file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// Rust.
    Rust,
    /// Python.
    Python,
    /// JavaScript / TypeScript (incl. jsx/tsx/mjs/cjs/vue/svelte).
    Js,
    /// A programming language we don't extract imports from.
    UnsupportedCode,
    /// Not a programming file (docs, data, config…).
    Other,
}

impl Lang {
    /// Classify by lowercase file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "rs" => Self::Rust,
            "py" | "pyw" | "pyi" => Self::Python,
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" => {
                Self::Js
            }
            "go" | "java" | "kt" | "kts" | "scala" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp"
            | "cs" | "rb" | "php" | "swift" | "dart" | "lua" | "pl" | "r" | "jl" | "m" | "mm"
            | "sh" | "bash" | "zsh" | "sql" | "graphql" | "proto" => Self::UnsupportedCode,
            _ => Self::Other,
        }
    }

    /// Any recognized programming language (even unsupported ones).
    pub fn is_code(self) -> bool {
        self != Self::Other
    }

    /// A language we can extract import edges from.
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Rust | Self::Python | Self::Js)
    }
}

/// How a piece of code evidence was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceKind {
    /// A real `use`/`import`/`require` statement.
    Import,
    /// An inline qualified path (`crate::x::y` in an expression).
    InlinePath,
    /// `pub use` / `from x import y` in `__init__.py` / `export … from`.
    ReExport,
    /// `mod x;` declaration — confirms parent→child pairs only; never
    /// used for transitive paths or undocumented detection.
    ModDecl,
    /// Synthesized edge through a facade's re-export closure.
    Facade,
}

/// A parsed import specifier before resolution — language-specific.
#[derive(Debug, Clone)]
pub enum ImportSpec {
    /// Rust path: root keyword + module segments.
    Rust {
        /// Root: `crate`, `self`, `super` (levels), or an extern crate name.
        root: RustRoot,
        /// Module path segments after the root.
        segs: Vec<String>,
    },
    /// Python `import` / `from … import`.
    Python {
        /// Leading dots in `from .x import y` (0 = absolute).
        level: u32,
        /// Module path segments (`a.b.c` → `a`,`b`,`c`).
        module: Vec<String>,
        /// Imported names (empty for plain `import x`).
        names: Vec<String>,
    },
    /// JS/TS raw specifier (`./x.js`, `@/y`, `react`…).
    Js {
        /// The specifier string as written.
        specifier: String,
    },
}

/// Rust path root.
#[derive(Debug, Clone)]
pub enum RustRoot {
    /// `crate::…`
    Crate,
    /// `self::…` — `inline` is the inline `mod {}` stack at that point.
    SelfMod {
        /// Names of enclosing inline `mod` blocks.
        inline: Vec<String>,
    },
    /// `super::…` — `levels` counts the `super::` prefixes.
    Super {
        /// Number of `super` hops.
        levels: usize,
        /// Names of enclosing inline `mod` blocks.
        inline: Vec<String>,
    },
    /// `<crate_name>::…` or a bare `other_crate::…`.
    Named(String),
}

/// One raw import found in a file, before resolution.
#[derive(Debug, Clone)]
pub struct RawImport {
    /// Language-specific specifier.
    pub spec: ImportSpec,
    /// Evidence kind.
    pub kind: EvidenceKind,
    /// Inside a `#[cfg(test)]` region or a test file.
    pub test_only: bool,
    /// 1-based line in the source file.
    pub line: u32,
}

/// Extract raw imports from sanitized `content` (comments/strings already
/// blanked, offsets preserved) of the file at `rel_path`.
///
/// `test_file` marks files matched by `drift.test_globs`; `crate_name` is
/// the importing crate's package name (`-`→`_`) so Rust files recognize
/// `use <crate>::…` self-references; `cfg_test` marks `#[cfg(test)]`
/// regions test-only.
pub fn extract_imports(
    lang: Lang,
    rel_path: &Path,
    sanitized: &str,
    test_file: bool,
    crate_name: Option<&str>,
    cfg_test: bool,
) -> Vec<RawImport> {
    match lang {
        Lang::Rust => rust::extract(sanitized, rel_path, test_file, crate_name, cfg_test),
        Lang::Python => python::extract(sanitized, rel_path, test_file),
        Lang::Js => js::extract(sanitized, rel_path, test_file),
        Lang::UnsupportedCode | Lang::Other => Vec::new(),
    }
}
