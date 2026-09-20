//! `DriftReport` — serialized to `.agentwiki/drift.json` and rendered
//! for humans by `run()`.

use std::collections::BTreeMap;

use serde::Serialize;

use super::findings::{Finding, FindingClass};

/// Schema version of the JSON report.
pub const REPORT_VERSION: u32 = 1;

/// Full drift-check result.
#[derive(Debug, Serialize)]
pub struct DriftReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Where claims were read from.
    pub claims_source: String,
    /// Claim edges after normalization/dedup.
    pub claims_total: usize,
    /// class → count.
    pub counts: BTreeMap<String, usize>,
    /// Hub nodes (in-degree ≥ `hub_in_degree_ratio`).
    pub hubs: Vec<String>,
    /// Per-filter blocked counts (undocumented chain).
    pub filtered: BTreeMap<String, usize>,
    /// Every finding, sorted by (class, id).
    pub findings: Vec<Finding>,
    /// Baseline bookkeeping, when a baseline is in play.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineInfo>,
}

/// Baseline diff summary inside the report.
#[derive(Debug, Serialize)]
pub struct BaselineInfo {
    /// Baseline file path.
    pub path: String,
    /// Current findings already in the baseline.
    pub known: usize,
    /// Current findings not in the baseline.
    pub new: usize,
    /// Baseline entries no longer produced.
    pub stale: usize,
}

impl DriftReport {
    /// Findings of one class.
    pub fn by_class(&self, class: FindingClass) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.class == class)
    }

    /// New `phantom`/`reversed` findings — what `--strict` fails on.
    pub fn strict_failures(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| matches!(f.class, FindingClass::Phantom | FindingClass::Reversed))
            .filter(|f| !f.in_baseline)
            .collect()
    }
}

/// Human-readable report. `verbose` lists unverifiable/structural edges
/// individually instead of grouping them by reason.
pub fn render(r: &DriftReport, verbose: bool) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "claims: {} ({} edges)", r.claims_source, r.claims_total);

    // ok — confirmed, split by reason.
    let mut by_reason: BTreeMap<&str, usize> = BTreeMap::new();
    for f in r.by_class(FindingClass::Confirmed) {
        *by_reason.entry(f.reason.as_str()).or_default() += 1;
    }
    let n_conf = r.counts.get("confirmed").copied().unwrap_or(0);
    let breakdown = by_reason
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(s, "  ok   {n_conf} confirmed ({breakdown})");

    // info — structural + unverifiable, grouped unless -v.
    for (class, label) in [
        (FindingClass::Structural, "structural"),
        (FindingClass::Unverifiable, "unverifiable"),
    ] {
        let items: Vec<&Finding> = r.by_class(class).collect();
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(s, " info  {} {label}", items.len());
        if verbose {
            for f in &items {
                let _ = writeln!(
                    s,
                    "        {} -> {} [{}]{}",
                    f.from,
                    f.to,
                    f.reason,
                    f.detail
                        .as_deref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default()
                );
            }
        } else {
            let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
            for f in &items {
                *reasons.entry(f.reason.as_str()).or_default() += 1;
            }
            let rs = reasons
                .iter()
                .map(|(k, v)| format!("{v} {k}"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(s, "        ({rs})");
        }
    }

    // warn — phantom / reversed / undocumented, always listed.
    for (class, label) in [
        (FindingClass::Phantom, "phantom"),
        (FindingClass::Reversed, "reversed"),
        (FindingClass::Undocumented, "undocumented"),
    ] {
        let items: Vec<&Finding> = r.by_class(class).collect();
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(s, " warn  {} {label}", items.len());
        for f in &items {
            let meta = match (&f.kind, f.importance) {
                (Some(k), Some(i)) => format!(" [{k}, importance {i}]"),
                _ => String::new(),
            };
            let _ = writeln!(s, "        {} -> {}{}", f.from, f.to, meta);
            for e in &f.evidence {
                let _ = writeln!(s, "          e.g. {}:{} (-> {})", e.file, e.line, e.target);
            }
            if let Some(d) = &f.detail {
                let _ = writeln!(s, "          {d}");
            }
        }
    }

    if !r.hubs.is_empty() {
        let _ = writeln!(s, "hubs: {}", r.hubs.join(", "));
    }
    if !r.filtered.is_empty() {
        let fs = r
            .filtered
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(s, "filtered: {fs}");
    }
    if let Some(b) = &r.baseline {
        let _ = writeln!(
            s,
            "baseline: {} ({} known, {} new, {} stale)",
            b.path, b.known, b.new, b.stale
        );
    }

    let n_fail = r.strict_failures().len();
    let _ = writeln!(s, "result: {n_fail} finding(s) would fail under --strict");
    s
}
