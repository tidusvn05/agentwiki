//! Offline two-run incremental tests: run 1 (full) → mutate a fixture
//! copy → run 2 (incremental) → assert which calls re-ran, plus the
//! golden invariant — a cosmetic-skip run leaves docs byte-identical to
//! a fresh full run on the same tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentwiki::backend::mock::MockBackend;
use agentwiki::backend::{AgentBackend, BackendKind};
use agentwiki::config::{Config, Mode};
use agentwiki::manifest;
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
        r#"{"database_projects":[],"tables":[],"views":[],"stored_procedures":[],"database_functions":[],"table_relationships":[],"data_flows":[],"confidence_score":7}"#,
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
    (
        "overview",
        "# System Context Overview\n\nfixture-app manages tasks.",
    ),
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

/// Copy the fixture — tests mutate it, and the shared tree must stay
/// pristine for `pipeline_offline` (plus parallel test runs).
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let (s, d) = (e.path(), dst.join(e.file_name()));
        if s.is_dir() {
            copy_dir(&s, &d);
        } else {
            std::fs::copy(&s, &d).unwrap();
        }
    }
}

fn test_config(project: &Path, tmp: &Path, incremental: bool) -> Config {
    let mut config = Config {
        project_path: project.to_path_buf(),
        output_path: tmp.join("docs"),
        internal_path: tmp.join(".agentwiki"),
        incremental,
        ..Default::default()
    };
    config.models.efficient = "mock".to_string();
    config.models.powerful = "mock".to_string();
    config.scan.git_tracked_only = false;
    config.limits.daily_cap = 200;
    config
}

fn mock_backends(mock: Arc<MockBackend>) -> HashMap<BackendKind, Arc<dyn AgentBackend>> {
    HashMap::from([(BackendKind::Mock, mock as Arc<dyn AgentBackend>)])
}

fn call_log(mock: &MockBackend) -> Vec<String> {
    mock.calls.lock().unwrap().clone()
}

