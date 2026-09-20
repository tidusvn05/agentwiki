//! Undocumented-edge detection — the U1–U9 filter chain over `G_strict`
//! (strong, non-test evidence only).
//!
//! Every filter a candidate fails is counted so the report can show how
//! much noise was suppressed (`filtered:` line). Candidates that survive
//! become `undocumented` findings.

use std::collections::{BTreeMap, BTreeSet};

use super::claims::display_path;
use super::compare::ClaimedEdge;
use super::config::DriftConfig;
use super::findings::{Finding, FindingClass, finding_id};
use super::graph::{self, NodeGraph, NodeKind, NodeSet};

/// Claims-graph path length that counts as "already documented": the
/// docs connect A⇝B through ≥2 intermediates — a genuinely implied
/// dependency. Shorter chains (a 1-intermediate layer-skip) are still
/// reported.
const MIN_COVERING_PATH: usize = 3;

/// Run U1–U9 on every lifted node edge; `filtered` collects per-filter
/// drop counts keyed by filter name.
pub fn find(
    cfg: &DriftConfig,
    nodes: &NodeSet,
    g_full: &NodeGraph,
    hubs: &BTreeSet<String>,
    claimed: &[ClaimedEdge],
    filtered: &mut BTreeMap<String, usize>,
) -> Vec<Finding> {
    let ignore_files: Vec<glob::Pattern> = cfg
        .ignore_files
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    let ignore_nodes: BTreeSet<&str> = cfg.ignore_nodes.iter().map(|s| s.as_str()).collect();

    // Claim-level adjacency for U6/U7.
    let claimed_pairs: BTreeSet<(String, String)> = claimed
        .iter()
        .map(|c| (c.from.clone(), c.to.clone()))
        .collect();
    let mut claim_adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for c in claimed {
        claim_adj
            .entry(c.from.as_str())
            .or_default()
            .insert(c.to.as_str());
    }

    let mut bump = |k: &str| *filtered.entry(k.to_string()).or_insert(0) += 1;
    let mut out: Vec<Finding> = Vec::new();
    let mut capped = 0usize;

    for (a, targets) in &g_full.adj {
        for b in targets.keys() {
            if claimed_pairs.contains(&(a.clone(), b.clone())) {
                continue; // already a claim — the C-chain owns it
            }
            // U1 — both ends must be claimed nodes.
            if !matches!(nodes.kinds.get(a), Some(NodeKind::File | NodeKind::Dir))
                || !matches!(nodes.kinds.get(b), Some(NodeKind::File | NodeKind::Dir))
            {
                bump("u1_unclaimed_end");
                continue;
            }
            // U2 — parent→child containment is structural, not drift.
            if is_ancestor(a, b) || is_ancestor(b, a) {
                bump("u2_containment");
                continue;
            }
            // U3 — need strong, non-test evidence.
            let strong = g_full.strong_evidence(a, b);
            if strong.is_empty() {
                bump("u3_weak_evidence");
                continue;
            }
            // U4 — ignored nodes / utility files.
            if ignore_nodes.contains(a.as_str()) || ignore_nodes.contains(b.as_str()) {
                bump("u4_ignored");
                continue;
            }
            let ignored = |p: &std::path::Path| {
                let s = display_path(p);
                ignore_files.iter().any(|pat| pat.matches(&s))
            };
            if strong
                .iter()
                .all(|e| ignored(&e.importer) || ignored(&e.target))
            {
                bump("u4_ignored");
                continue;
            }
            // U5 — edges into hubs are mostly mechanical fan-in.
            if hubs.contains(b) {
                bump("u5_hub_target");
                continue;
            }
            // U6 — the reverse is already claimed (the claim side flags
            // it as `reversed`; don't double-report).
            if claimed_pairs.contains(&(b.clone(), a.clone())) {
                bump("u6_reverse_claimed");
                continue;
            }
            // U7 — claims already connect a⇝b through a deep chain.
            // `None` (no claims path at all) is *more* undocumented, not
            // less — only an existing, deep chain suppresses.
            if matches!(claim_distance(&claim_adj, a, b), Some(d) if d >= MIN_COVERING_PATH) {
                bump("u7_claimed_path");
                continue;
            }
            // U8 — need several independent import sites.
            let pairs: BTreeSet<(String, String)> = strong
                .iter()
                .map(|e| (display_path(&e.importer), display_path(&e.target)))
                .collect();
            if pairs.len() < cfg.min_undocumented_imports {
                bump("u8_thin");
                continue;
            }
            // U9 — report cap.
            if out.len() >= cfg.max_undocumented_reported {
                capped += 1;
                continue;
            }
            out.push(Finding {
                id: finding_id(FindingClass::Undocumented, a, b),
                class: FindingClass::Undocumented,
                from: a.clone(),
                to: b.clone(),
                kind: None,
                importance: None,
                reason: "undocumented".to_string(),
                detail: Some(format!("{} import site(s)", pairs.len())),
                evidence: graph::evidence_samples(&strong.into_iter().cloned().collect::<Vec<_>>()),
                in_baseline: false,
            });
        }
    }
    if capped > 0 {
        *filtered.entry("u9_capped".to_string()).or_insert(0) += capped;
    }
    out
}

