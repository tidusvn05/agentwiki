//! Offline drift-check tests: `tests/fixture-app` (Python) and
//! `tests/fixture-rs` (Rust). No LLM calls — `drift` is fully static.
//!
//! Claims are built inline with `serde_json::json!` and written to temp
//! files; the fixture trees stay untouched.

use std::path::{Path, PathBuf};

use agentwiki::agent::reports::CoreDependency;
use agentwiki::config::Config;
use agentwiki::drift::config::DriftConfig;
use agentwiki::drift::findings::FindingClass;
use agentwiki::drift::{self, DriftArgs};
use agentwiki::scanner;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

fn cfg_for(project: &Path) -> Config {
    let mut c = Config {
        project_path: project.to_path_buf(),
        output_path: project.join("docs-out"),
        internal_path: project.join(".agentwiki"),
        ..Default::default()
    };
    c.scan.git_tracked_only = false;
    c
}

fn claims_json(deps: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "relationships": { "core_dependencies": deps } })
}

fn parse_claims(v: &serde_json::Value) -> Vec<CoreDependency> {
    serde_json::from_value(v["relationships"]["core_dependencies"].clone()).unwrap()
}

fn analyze_fixture(
    name: &str,
    claims: serde_json::Value,
    tweak: impl FnOnce(&mut DriftConfig),
) -> drift::Outcome {
    let root = fixture(name);
    let cfg = cfg_for(&root);
    let mut dcfg = DriftConfig::default();
    tweak(&mut dcfg);
    let scan = scanner::scan(&cfg).unwrap();
    let claims = parse_claims(&claims);
    let globs: Vec<glob::Pattern> = dcfg
        .test_globs
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    drift::analyze(&dcfg, &root, &scan, &claims, &globs)
}

fn class_of(out: &drift::Outcome, from: &str, to: &str) -> Option<FindingClass> {
    out.findings
        .iter()
        .find(|f| f.from == from && f.to == to)
        .map(|f| f.class)
}

/// The plan's fixture claim set: 4 real edges claimed directly,
/// `storage→schema.sql` unverifiable, `main→models` transitive, plus a
/// reversed and a phantom fake claim.
fn fixture_app_claims() -> serde_json::Value {
    claims_json(serde_json::json!([
        {"from":"src/main.py","to":"src/api.py","dependency_type":"FunctionCall","importance":5},
        {"from":"src/api.py","to":"src/storage.py","dependency_type":"Composition","importance":5},
        {"from":"src/api.py","to":"src/models.py","dependency_type":"Import","importance":4},
        {"from":"src/storage.py","to":"src/models.py","dependency_type":"Import","importance":4},
        {"from":"src/storage.py","to":"db/schema.sql","dependency_type":"DataFlow","importance":4},
        {"from":"src/main.py","to":"src/models.py","dependency_type":"Import","importance":2},
        {"from":"src/models.py","to":"src/api.py","dependency_type":"Import","importance":3},
        {"from":"src/models.py","to":"src/main.py","dependency_type":"Import","importance":2},
    ]))
}

#[test]
fn fixture_app_classifies_edges() {
    let out = analyze_fixture("fixture-app", fixture_app_claims(), |_| {});
    let find = |f: &str, t: &str| class_of(&out, f, t);

    assert_eq!(
        find("src/main.py", "src/api.py"),
        Some(FindingClass::Confirmed)
    );
    assert_eq!(
        find("src/api.py", "src/storage.py"),
        Some(FindingClass::Confirmed)
    );
    assert_eq!(
        find("src/api.py", "src/models.py"),
        Some(FindingClass::Confirmed)
    );
    assert_eq!(
        find("src/storage.py", "src/models.py"),
        Some(FindingClass::Confirmed)
    );
    assert_eq!(
        find("src/main.py", "src/models.py"),
        Some(FindingClass::Confirmed)
    );
    assert_eq!(
        find("src/storage.py", "db/schema.sql"),
        Some(FindingClass::Unverifiable)
    );
    assert_eq!(
        find("src/models.py", "src/api.py"),
        Some(FindingClass::Reversed)
    );
    assert_eq!(
        find("src/models.py", "src/main.py"),
        Some(FindingClass::Phantom)
    );

    // main→storage is real but unclaimed — thin (1 site), default min=3.
    assert!(
        out.findings
            .iter()
            .all(|f| f.class != FindingClass::Undocumented)
    );
    assert_eq!(out.filtered.get("u8_thin").copied().unwrap_or(0), 1);
}

