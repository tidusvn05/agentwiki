//! File → node graph lifting, reachability, hub detection, and the
//! facade re-export closure.
//!
//! Asymmetric evidence: phantom/reversed verdicts run on the widest graph
//! (test imports, inline paths, facade-weak edges), undocumented
//! detection on the strictest (strong evidence only). `mod x;` edges are
//! kept apart: they confirm parent→child claims but never participate in
//! reachability or undocumented checks — otherwise `lib.rs` would make
//! every node reachable and nothing could be a phantom.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::claims::{Endpoint, display_path};
use super::findings::EvidenceRef;
use super::imports::EvidenceKind;

/// Node kind in the lifted graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A claim endpoint that resolved to a file.
    File,
    /// A claim endpoint that resolved to a directory.
    Dir,
    /// Unclaimed parent dir of a file — joins reachability, never
    /// appears in findings.
    Hidden,
}

/// The claim-defined node set plus the file→node lift.
#[derive(Debug, Default)]
pub struct NodeSet {
    /// node id (normalized `/`-path) → kind.
    pub kinds: BTreeMap<String, NodeKind>,
    /// scanned file → owning node id.
    pub file_node: BTreeMap<PathBuf, String>,
    /// node → count of owned code files (`Lang::is_code`).
    pub code_files: BTreeMap<String, usize>,
    /// node → count of owned supported-language files.
    pub supported_files: BTreeMap<String, usize>,
}

/// One resolved file-level edge.
#[derive(Debug, Clone)]
pub struct FileEdge {
    /// Importer file (repo-relative).
    pub importer: PathBuf,
    /// Resolved target file (repo-relative).
    pub target: PathBuf,
    /// Evidence kind.
    pub kind: EvidenceKind,
    /// Test-gated or test-file evidence.
    pub test_only: bool,
    /// 1-based line in `importer`.
    pub line: u32,
}

/// File-level resolved graph plus per-file resolver stats.
#[derive(Debug, Default)]
pub struct FileGraph {
    /// All resolved internal edges (incl. ModDecl and facade-weak).
    pub edges: Vec<FileEdge>,
    /// importer → imports that stayed `Unresolved`.
    pub unresolved: BTreeMap<PathBuf, usize>,
    /// importer → total imports seen.
    pub total_imports: BTreeMap<PathBuf, usize>,
}

impl FileGraph {
    /// Internal edges usable for `G_full` reasoning (no `mod` decls).
    pub fn full_edges(&self) -> impl Iterator<Item = &FileEdge> {
        self.edges
            .iter()
            .filter(|e| e.kind != EvidenceKind::ModDecl)
    }

    /// Edges that count as evidence for a containment claim — everything,
    /// including `mod` decls.
    pub fn any_evidence(&self, importer: &PathBuf, target: &PathBuf) -> bool {
        self.edges
            .iter()
            .any(|e| &e.importer == importer && &e.target == target)
    }
}

/// One node-level edge with its backing file evidence.
#[derive(Debug, Default)]
pub struct NodeEdge {
    /// Backing file edges.
    pub evidence: Vec<FileEdge>,
}

/// Lifted node graph: adjacency `from → to → NodeEdge`.
#[derive(Debug, Default)]
pub struct NodeGraph {
    /// `from_node → to_node → evidence`.
    pub adj: BTreeMap<String, BTreeMap<String, NodeEdge>>,
}

impl NodeGraph {
    /// Evidence records for `a → b` (empty when absent).
    pub fn edge(&self, a: &str, b: &str) -> &[FileEdge] {
        self.adj
            .get(a)
            .and_then(|m| m.get(b))
            .map(|e| e.evidence.as_slice())
            .unwrap_or(&[])
    }

    /// Any evidence for `a → b` present in this graph.
    pub fn has(&self, a: &str, b: &str) -> bool {
        !self.edge(a, b).is_empty()
    }

    /// `G_strict` evidence: strong kinds, non-test.
    pub fn strong_evidence(&self, a: &str, b: &str) -> Vec<&FileEdge> {
        self.edge(a, b)
            .iter()
            .filter(|e| {
                !e.test_only
                    && matches!(
                        e.kind,
                        EvidenceKind::Import | EvidenceKind::InlinePath | EvidenceKind::ReExport
                    )
            })
            .collect()
    }

