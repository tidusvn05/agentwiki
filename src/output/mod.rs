//! Output stage: deterministic doc renderers, doc-tree writer, verifier,
//! summary report.

pub mod boundary;
pub mod database;
pub mod summary;
pub mod verify;
pub mod writer;

pub use boundary::boundary_doc;
pub use database::database_doc;
pub use summary::write_summary;
pub use verify::{VerifyReport, verify};
pub use writer::write_docs;
