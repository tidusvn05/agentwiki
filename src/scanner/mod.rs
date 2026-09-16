//! Phase 0 — deterministic repository scan (no AI calls).

pub mod files;
pub mod insights;
pub mod structure;

use std::path::Path;

use crate::config::Config;
use crate::error::{Error, Result};

pub use files::{FileEntry, scan_files};
pub use insights::{FileStatics, extract};
pub use structure::{DirectoryInfo, ScanData, build_structure};

/// Run the whole scan phase: walk → structure → docs → per-file statics.
///
/// Statics are computed lazily by [`FileStatics::for_entry`] inside prompt
/// building; this function only collects files + README.
pub fn scan(config: &Config) -> Result<ScanData> {
    let root = Path::new(&config.project_path)
        .canonicalize()
        .map_err(|e| Error::io(&config.project_path, e))?;
    let files = scan_files(&root, &config.scan, Path::new(&config.output_path))?;
    let mut data = build_structure(&root, files);
    data.readme = structure::extract_docs(&root, &data.files);
    tracing::info!(
        files = data.files.len(),
        dirs = data.directories.len(),
        "scan complete"
    );
    Ok(data)
}