#[test]
fn fixture_app_undocumented_at_min_1() {
    let out = analyze_fixture("fixture-app", fixture_app_claims(), |c| {
        c.min_undocumented_imports = 1;
    });
    assert_eq!(
        class_of(&out, "src/main.py", "src/storage.py"),
        Some(FindingClass::Undocumented)
    );
}

/// `drift::run` exit codes: 0 normal, 1 strict with new findings,
/// 0 after --update-baseline, 2 on missing claims.
#[tokio::test(flavor = "multi_thread")]
async fn fixture_app_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fixture("fixture-app");
    let claims_file = tmp.path().join("claims.json");
    std::fs::write(
        &claims_file,
        serde_json::to_string_pretty(&fixture_app_claims()).unwrap(),
    )
    .unwrap();
    // Keep baseline + drift.json inside tmp, not the fixture.
    let cfg_toml = tmp.path().join("agentwiki.toml");
    std::fs::write(
        &cfg_toml,
        format!(
            "internal_path = \"{}\"\noutput_path = \"{}\"\n[scan]\ngit_tracked_only = false\n[drift]\nbaseline_path = \"{}\"\n",
            tmp.path().join("internal").display(),
            tmp.path().join("docs").display(),
            tmp.path().join("baseline.json").display(),
        ),
    )
    .unwrap();

    let args = |extra: &[&str]| {
        let mut a = DriftArgs {
            project_path: Some(root.clone()),
            config: Some(cfg_toml.clone()),
            claims: Some(claims_file.clone()),
            ..Default::default()
        };
        for f in extra {
            match *f {
                "--strict" => a.strict = true,
                "--update-baseline" => a.update_baseline = true,
                _ => {}
            }
        }
        a
    };
    async fn run(a: &DriftArgs) -> i32 {
        drift::run(a.project_path.clone(), a.config.clone(), a, false).await
    }

    assert_eq!(run(&args(&[])).await, 0, "plain run must exit 0");
    assert_eq!(
        run(&args(&["--strict"])).await,
        1,
        "strict with new phantom+reversed must exit 1"
    );
    assert_eq!(run(&args(&["--update-baseline"])).await, 0);
    assert_eq!(
        run(&args(&["--strict"])).await,
        0,
        "strict after baseline update must exit 0"
    );

    // Baseline only stores gating classes — confirmed/unverifiable ids
    // would just churn the committed file.
    let baseline: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.path().join("baseline.json")).unwrap())
            .unwrap();
    let ids: Vec<&str> = baseline["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "baseline should hold only phantom+reversed");
    assert!(
        ids.iter().all(|id| {
            id.starts_with("phantom:")
                || id.starts_with("reversed:")
                || id.starts_with("undocumented:")
        }),
        "unexpected baseline ids: {ids:?}"
    );

    // `--baseline` overrides the config path for both write and read.
    let alt = tmp.path().join("alt-baseline.json");
    let mut a = args(&["--update-baseline"]);
    a.baseline = Some(alt.clone());
    assert_eq!(run(&a).await, 0);
    assert!(alt.is_file(), "--baseline must redirect the write");
    let mut a = args(&["--strict"]);
    a.baseline = Some(alt);
    assert_eq!(run(&a).await, 0, "--baseline must redirect the read");

    let missing = DriftArgs {
        project_path: Some(root.clone()),
        config: Some(cfg_toml.clone()),
        claims: Some(tmp.path().join("nope.json")),
        ..Default::default()
    };
    assert_eq!(run(&missing).await, 2);

    // Coverage distinguishes "checked" (confirmed/phantom/reversed)
    // from "couldn't check" (structural/unverifiable). Fixture: 5
    // confirmed + 1 reversed + 1 phantom checked of 8 claims; 1
    // unverifiable (the DataFlow edge).
    let drift_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join("internal/drift.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(drift_json["coverage"]["checked"], 7);
    assert_eq!(drift_json["coverage"]["total"], 8);
    assert_eq!(drift_json["coverage"]["ratio"], 0.875);
    assert_eq!(drift_json["counts"]["unverifiable"], 1);
}

/// `.agentwiki/` doesn't exist on a fresh checkout — `drift` must create
/// it rather than lose `drift.json` (regression: non-fatal warn + no file).
#[tokio::test(flavor = "multi_thread")]
async fn creates_internal_dir_for_drift_json() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    let claims_file = root.join("claims.json");
    std::fs::write(
        &claims_file,
        r#"{"relationships":{"core_dependencies":[
            {"from":"src","to":"src","dependency_type":"Module","importance":1}
        ]}}"#,
    )
    .unwrap();
    let cfg_toml = root.join("agentwiki.toml");
    std::fs::write(&cfg_toml, "[scan]\ngit_tracked_only = false\n").unwrap();

    assert!(!root.join(".agentwiki").exists());
    let args = DriftArgs {
        project_path: Some(root.to_path_buf()),
        config: Some(cfg_toml),
        claims: Some(claims_file),
        ..Default::default()
    };
    assert_eq!(
        drift::run(args.project_path.clone(), args.config.clone(), &args, false).await,
        0
    );
    let report = root.join(".agentwiki/drift.json");
    assert!(report.is_file(), "drift.json must be created with its dir");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(report).unwrap()).unwrap();
    assert_eq!(v["schema_version"], 2);
}