/// Doc-tree contents, excluding the summary (it embeds per-run stats).
fn docs_snapshot(dir: &Path) -> HashMap<String, Vec<u8>> {
    fn walk(d: &Path, base: &Path, out: &mut HashMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                if rel != "__AgentWiki_Summary__.md" {
                    out.insert(rel, std::fs::read(&p).unwrap());
                }
            }
        }
    }
    let mut out = HashMap::new();
    walk(dir, dir, &mut out);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn cosmetic_change_incremental_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    let calls_after_first = call_log(&mock).len();
    assert!(calls_after_first > 0);
    let manifest_file = manifest::manifest_path(&cfg);
    assert!(manifest_file.is_file(), "run must write a manifest");
    let manifest_before = std::fs::read(&manifest_file).unwrap();
    let docs_before = docs_snapshot(&tmp.path().join("docs"));

    // Comment-only edit: file hash changes, no import-graph delta.
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\n# cosmetic comment\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg, Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    assert_eq!(
        call_log(&mock).len(),
        calls_after_first,
        "cosmetic diff must be a 0-call no-op"
    );
    // The manifest is NOT absorbed: the pending cosmetic delta stays
    // visible to `status` until a real run documents it.
    assert_eq!(std::fs::read(&manifest_file).unwrap(), manifest_before);
    assert_eq!(docs_snapshot(&tmp.path().join("docs")), docs_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn structural_change_reruns_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    let calls_after_first = call_log(&mock).len();
    let manifest_before = std::fs::read(manifest::manifest_path(&cfg)).unwrap();

    // New internal edge: models.py now imports api.
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\nfrom .api import TaskAPI\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    assert!(
        call_log(&mock).len() > calls_after_first,
        "structural diff must run the pipeline"
    );
    // The manifest advanced to the new tree.
    assert_ne!(
        std::fs::read(manifest::manifest_path(&cfg)).unwrap(),
        manifest_before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn env_change_forces_full_run() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    let calls_after_first = call_log(&mock).len();

    // Same code, different language → env fingerprint changes → full run.
    let mut cfg_vi = cfg;
    cfg_vi.target_language = agentwiki::TargetLanguage::Vi;
    let pctx2 = PipelineCtx::new(cfg_vi, Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();
    assert!(
        call_log(&mock).len() > calls_after_first,
        "a language change must not classify as cosmetic"
    );
}

/// The golden invariant: incremental-skip output == fresh full-run
/// output on the same tree.
#[tokio::test(flavor = "multi_thread")]
async fn incremental_skip_matches_fresh_full_run() {
    let tmp = tempfile::tempdir().unwrap();
    let proj_a = tmp.path().join("a");
    let proj_b = tmp.path().join("b");
    copy_dir(&fixture_dir(), &proj_a);
    copy_dir(&fixture_dir(), &proj_b);

    // A: full run, cosmetic edit, incremental run (skips).
    let cfg_a = test_config(&proj_a, &tmp.path().join("ta"), true);
    std::fs::create_dir_all(tmp.path().join("ta")).unwrap();
    let mock_a = Arc::new(MockBackend::canned(CANNED));
    let pctx = PipelineCtx::new(cfg_a.clone(), Some(mock_backends(mock_a)))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    for p in [&proj_a, &proj_b] {
        let f = p.join("src/models.py");
        let mut body = std::fs::read_to_string(&f).unwrap();
        body.push_str("\n# same cosmetic edit\n");
        std::fs::write(&f, body).unwrap();
    }
    let pctx2 = PipelineCtx::new(
        cfg_a,
        Some(mock_backends(Arc::new(MockBackend::canned(CANNED)))),
    )
    .await
    .unwrap();
    run(&pctx2).await.unwrap();

    // B: single full run on the mutated tree.
    let cfg_b = test_config(&proj_b, &tmp.path().join("tb"), false);
    std::fs::create_dir_all(tmp.path().join("tb")).unwrap();
    let mock_b = Arc::new(MockBackend::canned(CANNED));
    let pctx_b = PipelineCtx::new(cfg_b, Some(mock_backends(mock_b)))
        .await
        .unwrap();
    run(&pctx_b).await.unwrap();

    assert_eq!(
        docs_snapshot(&tmp.path().join("ta/docs")),
        docs_snapshot(&tmp.path().join("tb/docs")),
        "incremental skip must leave the same doc tree a full run would write"
    );
}

/// Agentic-mode cache fix: file content never enters the prompt, so the
/// manifest fingerprint must — a comment-only edit past the source
/// preview must still bust `dir_summary@src` (and every global agent),
/// while untouched directories stay cached.
#[tokio::test(flavor = "multi_thread")]
async fn agentic_cache_tracks_file_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let mut cfg = test_config(&proj, tmp.path(), false);
    cfg.mode = Mode::Agentic;
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx).await.unwrap();
    let calls_after_first = call_log(&mock).len();

    // Append a comment past the embedded preview — the prompt does not
    // change; only the fingerprint can catch this.
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\n# trailing comment\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg, Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();
    let new_calls: Vec<String> = call_log(&mock)
        .into_iter()
        .skip(calls_after_first)
        .collect();

    assert!(
        new_calls.iter().any(|c| c == "dir_summary@src"),
        "changed dir must re-run in agentic mode: {new_calls:?}"
    );
    assert!(
        new_calls.iter().any(|c| c == "system_context"),
        "global agents must re-run (repo-wide read scope)"
    );
    assert!(
        !new_calls.iter().any(|c| c == "dir_summary@db"),
        "untouched dir must stay cached: {new_calls:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn status_after_run_and_pending_cosmetic() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);
    let mock = Arc::new(MockBackend::canned(CANNED));
    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock)))
        .await
        .unwrap();
    run(&pctx).await.unwrap();

    // Point status at the same internal/output paths via a config file —
    // the manifest slot is keyed by the absolute output path.
    let cfg_toml = proj.join("agentwiki.toml");
    std::fs::write(
        &cfg_toml,
        format!(
            "internal_path = \"{}\"\noutput_path = \"{}\"\n[scan]\ngit_tracked_only = false\n",
            cfg.internal_path.display(),
            cfg.output_path.display(),
        ),
    )
    .unwrap();

    // Baseline exists → status runs clean.
    assert_eq!(
        agentwiki::status::run(Some(proj.clone()), Some(cfg_toml.clone()), None, false).await,
        0
    );

    // Cosmetic edit → status still exits 0 (report content goes to stdout).
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\n# pending cosmetic\n");
    std::fs::write(&f, body).unwrap();
    assert_eq!(
        agentwiki::status::run(Some(proj), Some(cfg_toml), None, true).await,
        0
    );
}
