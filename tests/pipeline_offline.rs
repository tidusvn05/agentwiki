//! Offline full-pipeline test: MockBackend + fixture app. No real CLI
//! calls — `cargo test` must stay green on machines without `devin`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agentwiki::backend::mock::MockBackend;
use agentwiki::backend::{AgentBackend, BackendKind};
use agentwiki::config::Config;
use agentwiki::error::Error;
use agentwiki::{PipelineCtx, run};

const CANNED: &[(&str, &str)] = &[
    (
        "dir_summary",
        r#"{"summary":"Core application directory","importance_score":0.9,"key_files":["main.py"],"file_insights":[{"name":"main.py","summary":"CLI entry point","code_purpose":"Entry","importance_score":0.9,"detailed_description":"argparse entry","source_summary":"parses args, dispatches","responsibilities":["parse args"],"interfaces":[],"dependencies":[]}]}"#,
    ),
    (
        "relationships",
        r#"{"core_dependencies":[{"from":"src","to":"db","dependency_type":"DataFlow","importance":4}],"architecture_layers":[{"name":"app","components":["src"],"level":1}],"key_insights":["layered"]}"#,
    ),
    (
        "system_context",
        r#"{"project_name":"fixture-app","project_description":"CLI task manager","project_type":"CLITool","business_value":"Manage tasks","target_users":[{"name":"dev","description":"developer","needs":["tracking"]}],"external_systems":[],"system_boundary":{"scope":"cli","included_components":["src"],"excluded_components":[]},"confidence_score":8}"#,
    ),
    (
        "domain_modules",
        r#"{"domain_modules":[{"name":"Task Management","description":"core domain","domain_type":"Core Business Domain","sub_modules":[{"name":"api","description":"ops","code_paths":["src/api.py"],"key_functions":["add"],"importance":8.0}],"code_paths":["src"],"importance":9.0,"complexity":3.0}],"domain_relations":[],"business_flows":[],"architecture_summary":"layered","confidence_score":8}"#,
    ),
    (
        "database",
        r#"{"database_projects":[],"tables":[{"schema":"main","name":"tasks","columns":[{"name":"id","data_type":"INTEGER","nullable":false,"is_identity":true,"default_value":null}],"primary_key":["id"],"description":"task rows","source_path":"db/schema.sql"}],"views":[],"stored_procedures":[],"database_functions":[],"table_relationships":[],"data_flows":[],"confidence_score":7}"#,
    ),
    (
        "key_module",
        r#"{"domain_name":"","module_name":"api","module_description":"task ops","interaction":"called by main","implementation":"thin wrapper over Storage","associated_files":["src/api.py"],"flowchart_mermaid":"graph TD\n  A-->B","sequence_diagram_mermaid":""}"#,
    ),
    (
        "boundary",
        r#"{"cli_boundaries":[{"command":"taskman","description":"task CLI","arguments":[],"options":[],"examples":["taskman add x"],"source_location":"src/main.py"}],"api_boundaries":[],"router_boundaries":[],"integration_suggestions":[],"confidence_score":8}"#,
    ),
    (
        "architecture",
        "# Architecture Research\n\n```mermaid\ngraph TD\n  CLI-->Storage\n```",
    ),
    (
        "workflow",
        "# System Workflow Analysis\n\n## 1. Main Workflow\nadd → validate → persist",
    ),
    ("overview", "# System Context Overview\n\nfixture-app manages tasks."),
    (
        "architecture_doc",
        "# System Architecture Documentation\n\n```mermaid\nflowchart TD\n  CLI-->API-->Storage\n```",
    ),
    (
        "workflow_doc",
        "# Core Workflows\n\n```mermaid\nsequenceDiagram\n  User->>CLI: add\n  CLI->>Storage: insert\n```",
    ),
    (
        "deep_dive",
        "# Task Management Deep Dive\n\nModule internals here.",
    ),
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-app")
}

fn test_config(tmp: &tempfile::TempDir, daily_cap: u32) -> Config {
    let mut config = Config {
        project_path: fixture_dir(),
        output_path: tmp.path().join("docs"),
        internal_path: tmp.path().join(".agentwiki"),
        ..Default::default()
    };
    config.models.efficient = "mock".to_string();
    config.models.powerful = "mock".to_string();
    config.scan.git_tracked_only = false;
    config.limits.daily_cap = daily_cap;
    config
}

fn mock_backends(mock: Arc<MockBackend>) -> HashMap<BackendKind, Arc<dyn AgentBackend>> {
    HashMap::from([(BackendKind::Mock, mock as Arc<dyn AgentBackend>)])
}