/// `/`-path ancestry between two node ids.
fn is_ancestor(a: &str, b: &str) -> bool {
    !a.is_empty() && b.len() > a.len() && b.starts_with(a) && b.as_bytes()[a.len()] == b'/'
}

/// Shortest distance `a →…→ b` in the claims graph (`None` = no path).
fn claim_distance(adj: &BTreeMap<&str, BTreeSet<&str>>, a: &str, b: &str) -> Option<usize> {
    let mut dist: BTreeMap<&str, usize> = BTreeMap::from([(a, 0)]);
    let mut frontier = vec![a];
    while let Some(n) = frontier.pop() {
        let Some(&d) = dist.get(n) else { continue };
        for next in adj.get(n).into_iter().flatten() {
            if !dist.contains_key(next) {
                dist.insert(*next, d + 1);
                frontier.push(*next);
            }
        }
    }
    dist.get(b).copied()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::claims::Endpoint;
    use super::super::compare::ClaimedEdge;
    use super::super::graph::{FileEdge, FileGraph, build_nodes, lift};
    use super::super::imports::{EvidenceKind, Lang};
    use super::*;
    use crate::agent::reports::DependencyType;

    /// Node dirs a–d each own one code file; the real edge under test is
    /// `a → b` with 3 import sites. `claims` lists the claim-side edges.
    fn setup(claims: &[(&str, &str)]) -> (Vec<ClaimedEdge>, NodeSet, NodeGraph) {
        let mut claimed: Vec<ClaimedEdge> = Vec::new();
        let mut endpoints: Vec<Endpoint> = Vec::new();
        for &(a, b) in claims {
            claimed.push(ClaimedEdge {
                from: a.to_string(),
                to: b.to_string(),
                from_ep: Endpoint::Dir(PathBuf::from(a)),
                to_ep: Endpoint::Dir(PathBuf::from(b)),
                kind: DependencyType::Import,
                importance: 1,
            });
            endpoints.push(Endpoint::Dir(PathBuf::from(a)));
            endpoints.push(Endpoint::Dir(PathBuf::from(b)));
        }
        let code: Vec<(PathBuf, Lang)> = [
            "a/f1.py", "a/f2.py", "a/f3.py", "b/g.py", "c/h.py", "d/i.py",
        ]
        .iter()
        .map(|f| (PathBuf::from(f), Lang::Python))
        .collect();
        let nodes = build_nodes(&endpoints, &code);
        let fg = FileGraph {
            edges: ["f1", "f2", "f3"]
                .iter()
                .map(|f| FileEdge {
                    importer: PathBuf::from(format!("a/{f}.py")),
                    target: PathBuf::from("b/g.py"),
                    kind: EvidenceKind::Import,
                    test_only: false,
                    line: 1,
                })
                .collect(),
            ..Default::default()
        };
        let g_full = lift(&fg.edges, &nodes, false);
        (claimed, nodes, g_full)
    }

    #[test]
    fn u7_unreachable_claims_still_report() {
        // Claims mention a and b but never connect a⇝b: the edge is as
        // undocumented as it gets — must NOT be filtered by U7.
        let cfg = DriftConfig::default();
        let (claimed, nodes, g_full) = setup(&[("a", "c"), ("c", "d"), ("b", "c")]);
        let mut filtered = BTreeMap::new();
        let out = find(
            &cfg,
            &nodes,
            &g_full,
            &BTreeSet::new(),
            &claimed,
            &mut filtered,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "undocumented:a->b");
    }

    #[test]
    fn u7_deep_claims_path_suppresses() {
        // Claims connect a→c→d→b (distance 3): implied dependency, drop.
        let cfg = DriftConfig::default();
        let (claimed, nodes, g_full) = setup(&[("a", "c"), ("c", "d"), ("d", "b")]);
        let mut filtered = BTreeMap::new();
        let out = find(
            &cfg,
            &nodes,
            &g_full,
            &BTreeSet::new(),
            &claimed,
            &mut filtered,
        );
        assert!(out.is_empty());
        assert_eq!(filtered.get("u7_claimed_path"), Some(&1));
    }

    #[test]
    fn u7_layer_skip_still_reports() {
        // Claims connect a→c→b (distance 2): a 1-intermediate skip is
        // still worth reporting.
        let cfg = DriftConfig::default();
        let (claimed, nodes, g_full) = setup(&[("a", "c"), ("c", "b")]);
        let mut filtered = BTreeMap::new();
        let out = find(
            &cfg,
            &nodes,
            &g_full,
            &BTreeSet::new(),
            &claimed,
            &mut filtered,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "undocumented:a->b");
    }
}
