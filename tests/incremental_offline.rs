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

/// Prompt-sensitive backend: canned bodies get a `_h` field (JSON) or a
/// trailing marker (markdown) carrying a hash of the request prompt —
/// so a changed prompt anywhere upstream propagates into the doc bodies,
/// unlike `canned`, which answers every prompt with the same bytes.
/// Agents without a canned body still get `{}` unchanged.
fn hashing_backend() -> MockBackend {
    let map: Vec<(String, String)> = CANNED
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    MockBackend::new(move |req| {
        let body = map
            .iter()
            .find(|(k, _)| req.agent == *k)
            .or_else(|| {
                map.iter().find(|(k, _)| {
                    req.agent
                        .strip_prefix(k.as_str())
                        .is_some_and(|rest| rest.starts_with('@'))
                })
            })
            .map(|(_, v)| v.clone());
        let Some(body) = body else {
            return Ok("{}".to_string());
        };
        use sha2::Digest;
        let tag = &hex::encode(sha2::Sha256::digest(&req.prompt))[..12];
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(mut v) if v.is_object() => {
                v["_h"] = tag.into();
                Ok(v.to_string())
            }
            _ => Ok(format!("{body}\n\n<!-- prompt:{tag} -->")),
        }
    })
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

/// The invariant that holds: an incremental *structural rerun* produces
/// the same doc tree as a fresh full run on the same tree. The prompt-
/// hashing backend gives this teeth — any step the rerun wrongly skipped
/// or fed stale ctx to changes a prompt, which changes the doc bytes.
/// (A cosmetic *skip* deliberately does not regenerate docs, so "skip ≡
/// full run" is not claimed — see `cosmetic_change_incremental_noop`
/// for the preservation contract instead.)
#[tokio::test(flavor = "multi_thread")]
async fn structural_rerun_matches_fresh_full_run() {
    let tmp = tempfile::tempdir().unwrap();
    // Both sides share one project dir — prompts embed the root's
    // name/path, so two copies at different paths can never produce
    // identical docs. Separate internal/output dirs keep each run's
    // cache+manifest state independent.
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);

    // A: full run, structural edit, incremental run (must re-run).
    let cfg_a = test_config(&proj, &tmp.path().join("ta"), true);
    std::fs::create_dir_all(tmp.path().join("ta")).unwrap();
    let pctx = PipelineCtx::new(
        cfg_a.clone(),
        Some(mock_backends(Arc::new(hashing_backend()))),
    )
    .await
    .unwrap();
    run(&pctx).await.unwrap();
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\nfrom .api import TaskAPI\n");
    std::fs::write(&f, body).unwrap();
    let pctx2 = PipelineCtx::new(cfg_a, Some(mock_backends(Arc::new(hashing_backend()))))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    // B: single full run on the same mutated tree.
    let cfg_b = test_config(&proj, &tmp.path().join("tb"), false);
    std::fs::create_dir_all(tmp.path().join("tb")).unwrap();
    let pctx_b = PipelineCtx::new(cfg_b, Some(mock_backends(Arc::new(hashing_backend()))))
        .await
        .unwrap();
    run(&pctx_b).await.unwrap();

    let (a, b) = (
        docs_snapshot(&tmp.path().join("ta/docs")),
        docs_snapshot(&tmp.path().join("tb/docs")),
    );
    let mut msg = String::new();
    for k in a
        .keys()
        .chain(b.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        match (a.get(k), b.get(k)) {
            (None, Some(_)) => msg.push_str(&format!("\n  only in fresh run: {k}")),
            (Some(_), None) => msg.push_str(&format!("\n  only in incremental run: {k}")),
            (Some(x), Some(y)) if x != y => msg.push_str(&format!("\n  content differs: {k}")),
            _ => {}
        }
    }
    assert!(msg.is_empty(), "structural rerun ≡ fresh full run:{msg}");
}

