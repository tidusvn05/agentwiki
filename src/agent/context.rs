//! `ResearchContext` — typed store replacing deepwiki-rs's "Memory".

use std::collections::HashMap;
use std::path::Path;

use serde::de::DeserializeOwned;
use tokio::sync::RwLock;

use crate::error::{Error, Result};

/// Async shared store: `agent_name` (or `name@target`) → JSON value.
///
/// Research-phase writes are read by later agents through their `deps`;
/// compose-phase writes are the final markdown documents.
#[derive(Debug, Default)]
pub struct ResearchContext {
    inner: RwLock<HashMap<String, serde_json::Value>>,
}

impl ResearchContext {
    /// Empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a result under `key`.
    pub async fn insert(&self, key: &str, value: serde_json::Value) {
        self.inner.write().await.insert(key.to_string(), value);
    }

    /// Raw JSON value for `key`.
    pub async fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.inner.read().await.get(key).cloned()
    }

    /// Typed view of a stored result.
    pub async fn get_typed<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get(key)
            .await
            .and_then(|v| serde_json::from_value(v).ok())
    }

    /// Whether `key` exists.
    pub async fn contains(&self, key: &str) -> bool {
        self.inner.read().await.contains_key(key)
    }

    /// All keys (sorted) — used by `--dry-run` and reports.
    pub async fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.inner.read().await.keys().cloned().collect();
        k.sort();
        k
    }

    /// Snapshot as a plain map.
    pub async fn snapshot(&self) -> HashMap<String, serde_json::Value> {
        self.inner.read().await.clone()
    }

    /// Persist to disk for `--skip-research` reuse.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let map = self.snapshot().await;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let body = serde_json::to_string_pretty(&map)
            .map_err(|e| Error::Pipeline(format!("research ctx serialize: {e}")))?;
        crate::util::write_atomic(path, body.as_bytes())
    }

    /// Load a previously saved context.
    pub async fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let map: HashMap<String, serde_json::Value> =
            serde_json::from_str(&text).map_err(|e| Error::Parse {
                agent: "research.json".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            inner: RwLock::new(map),
        })
    }
}
