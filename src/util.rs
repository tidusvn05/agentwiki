//! Small shared filesystem helpers.

use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// then rename. A crash leaves either the old or the new content — a
/// reader never sees a torn file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| Error::io(dir, e))?;
    tmp.write_all(bytes).map_err(|e| Error::io(path, e))?;
    tmp.persist(path).map_err(|e| Error::io(path, e.error))?;
    Ok(())
}
