//! Smoke tests for `examples/` — the shipped walkthroughs must keep
//! producing the documented verdicts. No LLM calls: `drift` is static,
//! and each example carries its own `agentwiki.toml` pointing at the
//! committed `docs/agentwiki.claims.json`.

use std::path::PathBuf;

use agentwiki::drift::{self, DriftArgs};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
}

async fn run(name: &str, strict: bool) -> (i32, serde_json::Value) {
    let root = example(name);
    let args = DriftArgs {
        project_path: Some(root.clone()),
        strict,
        ..Default::default()
    };
    let code = drift::run(args.project_path.clone(), args.config.clone(), &args, false).await;
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(".agentwiki/drift.json")).unwrap())
            .unwrap();
    (code, report)
}

fn ids(report: &serde_json::Value) -> Vec<&str> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect()
}

/// taskman-py ships real `devin` claims: 8 confirmed, 1 real phantom
/// (the model hallucinated report.py→core/db.py), 2 unverifiable.
#[tokio::test(flavor = "multi_thread")]
async fn taskman_py_catches_the_hallucinated_edge() {
    let (code, r) = run("taskman-py", false).await;
    assert_eq!(code, 0);
    assert_eq!(r["counts"]["confirmed"], 8);
    assert_eq!(r["counts"]["phantom"], 1);
    assert_eq!(r["counts"]["unverifiable"], 2);
    assert_eq!(r["coverage"]["checked"], 9);
    assert_eq!(r["coverage"]["total"], 11);
    assert!(
        ids(&r).contains(&"phantom:src/services/report.py->src/core/db.py"),
        "the hallucinated report→db claim must stay phantom"
    );
    assert!(
        r["claims_source"]
            .as_str()
            .unwrap()
            .ends_with("agentwiki.claims.json"),
        "claims must come from the committed file"
    );

    // Strict mode must fail on the new phantom — that is the demo.
    let (code, _) = run("taskman-py", true).await;
    assert_eq!(code, 1, "--strict must gate on the phantom");
}

/// notes-api-ts is the clean run: every checkable claim confirms.
#[tokio::test(flavor = "multi_thread")]
async fn notes_api_ts_is_fully_confirmed() {
    let (code, r) = run("notes-api-ts", false).await;
    assert_eq!(code, 0);
    assert_eq!(r["counts"]["confirmed"], 14);
    assert_eq!(r["counts"]["unverifiable"], 1);
    assert_eq!(r["coverage"]["checked"], 14);
    assert_eq!(r["coverage"]["total"], 15);
    for f in ids(&r) {
        assert!(
            !f.starts_with("phantom:") && !f.starts_with("reversed:"),
            "green example must have no gating findings, got {f}"
        );
    }

    let (code, _) = run("notes-api-ts", true).await;
    assert_eq!(code, 0, "--strict must pass on a clean example");
}
