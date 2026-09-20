//! Claimed-edge verdicts — the C1–C12 filter chain.
//!
//! First verdict wins. Order: endpoint sanity (C1–C3), checkability
//! guards (C7–C10), positive evidence (C4 containment → C5 direct →
//! C6 transitive), negatives last (C11 reversed → C12 phantom).
//!
//! Guards run before evidence on purpose: an uncheckable claim is
//! `unverifiable` no matter what the code shows — e.g. a `data_flow`
//! claim into a fixture dir is containment-shaped but still not
//! import-checkable, and a claim into a prose-only dir is `non_code`
//! even when the two endpoints nest.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::reports::{CoreDependency, DependencyType};

use super::claims::{Endpoint, display_path};
use super::config::DriftConfig;
use super::findings::{EvidenceRef, Finding, FindingClass, finding_id};
use super::graph::{self, FileGraph, NodeGraph, NodeSet};

/// A claim normalized to node ids, deduped on `(from, to)`.
#[derive(Debug)]
pub struct ClaimedEdge {
    /// Normalized source node id.
    pub from: String,
    /// Normalized target node id.
    pub to: String,
    /// Source endpoint resolution.
    pub from_ep: Endpoint,
    /// Target endpoint resolution.
    pub to_ep: Endpoint,
    /// Claimed dependency kind (of the highest-importance dup).
    pub kind: DependencyType,
    /// Claimed importance (max over dups).
    pub importance: u8,
}

/// Everything `analyze` precomputes once for the claim chain.
pub struct CompareCtx<'a> {
    /// Config.
    pub cfg: &'a DriftConfig,
    /// Lifted node set.
    pub nodes: &'a NodeSet,
    /// File graph (raw resolved edges).
    pub files: &'a FileGraph,
    /// `G_full` node graph (no `mod` decls).
    pub g_full: &'a NodeGraph,
    /// `G_all` node graph (incl. `mod` decls) — containment evidence only.
    pub g_all: &'a NodeGraph,
    /// Hub nodes (blocked as intermediates for transitive checks).
    pub hubs: &'a BTreeSet<String>,
}

/// Dedupe raw claims into normalized node-level edges. Claims whose
/// endpoints both normalize to the same node collapse into self-loops
/// here; truly identical `(from,to)` pairs merge keeping the
/// highest-importance entry.
pub fn normalize_claims(
    claims: &[CoreDependency],
    files: &BTreeSet<std::path::PathBuf>,
    dirs: &BTreeSet<std::path::PathBuf>,
    root: &std::path::Path,
) -> Vec<ClaimedEdge> {
    let mut out: Vec<ClaimedEdge> = Vec::new();
    let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
    for c in claims {
        let from_ep = super::claims::normalize_endpoint(&c.from, files, dirs, root);
        let to_ep = super::claims::normalize_endpoint(&c.to, files, dirs, root);
        let from = endpoint_node(&c.from, &from_ep);
        let to = endpoint_node(&c.to, &to_ep);
        if let Some(&i) = seen.get(&(from.clone(), to.clone())) {
            if c.importance > out[i].importance {
                out[i].importance = c.importance;
                out[i].kind = c.dependency_type.clone();
            }
            continue;
        }
        seen.insert((from.clone(), to.clone()), out.len());
        out.push(ClaimedEdge {
            from,
            to,
            from_ep,
            to_ep,
            kind: c.dependency_type.clone(),
            importance: c.importance,
        });
    }
    out
}

/// Node id for an endpoint — normalized path, or the raw text for
/// `Empty`/`Unknown` so findings stay readable.
fn endpoint_node(raw: &str, ep: &Endpoint) -> String {
    match ep {
        Endpoint::File(p) | Endpoint::Dir(p) => display_path(p),
        _ => raw.trim().trim_matches('`').to_string(),
    }
}

