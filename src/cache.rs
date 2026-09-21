//! Content-hash cache: `sha256(prompt ‖ model ‖ backend ‖ inputs ‖
//! SCHEMA_VERSION)` → `.agentwiki/cache/<key>.json`.
//!
//! `inputs` covers what the agent can read but the prompt does not embed
//! — in agentic mode the file-content fingerprint (per-dir subtree for
//! `dir_summary`, whole-repo otherwise). In embedded mode the prompt is
//! the complete input, so `inputs` stays empty.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Bumped when prompt/schema semantics change; part of every cache key.
/// v3: PerDomain dep projection + agentic-mode input fingerprints.
pub const SCHEMA_VERSION: &str = "3";

/// On-disk record for one cached call.
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Raw text returned by the backend.
    pub text: String,
    /// Provenance.
    pub meta: CacheMeta,
}

/// Provenance metadata stored alongside each cached result.
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Agent name.
    pub agent: String,
    /// Backend kind.
    pub backend: String,
    /// Model string used.
    pub model: String,
    /// ISO-ish UTC timestamp.
    pub created_at: String,
    /// Backend call duration in seconds.
    pub secs: f64,
}

/// Filesystem cache rooted at `<internal>/cache`.
pub struct Cache {
    dir: PathBuf,
    /// `--no-cache`: skip both reads and writes.
    disabled: bool,
    /// `--force-regenerate`: skip reads, still write.
    no_read: bool,
}

impl Cache {
    /// Create a cache under `internal_dir`.
    pub fn new(internal_dir: &Path, disabled: bool, no_read: bool) -> Self {
        Self {
            dir: internal_dir.join("cache"),
            disabled,
            no_read,
        }
    }

    /// The cache key for a call. `inputs` is a content fingerprint of
    /// anything the agent may read beyond the prompt itself (agentic
    /// mode); pass `""` when the prompt is the complete input.
    pub fn key(prompt: &str, model: &str, backend: &str, inputs: &str) -> String {
        let mut h = Sha256::new();
        h.update(prompt.as_bytes());
        h.update(b"\x00");
        h.update(model.as_bytes());
        h.update(b"\x00");
        h.update(backend.as_bytes());
        h.update(b"\x00");
        h.update(inputs.as_bytes());
        h.update(b"\x00");
        h.update(SCHEMA_VERSION.as_bytes());
        hex::encode(h.finalize())
    }

    /// Look up a cached result.
    pub fn get(&self, key: &str) -> Option<CacheEntry> {
        if self.disabled || self.no_read {
            return None;
        }
        let path = self.dir.join(format!("{key}.json"));
        let text = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Persist a result.
    pub fn put(&self, key: &str, text: &str, meta: CacheMeta) -> Result<()> {
        if self.disabled {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::io(&self.dir, e))?;
        let path = self.dir.join(format!("{key}.json"));
        let body = serde_json::to_string(&CacheEntry {
            text: text.to_string(),
            meta,
        })
        .map_err(|e| Error::Pipeline(format!("cache serialize: {e}")))?;
        crate::util::write_atomic(&path, body.as_bytes())
    }
}

/// Cache-hit bookkeeping for the summary report.
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Calls served from cache.
    pub hits: usize,
    /// Calls that hit the backend.
    pub misses: usize,
    /// Wall time saved by hits (sum of original durations).
    pub saved: Duration,
}