    /// Nodes whose in-degree ≥ `ratio × total_nodes` (only when
    /// `total_nodes ≥ min_nodes`). `total_nodes` includes edge-less
    /// claimed nodes, so pass the lifted node-set size.
    pub fn hubs(&self, total_nodes: usize, ratio: f64, min_nodes: usize) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if total_nodes < min_nodes {
            return out;
        }
        let mut in_deg: BTreeMap<&str, usize> = BTreeMap::new();
        for targets in self.adj.values() {
            for (to, e) in targets {
                // Fan-in counts distinct importer *files* — a node-edge
                // count underestimates dirs, which aggregate many files.
                let importers: BTreeSet<&PathBuf> =
                    e.evidence.iter().map(|x| &x.importer).collect();
                *in_deg.entry(to.as_str()).or_default() += importers.len();
            }
        }
        tracing::debug!(total_nodes, ?in_deg, "hub in-degrees");
        for (node, deg) in in_deg {
            if deg as f64 >= ratio * total_nodes as f64 {
                out.insert(node.to_string());
            }
        }
        out
    }

    /// Shortest path `a →…→ b` within `max_depth` hops; `blocked` nodes
    /// may not appear as intermediates. Among shortest paths, returns the
    /// lexicographically smallest — deterministic for stable reports.
    pub fn reachable(
        &self,
        a: &str,
        b: &str,
        max_depth: usize,
        blocked: &BTreeSet<String>,
    ) -> Option<Vec<String>> {
        if a == b {
            return Some(vec![a.to_string()]);
        }
        // Uniform-cost search ordered by (depth, path) — the first time we
        // pop `b` we hold the shortest, lexicographically smallest path.
        let mut heap: std::collections::BinaryHeap<(
            std::cmp::Reverse<usize>,
            std::cmp::Reverse<Vec<String>>,
        )> = std::collections::BinaryHeap::new();
        heap.push((std::cmp::Reverse(0), std::cmp::Reverse(vec![a.to_string()])));
        let mut best: BTreeMap<String, usize> = BTreeMap::from([(a.to_string(), 0)]);
        while let Some((std::cmp::Reverse(depth), std::cmp::Reverse(path))) = heap.pop() {
            let node = path.last().map(|s| s.as_str()).unwrap_or(a);
            if node == b {
                return Some(path);
            }
            if depth >= max_depth {
                continue;
            }
            for next in self.adj.get(node).map(|m| m.keys()).into_iter().flatten() {
                if next != b && blocked.contains(next.as_str()) {
                    continue; // hubs blocked only as intermediates
                }
                if path.contains(next) {
                    continue; // simple paths only — cycles never help
                }
                let nd = depth + 1;
                if best.get(next.as_str()).is_some_and(|d| *d < nd) {
                    continue;
                }
                let mut np = path.clone();
                np.push(next.clone());
                best.insert(next.clone(), nd);
                heap.push((std::cmp::Reverse(nd), std::cmp::Reverse(np)));
            }
        }
        None
    }
}

/// Build the node set from resolved claim endpoints and the scanned
/// code-file set.
///
/// Each `File`/`Dir` endpoint becomes a claimed node. Every scanned code
/// file lifts to: itself when it is a file-node, else the deepest
/// claimed ancestor dir, else a hidden node for its parent dir.
pub fn build_nodes(
    claimed: &[Endpoint],
    code_files: &[(PathBuf, super::imports::Lang)],
) -> NodeSet {
    let mut set = NodeSet::default();
    for ep in claimed {
        match ep {
            Endpoint::File(p) => {
                set.kinds.insert(display_path(p), NodeKind::File);
            }
            Endpoint::Dir(p) => {
                set.kinds.insert(display_path(p), NodeKind::Dir);
            }
            _ => {}
        }
    }

    let claimed_dirs: Vec<String> = set
        .kinds
        .iter()
        .filter(|(_, k)| **k == NodeKind::Dir)
        .map(|(id, _)| id.clone())
        .collect();

    for (file, lang) in code_files {
        let node = if set.kinds.contains_key(display_path(file).as_str()) {
            display_path(file)
        } else {
            // Deepest claimed ancestor dir.
            let mut owner: Option<String> = None;
            let mut anc = file.parent();
            while let Some(d) = anc {
                let id = display_path(d);
                if matches!(set.kinds.get(&id), Some(NodeKind::Dir)) {
                    owner = Some(id);
                    break;
                }
                anc = d.parent();
            }
            owner.unwrap_or_else(|| {
                let parent = file.parent().map(display_path).unwrap_or_default();
                set.kinds.entry(parent.clone()).or_insert(NodeKind::Hidden);
                parent
            })
        };
        set.file_node.insert(file.clone(), node.clone());

        // Count toward the owning node AND every claimed ancestor dir —
        // a dir with all its code in claimed subdirs is still a code
        // dir, not a `non_code_endpoint`.
        let mut count_for = |node: &str| {
            if lang.is_code() {
                *set.code_files.entry(node.to_string()).or_default() += 1;
            }
            if lang.is_supported() {
                *set.supported_files.entry(node.to_string()).or_default() += 1;
            }
        };
        count_for(&node);
        let fp = display_path(file);
        for d in &claimed_dirs {
            if *d != node
                && fp.len() > d.len()
                && fp.starts_with(d)
                && fp.as_bytes()[d.len()] == b'/'
            {
                count_for(d);
            }
        }
    }
    set
}