/// Run the C1–C12 chain on one normalized claim.
pub fn classify_claim(ctx: &CompareCtx, c: &ClaimedEdge) -> Finding {
    let f =
        |class: FindingClass, reason: &str, detail: Option<String>, ev: Vec<EvidenceRef>| Finding {
            id: finding_id(class, &c.from, &c.to),
            class,
            from: c.from.clone(),
            to: c.to.clone(),
            kind: Some(c.kind.as_str().to_string()),
            importance: Some(c.importance),
            reason: reason.to_string(),
            detail,
            evidence: ev,
            in_baseline: false,
        };

    // C1/C2 — endpoint sanity.
    if matches!(c.from_ep, Endpoint::Empty) || matches!(c.to_ep, Endpoint::Empty) {
        return f(FindingClass::Unverifiable, "empty_endpoint", None, vec![]);
    }
    if matches!(c.from_ep, Endpoint::Unknown) || matches!(c.to_ep, Endpoint::Unknown) {
        return f(FindingClass::Unverifiable, "unknown_endpoint", None, vec![]);
    }
    // C3 — same node both ends.
    if c.from == c.to {
        return f(FindingClass::Structural, "self_loop", None, vec![]);
    }

    // C7 — kind guard: `data_flow` is never import-checkable.
    if matches!(c.kind, DependencyType::DataFlow)
        || !ctx.cfg.checkable_kinds.contains(c.kind.as_str())
    {
        return f(
            FindingClass::Unverifiable,
            "kind_not_checkable",
            None,
            vec![],
        );
    }

    // C8 — an endpoint with no code files (incl. dirs on disk that the
    // scanner produced zero files for).
    for (node, name) in [(&c.from, "from"), (&c.to, "to")] {
        if ctx.nodes.code_files.get(node).copied().unwrap_or(0) == 0 {
            return f(
                FindingClass::Unverifiable,
                "non_code_endpoint",
                Some(format!(
                    "{name} endpoint '{node}' has no scanned code files"
                )),
                vec![],
            );
        }
    }

    // C9 — language coverage at either end.
    for (node, name) in [(&c.from, "from"), (&c.to, "to")] {
        let code = ctx.nodes.code_files.get(node).copied().unwrap_or(0);
        let sup = ctx.nodes.supported_files.get(node).copied().unwrap_or(0);
        let cov = if code == 0 {
            1.0
        } else {
            sup as f64 / code as f64
        };
        if cov < ctx.cfg.min_language_coverage {
            return f(
                FindingClass::Unverifiable,
                "language_coverage",
                Some(format!(
                    "{name} endpoint '{node}': {sup}/{code} files in supported languages"
                )),
                vec![],
            );
        }
    }

    // C10 — resolver confidence: source node drowning in unresolved
    // imports, or a repo with zero internal edges (resolver is broken —
    // everything would look phantom).
    let total: usize = ctx
        .files
        .total_imports
        .iter()
        .filter(|(f, _)| ctx.nodes.file_node.get(*f) == Some(&c.from))
        .map(|(_, n)| n)
        .sum();
    let unres: usize = ctx
        .files
        .unresolved
        .iter()
        .filter(|(f, _)| ctx.nodes.file_node.get(*f) == Some(&c.from))
        .map(|(_, n)| n)
        .sum();
    if (total > 0 && unres as f64 / total as f64 > ctx.cfg.max_unresolved_ratio)
        || ctx.files.edges.is_empty()
    {
        return f(
            FindingClass::Unverifiable,
            "resolver_confidence",
            Some(format!("{unres}/{total} imports unresolved at source")),
            vec![],
        );
    }

    // C4 — containment: one endpoint is the other's ancestor. Never
    // phantom; `mod` decls count as evidence here.
    if is_ancestor(&c.from, &c.to) || is_ancestor(&c.to, &c.from) {
        if ctx.g_all.has(&c.from, &c.to) {
            return f(
                FindingClass::Confirmed,
                "containment",
                None,
                graph::evidence_samples(ctx.g_all.edge(&c.from, &c.to)),
            );
        }
        return f(FindingClass::Structural, "containment", None, vec![]);
    }

    // C5 — direct evidence on G_full.
    if ctx.g_full.has(&c.from, &c.to) {
        return f(
            FindingClass::Confirmed,
            "direct_evidence",
            None,
            graph::evidence_samples(ctx.g_full.edge(&c.from, &c.to)),
        );
    }

    // C6 — transitive evidence within `max_transitive_depth`, hubs
    // blocked as intermediates.
    if let Some(path) = ctx
        .g_full
        .reachable(&c.from, &c.to, ctx.cfg.max_transitive_depth, ctx.hubs)
    {
        return f(
            FindingClass::Confirmed,
            "transitive",
            Some(format!("via {}", path.join(" -> "))),
            vec![],
        );
    }

    // C11 — the code has the edge backwards.
    if ctx.g_full.has(&c.to, &c.from) {
        return f(
            FindingClass::Reversed,
            "reversed",
            None,
            graph::evidence_samples(ctx.g_full.edge(&c.to, &c.from)),
        );
    }

    // C12 — nothing.
    f(FindingClass::Phantom, "phantom", None, vec![])
}

