//! Agent layer: specs (DAG nodes), the runner engine, shared context,
//! prompt materials, and structured report types.

pub mod context;
pub mod materials;
pub mod registry;
pub mod reports;
pub mod runner;
pub mod spec;

pub use context::ResearchContext;
pub use runner::run_spec;
pub use spec::{AgentSpec, ExecKind, FanOut, FanTarget, Material, Phase};
