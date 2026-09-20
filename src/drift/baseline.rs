//! Drift baseline — `.agentwiki-drift-baseline.json` at the repo root.
//!
//! The baseline records finding ids (phantom/reversed/undocumented) that
//! are already known, so `--strict` only fails on *new* strong signals.
//! The leading dot keeps it out of the scanner (`include_hidden=false`).

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Current baseline schema.
pub const BASELINE_VERSION: u32 = 1;

/// On-disk baseline: sorted finding ids.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Baseline {
    /// Schema version.
    pub schema_version: u32,
    /// Known finding ids.
    pub findings: BTreeSet<String>,
}

impl Baseline {
    /// Load from `path`; `Ok(None)` when the file doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let b: Baseline = serde_json::from_str(&text).map_err(|e| Error::Parse {
            agent: format!("baseline {}", path.display()),
            message: e.to_string(),
        })?;
        Ok(Some(b))
    }

    /// Write sorted ids atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Pipeline(format!("baseline serialize: {e}")))?;
        crate::util::write_atomic(path, body.as_bytes())
    }

    /// Build from a set of finding ids.
    pub fn from_ids(ids: BTreeSet<String>) -> Self {
        Self {
            schema_version: BASELINE_VERSION,
            findings: ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("baseline.json");
        assert!(Baseline::load(&path).unwrap().is_none());

        let b = Baseline::from_ids(BTreeSet::from([
            "phantom:a->b".to_string(),
            "reversed:c->d".to_string(),
        ]));
        b.save(&path).unwrap();
        let loaded = Baseline::load(&path).unwrap().unwrap();
        assert_eq!(loaded.findings.len(), 2);
        assert!(loaded.findings.contains("phantom:a->b"));

        // Corrupt file → error, not silent None.
        std::fs::write(&path, "{oops").unwrap();
        assert!(Baseline::load(&path).is_err());
    }
}