/// Is `a` an ancestor dir of `b`? Node ids are `/`-joined rel paths.
fn is_ancestor(a: &str, b: &str) -> bool {
    !a.is_empty() && b.len() > a.len() && b.starts_with(a) && b.as_bytes()[a.len()] == b'/'
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::graph::{FileEdge, build_nodes, lift};
    use super::super::imports::{EvidenceKind, Lang};
    use super::*;

    fn dep(from: &str, to: &str, kind: DependencyType) -> CoreDependency {
        CoreDependency {
            from: from.to_string(),
            to: to.to_string(),
            dependency_type: kind,
            importance: 3,
            description: None,
        }
    }

    fn fe(importer: &str, target: &str) -> FileEdge {
        FileEdge {
            importer: importer.into(),
            target: target.into(),
            kind: EvidenceKind::Import,
            test_only: false,
            line: 1,
        }
    }

    /// Hand-rolled ctx: files `src/a/x.rs` `src/b/y.rs`, claims dirs
    /// `src/a` + `src/b`; file edge a/x.rs → b/y.rs.
    fn mk_ctx<'a>(
        cfg: &'a DriftConfig,
        nodes: &'a NodeSet,
        files: &'a FileGraph,
        g_full: &'a NodeGraph,
        g_all: &'a NodeGraph,
        hubs: &'a BTreeSet<String>,
    ) -> CompareCtx<'a> {
        CompareCtx {
            cfg,
            nodes,
            files,
            g_full,
            g_all,
            hubs,
        }
    }

    #[test]
    fn guard_first_then_evidence() {
        let cfg = DriftConfig::default();
        let files = BTreeSet::from([PathBuf::from("src/a/x.rs"), PathBuf::from("src/b/y.rs")]);
        let dirs = BTreeSet::from([
            PathBuf::from("src"),
            PathBuf::from("src/a"),
            PathBuf::from("src/b"),
        ]);
        let claims = vec![
            dep("src/a", "src/b", DependencyType::Import),
            dep("src/b", "src/a", DependencyType::DataFlow), // guard → unverifiable despite no edge
            dep("src/a", "src", DependencyType::Module), // containment, no evidence → structural
            dep("src", "src/b", DependencyType::Import), // containment + no direct → structural?
        ];
        let root = std::path::Path::new("/definitely-not-on-disk-xyz");
        let claimed = normalize_claims(&claims, &files, &dirs, root);

        let code = [
            (PathBuf::from("src/a/x.rs"), Lang::Rust),
            (PathBuf::from("src/b/y.rs"), Lang::Rust),
        ];
        let nodes = build_nodes(
            &claimed
                .iter()
                .flat_map(|c| [c.from_ep.clone(), c.to_ep.clone()])
                .collect::<Vec<_>>(),
            &code,
        );

        let fg = FileGraph {
            edges: vec![fe("src/a/x.rs", "src/b/y.rs")],
            unresolved: BTreeMap::new(),
            total_imports: BTreeMap::from([(PathBuf::from("src/a/x.rs"), 1usize)]),
        };
        let g_all = lift(&fg.edges, &nodes, true);
        let g_full = lift(&fg.edges, &nodes, false);
        let hubs = BTreeSet::new();
        let ctx = mk_ctx(&cfg, &nodes, &fg, &g_full, &g_all, &hubs);

        let v: Vec<(FindingClass, String)> = claimed
            .iter()
            .map(|c| {
                let f = classify_claim(&ctx, c);
                (f.class, f.reason.clone())
            })
            .collect();
        assert_eq!(v[0], (FindingClass::Confirmed, "direct_evidence".into()));
        assert_eq!(
            v[1],
            (FindingClass::Unverifiable, "kind_not_checkable".into())
        );
        assert_eq!(v[2], (FindingClass::Structural, "containment".into()));
        // src→src/b is containment; no mod-decl evidence → structural.
        assert_eq!(v[3], (FindingClass::Structural, "containment".into()));
    }

    #[test]
    fn phantom_and_reversed() {
        let cfg = DriftConfig::default();
        let files = BTreeSet::from([PathBuf::from("src/a/x.rs"), PathBuf::from("src/b/y.rs")]);
        let dirs = BTreeSet::from([
            PathBuf::from("src"),
            PathBuf::from("src/a"),
            PathBuf::from("src/b"),
        ]);
        let claims = vec![
            dep("src/b", "src/a", DependencyType::Import), // code has a→b → reversed
            dep("src/b", "src/b", DependencyType::Import), // self → structural
        ];
        let root = std::path::Path::new("/definitely-not-on-disk-xyz");
        let claimed = normalize_claims(&claims, &files, &dirs, root);
        let code = [
            (PathBuf::from("src/a/x.rs"), Lang::Rust),
            (PathBuf::from("src/b/y.rs"), Lang::Rust),
        ];
        let nodes = build_nodes(
            &claimed
                .iter()
                .flat_map(|c| [c.from_ep.clone(), c.to_ep.clone()])
                .collect::<Vec<_>>(),
            &code,
        );
        let fg = FileGraph {
            edges: vec![fe("src/a/x.rs", "src/b/y.rs")],
            unresolved: BTreeMap::new(),
            total_imports: BTreeMap::new(),
        };
        let g_all = lift(&fg.edges, &nodes, true);
        let g_full = lift(&fg.edges, &nodes, false);
        let hubs = BTreeSet::new();
        let ctx = mk_ctx(&cfg, &nodes, &fg, &g_full, &g_all, &hubs);

        let v: Vec<FindingClass> = claimed
            .iter()
            .map(|c| classify_claim(&ctx, c).class)
            .collect();
        assert_eq!(v[0], FindingClass::Reversed);
        assert_eq!(v[1], FindingClass::Structural); // self_loop
    }

    #[test]
    fn phantom_when_nothing_matches() {
        let cfg = DriftConfig::default();
        let files = BTreeSet::from([PathBuf::from("src/a/x.rs"), PathBuf::from("src/b/y.rs")]);
        let dirs = BTreeSet::from([
            PathBuf::from("src"),
            PathBuf::from("src/a"),
            PathBuf::from("src/b"),
        ]);
        let claims = vec![dep("src/b", "src/a", DependencyType::Import)];
        let root = std::path::Path::new("/definitely-not-on-disk-xyz");
        let claimed = normalize_claims(&claims, &files, &dirs, root);
        let code = [
            (PathBuf::from("src/a/x.rs"), Lang::Rust),
            (PathBuf::from("src/b/y.rs"), Lang::Rust),
        ];
        let nodes = build_nodes(
            &claimed
                .iter()
                .flat_map(|c| [c.from_ep.clone(), c.to_ep.clone()])
                .collect::<Vec<_>>(),
            &code,
        );
        // No edges at all → C10 resolver_confidence (0 internal edges).
        let fg = FileGraph::default();
        let g_all = lift(&fg.edges, &nodes, true);
        let g_full = lift(&fg.edges, &nodes, false);
        let hubs = BTreeSet::new();
        let ctx = mk_ctx(&cfg, &nodes, &fg, &g_full, &g_all, &hubs);
        assert_eq!(
            classify_claim(&ctx, &claimed[0]).reason,
            "resolver_confidence"
        );

        // With one unrelated edge the same claim becomes phantom.
        let fg = FileGraph {
            edges: vec![fe("src/a/x.rs", "src/b/y.rs")],
            ..Default::default()
        };
        let g_all = lift(&fg.edges, &nodes, true);
        let g_full = lift(&fg.edges, &nodes, false);
        let ctx = mk_ctx(&cfg, &nodes, &fg, &g_full, &g_all, &hubs);
        assert_eq!(
            classify_claim(&ctx, &claimed[0]).class,
            FindingClass::Reversed
        );
    }
}