/// fixture-rs exercises the Rust paths: brace `use` groups, `super::`,
/// inline paths, `#[cfg(test)]`, facade re-exports, crate-name imports
/// from integration tests, and doc-comment decoys.
#[test]
fn fixture_rs_extracts_and_resolves() {
    let claims = claims_json(serde_json::json!([
        // brace group: consumer -> engine + util
        {"from":"src/consumer.rs","to":"src/core/engine.rs","dependency_type":"Import","importance":4},
        {"from":"src/consumer.rs","to":"src/core/util.rs","dependency_type":"Import","importance":4},
        // super:: from engine; super::super:: from deep/inner
        {"from":"src/core/engine.rs","to":"src/core/util.rs","dependency_type":"Import","importance":4},
        {"from":"src/deep/inner.rs","to":"src/core/util.rs","dependency_type":"Import","importance":4},
        // inline path only: engine calls crate::deep::inner::probe()
        {"from":"src/core/engine.rs","to":"src/deep/inner.rs","dependency_type":"FunctionCall","importance":3},
        // facade re-export: facade/mod.rs pub use reexport_target::Helper
        {"from":"src/facade/mod.rs","to":"src/facade/reexport_target.rs","dependency_type":"Import","importance":4},
        // weak facade edge: consumer -> Helper reaches reexport_target
        {"from":"src/consumer.rs","to":"src/facade/reexport_target.rs","dependency_type":"Import","importance":3},
        // integration test imports by crate name
        {"from":"tests/integration.rs","to":"src/facade/reexport_target.rs","dependency_type":"Import","importance":2},
        // containment via `mod` decls
        {"from":"src","to":"src/core","dependency_type":"Module","importance":4},
        {"from":"src/deep","to":"src/deep/inner.rs","dependency_type":"Module","importance":3},
        // doc decoy: util.rs docs mention the facade — must stay phantom
        {"from":"src/core/util.rs","to":"src/facade/reexport_target.rs","dependency_type":"Import","importance":2},
        // code has engine -> util; claimed backwards
        {"from":"src/core/util.rs","to":"src/core/engine.rs","dependency_type":"Import","importance":3},
    ]));
    let out = analyze_fixture("fixture-rs", claims, |_| {});
    let find = |f: &str, t: &str| class_of(&out, f, t);
    let reason = |f: &str, t: &str| {
        out.findings
            .iter()
            .find(|x| x.from == f && x.to == t)
            .map(|x| x.reason.as_str())
    };

    for (f, t) in [
        ("src/consumer.rs", "src/core/engine.rs"),
        ("src/consumer.rs", "src/core/util.rs"),
        ("src/core/engine.rs", "src/core/util.rs"),
        ("src/deep/inner.rs", "src/core/util.rs"),
        ("src/core/engine.rs", "src/deep/inner.rs"),
        ("src/facade/mod.rs", "src/facade/reexport_target.rs"),
        ("src/consumer.rs", "src/facade/reexport_target.rs"),
        ("tests/integration.rs", "src/facade/reexport_target.rs"),
    ] {
        assert_eq!(
            find(f, t),
            Some(FindingClass::Confirmed),
            "{f} -> {t} should be confirmed"
        );
    }
    // inline-path-only evidence is what confirms engine -> inner.
    assert_eq!(
        reason("src/core/engine.rs", "src/deep/inner.rs"),
        Some("direct_evidence")
    );
    // containment claims confirm via `mod` decls (g_all).
    assert_eq!(
        reason("src", "src/core"),
        Some("containment"),
        "src -> src/core"
    );
    assert_eq!(reason("src/deep", "src/deep/inner.rs"), Some("containment"));
    assert_eq!(
        find("src/core/util.rs", "src/facade/reexport_target.rs"),
        Some(FindingClass::Phantom),
        "doc-comment decoy must not create an edge"
    );
    assert_eq!(
        find("src/core/util.rs", "src/core/engine.rs"),
        Some(FindingClass::Reversed)
    );
}