#[tokio::test(flavor = "multi_thread")]
async fn full_pipeline_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let mock = Arc::new(MockBackend::canned(CANNED));
    let pctx = PipelineCtx::new(test_config(&tmp, 100), Some(mock_backends(mock.clone())))
        .await
        .unwrap();

    run(&pctx).await.unwrap();

    let out = tmp.path().join("docs");
    for f in [
        "1.Overview.md",
        "2.Architecture.md",
        "3.Workflow.md",
        "5.Boundary-Interfaces.md",
        "6.Database-Overview.md",
        "__AgentWiki_Summary__.md",
    ] {
        assert!(out.join(f).is_file(), "missing {f}");
    }
    assert!(
        out.join("4.Deep-Exploration/Task Management.md").is_file(),
        "missing deep-dive doc"
    );

    // Cache + audit artifacts exist.
    let internal = tmp.path().join(".agentwiki");
    assert!(internal.join("calls.jsonl").is_file());
    assert!(internal.join("research.json").is_file());
    assert!(internal.join("cache").is_dir());
    assert!(mock.calls.lock().unwrap().len() > 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn daily_cap_blocks_calls() {
    let tmp = tempfile::tempdir().unwrap();
    // fixture has 3 dirs (root, src, db) → 3 dir_summary instances; cap=1.
    let mock = Arc::new(MockBackend::canned(CANNED));
    let pctx = PipelineCtx::new(test_config(&tmp, 1), Some(mock_backends(mock)))
        .await
        .unwrap();

    let err = run(&pctx).await.unwrap_err();
    assert!(
        matches!(err, Error::QuotaExceeded { .. }),
        "expected QuotaExceeded, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn second_run_is_fully_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(
        test_config(&tmp, 100),
        Some(mock_backends(mock.clone())),
    )
    .await
    .unwrap();
    run(&pctx).await.unwrap();
    let calls_after_first = mock.calls.lock().unwrap().len();
    assert!(calls_after_first > 0);

    // Fresh context, same internal dir → every call served from cache.
    let pctx2 = PipelineCtx::new(
        test_config(&tmp, 100),
        Some(mock_backends(mock.clone())),
    )
    .await
    .unwrap();
    run(&pctx2).await.unwrap();
    assert_eq!(mock.calls.lock().unwrap().len(), calls_after_first);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_aborts_mid_flight() {
    // Slow mock keeps dir_summary in flight; cancelling the token must
    // unwind the pipeline quickly (aborted tasks drop their futures).
    let tmp = tempfile::tempdir().unwrap();
    let mock = Arc::new(
        MockBackend::canned(CANNED).with_delay(Duration::from_secs(30)),
    );
    let pctx = PipelineCtx::new(test_config(&tmp, 100), Some(mock_backends(mock)))
        .await
        .unwrap();

    let pctx2 = pctx.clone();
    let handle = tokio::spawn(async move { run(&pctx2).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    pctx.cancel.cancel();

    let err = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("run did not finish within 5s of cancel")
        .expect("run task panicked")
        .unwrap_err();
    assert!(matches!(err, Error::Cancelled), "expected Cancelled, got {err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn second_concurrent_run_refused() {
    // While one run holds .agentwiki/run.lock, another on the same
    // internal dir must fail fast with AlreadyRunning.
    let tmp = tempfile::tempdir().unwrap();
    let mock = Arc::new(
        MockBackend::canned(CANNED).with_delay(Duration::from_secs(5)),
    );
    let pctx = PipelineCtx::new(
        test_config(&tmp, 100),
        Some(mock_backends(mock.clone())),
    )
    .await
    .unwrap();
    let pctx2 = pctx.clone();
    let first = tokio::spawn(async move { run(&pctx2).await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let pctx_b = PipelineCtx::new(test_config(&tmp, 100), Some(mock_backends(mock)))
        .await
        .unwrap();
    let err = run(&pctx_b).await.unwrap_err();
    assert!(
        matches!(err, Error::AlreadyRunning { .. }),
        "expected AlreadyRunning, got {err:?}"
    );
    first.abort();
    // Awaiting ensures the aborted run dropped its lock guard.
    let _ = first.await;

    // After the first run's lock is gone, a fresh run proceeds.
    let pctx_c = PipelineCtx::new(
        test_config(&tmp, 100),
        Some(mock_backends(Arc::new(MockBackend::canned(CANNED)))),
    )
    .await
    .unwrap();
    run(&pctx_c).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_on_garbage_then_success() {
    // First call returns prose, retry returns valid JSON.
    let tmp = tempfile::tempdir().unwrap();
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = counter.clone();
    let mock = Arc::new(MockBackend::new(move |req| {
        if req.agent == "system_context" && c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Err("simulated garbage".to_string());
        }
        Ok(CANNED
            .iter()
            .find(|(k, _)| req.agent == *k || req.agent.starts_with(&format!("{k}@")))
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| "{}".to_string()))
    }));
    let pctx = PipelineCtx::new(test_config(&tmp, 100), Some(mock_backends(mock)))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    assert!(tmp.path().join("docs/1.Overview.md").is_file());
}