/// Lift file edges to node edges. Self-loops (both files in the same
/// node) are dropped. `include_mod` selects whether `mod` decls enter
/// the graph — they belong only in the containment-evidence view, never
/// in `G_full` (reachability/undocumented).
pub fn lift(file_edges: &[FileEdge], set: &NodeSet, include_mod: bool) -> NodeGraph {
    let mut g = NodeGraph::default();
    for e in file_edges
        .iter()
        .filter(|e| include_mod || e.kind != EvidenceKind::ModDecl)
    {
        let (Some(a), Some(b)) = (set.file_node.get(&e.importer), set.file_node.get(&e.target))
        else {
            continue;
        };
        if a == b {
            continue;
        }
        g.adj
            .entry(a.clone())
            .or_default()
            .entry(b.clone())
            .or_default()
            .evidence
            .push(e.clone());
    }
    g
}

/// Add weak facade edges: for every edge `X → F` where `F` is a facade
/// file (`lib.rs`, `mod.rs`, `__init__.py`, `index.*`), follow `F`'s
/// `ReExport` edges transitively (facade → facade, depth ≤ 3) and add
/// `X → T` as `EvidenceKind::Facade` evidence.
pub fn facade_closure(edges: &mut Vec<FileEdge>) {
    let reexports: BTreeMap<PathBuf, Vec<PathBuf>> = {
        let mut m: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        for e in edges.iter().filter(|e| e.kind == EvidenceKind::ReExport) {
            m.entry(e.importer.clone())
                .or_default()
                .push(e.target.clone());
        }
        m
    };
    let facade_targets: BTreeSet<PathBuf> = edges
        .iter()
        .filter(|e| {
            matches!(e.kind, EvidenceKind::Import | EvidenceKind::ReExport) && is_facade(&e.target)
        })
        .map(|e| e.target.clone())
        .collect();

    let mut extra: Vec<FileEdge> = Vec::new();
    for e in edges.iter() {
        if !matches!(e.kind, EvidenceKind::Import | EvidenceKind::ReExport)
            || !facade_targets.contains(&e.target)
        {
            continue;
        }
        // BFS through the facade's re-export closure, ≤3 deep.
        let mut seen: BTreeSet<PathBuf> = BTreeSet::from([e.target.clone()]);
        let mut frontier: Vec<PathBuf> = vec![e.target.clone()];
        for _ in 0..3 {
            let mut next: Vec<PathBuf> = Vec::new();
            for f in &frontier {
                for t in reexports.get(f).into_iter().flatten() {
                    if seen.insert(t.clone()) {
                        next.push(t.clone());
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            for t in &next {
                if t != &e.importer {
                    extra.push(FileEdge {
                        importer: e.importer.clone(),
                        target: t.clone(),
                        kind: EvidenceKind::Facade,
                        test_only: e.test_only,
                        line: e.line,
                    });
                }
            }
            // Only facades propagate further.
            frontier = next.into_iter().filter(|t| is_facade(t)).collect();
        }
    }
    edges.extend(extra);
}

/// `lib.rs`, `mod.rs`, `__init__.py`, `index.*` — files whose imports are
/// mostly re-exports for downstream consumers.
pub fn is_facade(p: &Path) -> bool {
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
        return false;
    };
    matches!(
        name.as_str(),
        "lib.rs"
            | "mod.rs"
            | "__init__.py"
            | "index.js"
            | "index.ts"
            | "index.tsx"
            | "index.jsx"
            | "index.mjs"
            | "index.cjs"
            | "index.d.ts"
    )
}

/// Up to `n` sorted evidence samples for report rendering.
pub fn evidence_samples(ev: &[FileEdge]) -> Vec<EvidenceRef> {
    let mut refs: Vec<EvidenceRef> = ev
        .iter()
        .map(|e| EvidenceRef {
            file: display_path(&e.importer),
            target: display_path(&e.target),
            line: e.line,
        })
        .collect();
    refs.sort();
    refs.dedup();
    refs.truncate(3);
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drift::imports::Lang;

    fn fe(importer: &str, target: &str, kind: EvidenceKind, test_only: bool) -> FileEdge {
        FileEdge {
            importer: PathBuf::from(importer),
            target: PathBuf::from(target),
            kind,
            test_only,
            line: 1,
        }
    }

    /// Nodes: `src` dir + `src/a.rs`,`src/b.rs`,`lib/lib.rs` files.
    fn nodes() -> NodeSet {
        let eps = vec![
            Endpoint::Dir(PathBuf::from("src")),
            Endpoint::File(PathBuf::from("lib/lib.rs")),
        ];
        build_nodes(
            &eps,
            &[
                (PathBuf::from("src/a.rs"), Lang::Rust),
                (PathBuf::from("src/b.rs"), Lang::Rust),
                (PathBuf::from("lib/lib.rs"), Lang::Rust),
                (PathBuf::from("docs/x.md"), Lang::Other),
            ],
        )
    }

    #[test]
    fn lift_maps_files_to_nodes() {
        let set = nodes();
        let edges = vec![
            fe("src/a.rs", "src/b.rs", EvidenceKind::Import, false),
            fe("src/a.rs", "lib/lib.rs", EvidenceKind::Import, false),
            fe("src/b.rs", "src/a.rs", EvidenceKind::Import, false),
        ];
        let g = lift(&edges, &set, false);
        // a→b inside src → self-loop dropped; a→lib survives.
        assert!(!g.has("src", "src"));
        assert!(g.has("src", "lib/lib.rs"));
        assert_eq!(g.edge("src", "lib/lib.rs").len(), 1);
    }

    #[test]
    fn mod_decls_only_in_g_all() {
        // `src/a.rs mod-declares src/sub/b.rs` → node edge src→src/sub.
        let eps = vec![
            Endpoint::Dir(PathBuf::from("src")),
            Endpoint::Dir(PathBuf::from("src/sub")),
        ];
        let set = build_nodes(
            &eps,
            &[
                (PathBuf::from("src/a.rs"), Lang::Rust),
                (PathBuf::from("src/sub/b.rs"), Lang::Rust),
            ],
        );
        let edges = vec![fe("src/a.rs", "src/sub/b.rs", EvidenceKind::ModDecl, false)];
        assert!(!lift(&edges, &set, false).has("src", "src/sub"));
        assert!(lift(&edges, &set, true).has("src", "src/sub"));
    }

    #[test]
    fn reachable_respects_depth_and_blocked() {
        let set = nodes();
        let mut g = NodeGraph::default();
        for (a, b) in [
            ("a", "b"),
            ("b", "c"),
            ("c", "d"),
            ("a", "hub"),
            ("hub", "d"),
        ] {
            g.adj
                .entry(a.to_string())
                .or_default()
                .entry(b.to_string())
                .or_default();
        }
        let _ = set;
        let blocked = BTreeSet::from(["hub".to_string()]);
        assert_eq!(
            g.reachable("a", "d", 3, &BTreeSet::new()),
            Some(vec!["a".into(), "hub".into(), "d".into()])
        );
        assert_eq!(
            g.reachable("a", "d", 3, &blocked),
            Some(vec!["a".into(), "b".into(), "c".into(), "d".into()])
        );
        assert_eq!(g.reachable("a", "d", 2, &blocked), None);
    }

    #[test]
    fn hubs_need_min_nodes_and_ratio() {
        let mut g = NodeGraph::default();
        for i in 0..8 {
            g.adj
                .entry(format!("n{i}"))
                .or_default()
                .entry("hot".to_string())
                .or_default()
                .evidence
                .push(fe(
                    &format!("n{i}/f.rs"),
                    "hot/h.rs",
                    EvidenceKind::Import,
                    false,
                ));
        }
        // 8 importers into "hot" — file-level fan-in = 8.
        assert!(g.hubs(10, 0.5, 5).contains("hot"));
        assert!(g.hubs(10, 0.5, 20).is_empty()); // min_nodes gate
        assert!(g.hubs(40, 0.5, 5).is_empty()); // ratio gate
    }

    #[test]
    fn facade_closure_adds_weak_edges() {
        let mut edges = vec![
            fe("a.rs", "pkg/mod.rs", EvidenceKind::Import, false),
            fe("pkg/mod.rs", "pkg/inner.rs", EvidenceKind::ReExport, false),
            fe("pkg/mod.rs", "pkg/other.rs", EvidenceKind::ReExport, false),
        ];
        facade_closure(&mut edges);
        assert!(edges.iter().any(|e| e.kind == EvidenceKind::Facade
            && e.importer == Path::new("a.rs")
            && e.target == Path::new("pkg/inner.rs")));
        assert!(
            edges
                .iter()
                .any(|e| e.kind == EvidenceKind::Facade && e.target == Path::new("pkg/other.rs"))
        );
    }
}
