//! `DriftReport` — serialized to `.agentwiki/drift.json` and rendered
//! for humans by `run()`.

use std::collections::BTreeMap;

use serde::Serialize;

use super::findings::{Finding, FindingClass};

/// Schema version of the JSON report.
pub const REPORT_VERSION: u32 = 2;

/// Full drift-check result.
#[derive(Debug, Serialize)]
pub struct DriftReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Where claims were read from.
    pub claims_source: String,
    /// Claim edges read from the claims file (before dedup).
    pub claims_total: usize,
    /// class → count.
    pub counts: BTreeMap<String, usize>,
    /// How many claims were actually checked against code evidence.
    pub coverage: Coverage,
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

/// Claim-check coverage — distinguishes "docs match code" from
/// "tool could not check". A claim counts as checked when it got a real
/// verdict (`confirmed`/`phantom`/`reversed`); `structural` and
/// `unverifiable` do not.
#[derive(Debug, Serialize)]
pub struct Coverage {
    /// Claims with a real verdict.
    pub checked: usize,
    /// Claim edges after normalization/dedup.
    pub total: usize,
    /// `checked / total` in 0.0–1.0 (0 when there are no claims).
    pub ratio: f64,
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
/// individually; the per-reason totals print either way.
pub fn render(r: &DriftReport, verbose: bool) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "claims: {} ({} edges)", r.claims_source, r.claims_total);
    let pct = (r.coverage.ratio * 100.0).round() as u32;
    let unver = r.counts.get("unverifiable").copied().unwrap_or(0);
    let _ = writeln!(
        s,
        "coverage: {}/{} claims checked ({pct}%) — {unver} unverifiable",
        r.coverage.checked, r.coverage.total
    );

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
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drift::findings::finding_id;

    fn finding(class: FindingClass, reason: &str) -> Finding {
        Finding {
            id: finding_id(class, "a", "b"),
            class,
            from: "a".to_string(),
            to: "b".to_string(),
            kind: Some("import".to_string()),
            importance: Some(3),
            reason: reason.to_string(),
            detail: None,
            evidence: vec![],
            in_baseline: false,
        }
    }

    #[test]
    fn render_shows_coverage_and_reason_totals() {
        let r = DriftReport {
            schema_version: REPORT_VERSION,
            claims_source: "claims.json".to_string(),
            claims_total: 4,
            counts: BTreeMap::from([
                ("confirmed".to_string(), 2),
                ("unverifiable".to_string(), 1),
                ("phantom".to_string(), 1),
            ]),
            coverage: Coverage {
                checked: 3,
                total: 4,
                ratio: 0.75,
            },
            hubs: vec![],
            filtered: BTreeMap::new(),
            findings: vec![
                finding(FindingClass::Confirmed, "direct_evidence"),
                finding(FindingClass::Confirmed, "direct_evidence"),
                finding(FindingClass::Unverifiable, "kind_not_checkable"),
                finding(FindingClass::Phantom, "no_evidence"),
            ],
            baseline: None,
        };
        let s = render(&r, false);
        assert!(
            s.contains("coverage: 3/4 claims checked (75%) — 1 unverifiable"),
            "{s}"
        );
        // -v lists each unverifiable edge AND keeps the reason totals.
        let v = render(&r, true);
        assert!(v.contains("a -> b [kind_not_checkable]"), "{v}");
        assert!(v.contains("(1 kind_not_checkable)"), "{v}");
    }
}
