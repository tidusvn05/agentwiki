//! Finding types — one classified edge (claimed or discovered).

use serde::Serialize;

/// Coarse verdict bucket for one edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    /// Claim backed by code evidence (direct / transitive / containment).
    Confirmed,
    /// Structurally true claim with no import evidence (e.g. parent→child).
    Structural,
    /// Claim cannot be checked (unknown endpoint, kind, coverage…).
    Unverifiable,
    /// Real code edge missing from claims.
    Undocumented,
    /// Claimed A→B but the code has B→A.
    Reversed,
    /// Claimed A→B with no code evidence at all.
    Phantom,
}

impl FindingClass {
    /// Classes worth recording in the baseline — `phantom`/`reversed`
    /// gate `--strict`, `undocumented` is tracked so its `in_baseline`
    /// flag still works. Confirmed/structural/unverifiable ids would
    /// only churn the file.
    pub fn is_gating(&self) -> bool {
        matches!(self, Self::Undocumented | Self::Reversed | Self::Phantom)
    }

    /// Stable label for reports.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Structural => "structural",
            Self::Unverifiable => "unverifiable",
            Self::Undocumented => "undocumented",
            Self::Reversed => "reversed",
            Self::Phantom => "phantom",
        }
    }
}

/// One `file:line` sample backing a finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct EvidenceRef {
    /// Importer file (repo-relative).
    pub file: String,
    /// Resolved target file (repo-relative).
    pub target: String,
    /// 1-based line of the import/reference.
    pub line: u32,
}

/// One classified edge.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `<class>:<from_node>-><to_node>` — kind-free so an LLM relabel of
    /// `dependency_type` never mints a new finding.
    pub id: String,
    /// Verdict bucket.
    pub class: FindingClass,
    /// Source node path (normalized claim endpoint, or code-side importer).
    pub from: String,
    /// Target node path.
    pub to: String,
    /// Claimed `dependency_type` label, when the edge came from claims.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Claimed importance (1–5), when the edge came from claims.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance: Option<u8>,
    /// Filter/check that produced the verdict (e.g. `direct_evidence`).
    pub reason: String,
    /// Extra context (transitive path, coverage numbers…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Up to 3 sample `file:line` evidence refs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    /// Present in the baseline file.
    #[serde(default)]
    pub in_baseline: bool,
}

/// `<class>:<from>-><to>` — stable across runs and kind relabels.
pub fn finding_id(class: FindingClass, from: &str, to: &str) -> String {
    format!("{}:{}->{}", class.as_str(), from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_kind_free() {
        assert_eq!(
            finding_id(FindingClass::Phantom, "src/a", "src/b"),
            "phantom:src/a->src/b"
        );
        // Kind is not part of the id — a relabelled claim keeps its id.
        assert_ne!(
            finding_id(FindingClass::Phantom, "src/a", "src/b"),
            finding_id(FindingClass::Reversed, "src/a", "src/b")
        );
    }
}