/// F2 regression: deleted docs must break the cosmetic no-op, not be
/// reported as "fresh". The written-docs list is the ground truth for
/// what should exist — a missing entry fails open to a real run.
#[tokio::test(flavor = "multi_thread")]
async fn deleted_docs_break_cosmetic_noop() {
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

    let out = tmp.path().join("docs");
    let overview = out.join("1.Overview.md");
    let deep = out.join("4.Deep-Exploration/Task Management.md");
    assert!(overview.is_file() && deep.is_file());
    std::fs::remove_file(&overview).unwrap();
    std::fs::remove_file(&deep).unwrap();

    // Purely cosmetic source edit — classification still says Cosmetic.
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\n# cosmetic comment\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg, Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    assert!(
        call_log(&mock).len() > calls_after_first,
        "missing docs must force a real run, not a cosmetic no-op"
    );
    assert!(overview.is_file(), "deleted doc must be regenerated");
    assert!(deep.is_file(), "deleted deep-dive must be regenerated");
}

/// F5 regression: stale deep-dive cleanup is bounded by the written-docs
/// record — a `.md` the user dropped into `4.Deep-Exploration/` is not
/// agentwiki's and must survive regeneration.
#[tokio::test(flavor = "multi_thread")]
async fn user_files_in_deep_exploration_survive() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);
    let mock = Arc::new(MockBackend::canned(CANNED));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx).await.unwrap();

    let user_doc = tmp.path().join("docs/4.Deep-Exploration/My Notes.md");
    std::fs::write(&user_doc, "hand-written notes — do not delete").unwrap();

    // A structural change forces a real rerun through write_docs.
    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\nfrom .api import TaskAPI\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg, Some(mock_backends(mock)))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    assert!(
        user_doc.is_file(),
        "user-authored docs must never be deleted by stale-doc cleanup"
    );
}

/// F1 regression: a `research.json` newer than the written-docs record
/// means the last run saved research but never finished compose+write
/// (interrupted mid-compose). The gate must refuse to no-op over docs
/// describing older research.
#[tokio::test(flavor = "multi_thread")]
async fn stale_written_docs_break_cosmetic_noop() {
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

    // Simulate "research saved, compose never completed": push
    // research.json's mtime past the written-docs record.
    let research = cfg.internal_path.join("research.json");
    std::fs::File::options()
        .write(true)
        .open(&research)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();

    let f = proj.join("src/models.py");
    let mut body = std::fs::read_to_string(&f).unwrap();
    body.push_str("\n# cosmetic comment\n");
    std::fs::write(&f, body).unwrap();

    let pctx2 = PipelineCtx::new(cfg, Some(mock_backends(mock.clone())))
        .await
        .unwrap();
    run(&pctx2).await.unwrap();

    assert!(
        call_log(&mock).len() > calls_after_first,
        "research newer than the written-docs record must force a real run"
    );
}

/// F1 regression, ordering side: a run that fails during compose must
/// leave neither a manifest claiming the tree nor a written-docs record.
#[tokio::test(flavor = "multi_thread")]
async fn failed_compose_writes_no_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("app");
    copy_dir(&fixture_dir(), &proj);
    let cfg = test_config(&proj, tmp.path(), true);

    let canned: Vec<(String, String)> = CANNED
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mock = Arc::new(MockBackend::new(move |req| {
        if req.agent == "overview" {
            return Err("simulated compose failure".to_string());
        }
        Ok(canned
            .iter()
            .find(|(k, _)| req.agent == *k)
            .or_else(|| {
                canned.iter().find(|(k, _)| {
                    req.agent
                        .strip_prefix(k.as_str())
                        .is_some_and(|rest| rest.starts_with('@'))
                })
            })
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "{}".to_string()))
    }));

    let pctx = PipelineCtx::new(cfg.clone(), Some(mock_backends(mock)))
        .await
        .unwrap();
    assert!(
        run(&pctx).await.is_err(),
        "compose failure must fail the run"
    );
    assert!(
        !manifest::manifest_path(&cfg).is_file(),
        "manifest must not claim a tree whose docs were never written"
    );
    assert!(
        !agentwiki::output::writer::written_docs_path(&cfg).is_file(),
        "a failed write must leave no written-docs record"
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
