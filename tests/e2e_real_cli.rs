//! Real-CLI end-to-end tests — ignored by default.
//!
//! Run one explicitly, e.g.:
//!   AGENTWIKI_E2E=1 cargo test --test e2e_real_cli devin -- --ignored
//!
//! Each test runs the full pipeline on `tests/fixture-app` with the real
//! authenticated CLI. They spend real quota — keep `daily_cap` low.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agentwiki::backend::{AgentBackend, BackendKind, for_kind};
use agentwiki::config::Config;
use agentwiki::{PipelineCtx, run};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-app")
}

fn enabled() -> bool {
    std::env::var("AGENTWIKI_E2E").is_ok_and(|v| v == "1")
}

async fn run_e2e(model: &str, kind: BackendKind) {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config {
        project_path: fixture_dir(),
        output_path: tmp.path().join("docs"),
        internal_path: tmp.path().join(".agentwiki"),
        ..Default::default()
    };
    config.models.efficient = model.to_string();
    config.models.powerful = model.to_string();
    config.scan.git_tracked_only = false;
    // Keep the spend small: a handful of calls for a tiny fixture.
    config.limits.daily_cap = 20;
    config.limits.call_timeout_s = 300;

    let backends: HashMap<BackendKind, Arc<dyn AgentBackend>> =
        HashMap::from([(kind, for_kind(kind))]);
    let pctx = PipelineCtx::new(config, Some(backends)).await.unwrap();
    run(&pctx).await.unwrap();

    let out = tmp.path().join("docs");
    assert!(out.join("1.Overview.md").is_file());
    assert!(out.join("2.Architecture.md").is_file());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real devin CLI call; enable with AGENTWIKI_E2E=1"]
async fn devin_end_to_end() {
    if !enabled() {
        return;
    }
    run_e2e("devin:swe-2-medium", BackendKind::Devin).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real claude CLI call; enable with AGENTWIKI_E2E=1"]
async fn claude_end_to_end() {
    if !enabled() {
        return;
    }
    run_e2e("claude:sonnet", BackendKind::Claude).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real codex CLI call; enable with AGENTWIKI_E2E=1"]
async fn codex_end_to_end() {
    if !enabled() {
        return;
    }
    run_e2e("codex", BackendKind::Codex).await;
}
