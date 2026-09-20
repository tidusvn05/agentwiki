//! `agentwiki drift` — compare generated relationship claims against a
//! statically-extracted import graph. Read-only: no LLM, no pipeline
//! writes, no run lock.
//!
//! Exit codes: `0` = ok or warnings only, `1` = `--strict` found new
//! phantom/reversed findings, `2` = claims missing/invalid.

pub mod baseline;
pub mod claims;
pub mod compare;
pub mod config;
pub mod findings;
pub mod graph;
pub mod imports;
pub mod report;
pub mod resolve;
pub mod undocumented;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::agent::reports::CoreDependency;
use crate::config::{CliOverrides, Config};
use crate::scanner::{self, ScanData};

pub use config::DriftConfig;
use findings::Finding;
use report::{BaselineInfo, DriftReport};

/// CLI flags for `agentwiki drift`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct DriftArgs {
    /// Repository path to check (defaults to top-level `-p` or cwd).
    #[arg(short = 'p', long)]
    pub project_path: Option<PathBuf>,

    /// Path to agentwiki.toml (defaults to top-level `-c`).
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,

    /// Claims file override (default `<internal>/research.json`).
    #[arg(long, value_name = "PATH")]
    pub claims: Option<PathBuf>,

    /// Baseline file override (default: `[drift].baseline_path`, else
    /// `<project>/.agentwiki-drift-baseline.json`).
    #[arg(long, value_name = "PATH")]
    pub baseline: Option<PathBuf>,

    /// Max transitive hops for `confirmed` evidence (default:
    /// `[drift].max_transitive_depth` = 3).
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// Fail (exit 1) on phantom/reversed findings not in the baseline.
    #[arg(long)]
    pub strict: bool,

    /// Write current findings to the baseline file, then exit 0.
    #[arg(long)]
    pub update_baseline: bool,

    /// Export `relationships` from research.json to a commit-able file.
    #[arg(long, value_name = "PATH")]
    pub export_claims: Option<PathBuf>,

    /// Print the machine-readable report to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Run the drift check. Returns the process exit code.
pub async fn run(
    project_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    args: &DriftArgs,
    verbose: bool,
) -> i32 {
    let ov = CliOverrides {
        project_path: project_path.clone(),
        ..Default::default()
    };
    let cfg = match Config::load(&ov, config_path.as_deref()) {
        Ok(c) => c,
        Err(_) => {
            // Fall back to defaults like `doctor` — a broken config
            // shouldn't block a read-only check.
            let mut c = Config::default();
            if let Some(p) = &project_path {
                c.internal_path = p.join(&c.internal_path);
                c.project_path = p.clone();
            }
            c
        }
    };
    let root = match cfg.project_path.canonicalize() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot read project path: {e}");
            return 2;
        }
    };
    let mut dcfg = cfg.drift.clone();
    if let Some(d) = args.max_depth {
        dcfg.max_transitive_depth = d.max(1);
    }

    // --export-claims: standalone action, needs no claims file.
    if let Some(dest) = &args.export_claims {
        let src = cfg.internal_path.join("research.json");
        let dest = if dest.is_absolute() {
            dest.clone()
        } else {
            root.join(dest)
        };
        return match claims::export_claims(&src, &dest) {
            Ok(n) => {
                eprintln!("exported {n} claim edges to {}", dest.display());
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        };
    }

    let claims_path = match &args.claims {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => root.join(p),
        None => dcfg.claims_path(&root, &cfg.internal_path),
    };
    let claims = match claims::load_claims(&claims_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("hint: run `agentwiki --skip-documentation` first, or");
            eprintln!("      `agentwiki drift --export-claims claims.json` and commit it.");
            return 2;
        }
    };

    let test_globs: Vec<glob::Pattern> = dcfg
        .test_globs
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    let scan = match scanner::scan(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: scan failed: {e}");
            return 2;
        }
    };

    let out = analyze(&dcfg, &root, &scan, &claims, &test_globs);
    let mut report = build_report(&claims_path, &claims, out);

    let baseline_path = match &args.baseline {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => root.join(p),
        None => dcfg.baseline_path(&root),
    };
    match baseline::Baseline::load(&baseline_path) {
        Ok(Some(b)) => {
            let mut known = 0;
            for f in &mut report.findings {
                if b.findings.contains(&f.id) {
                    f.in_baseline = true;
                    known += 1;
                }
            }
            let cur: BTreeSet<&str> = report.findings.iter().map(|f| f.id.as_str()).collect();
            report.baseline = Some(BaselineInfo {
                path: baseline_path.display().to_string(),
                known,
                new: report.findings.len() - known,
                stale: b
                    .findings
                    .iter()
                    .filter(|id| !cur.contains(id.as_str()))
                    .count(),
            });
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("ignoring unreadable baseline: {e}"),
    }

    if args.update_baseline {
        let ids: BTreeSet<String> = report
            .findings
            .iter()
            .filter(|f| f.class.is_gating())
            .map(|f| f.id.clone())
            .collect();
        match baseline::Baseline::from_ids(ids).save(&baseline_path) {
            Ok(()) => eprintln!("baseline written to {}", baseline_path.display()),
            Err(e) => tracing::warn!("failed to write baseline: {e}"),
        }
        // After an update, every finding is known — strict never fires.
        for f in &mut report.findings {
            f.in_baseline = true;
        }
    }

    // Machine-readable report: `<internal>/drift.json` (non-fatal).
    // `.agentwiki/` may not exist yet — create it like the other writers.
    if let Err(e) = std::fs::create_dir_all(&cfg.internal_path) {
        tracing::warn!("cannot create {}: {e}", cfg.internal_path.display());
    }
    let out_json = cfg.internal_path.join("drift.json");
    match serde_json::to_string_pretty(&report) {
        Ok(body) => {
            if let Err(e) = crate::util::write_atomic(&out_json, body.as_bytes()) {
                tracing::warn!("failed to write {}: {e}", out_json.display());
            }
        }
        Err(e) => tracing::warn!("failed to serialize drift report: {e}"),
    }

    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => tracing::warn!("failed to serialize drift report: {e}"),
        }
    } else {
        print!("{}", report::render(&report, verbose));
    }

    let n_fail = report.strict_failures().len();
    if args.strict && n_fail > 0 { 1 } else { 0 }
}

/// Intermediate result of [`analyze`].
pub struct Outcome {
    /// All findings (claim-side + undocumented), sorted by id.
    pub findings: Vec<Finding>,
    /// U-filter drop counts.
    pub filtered: BTreeMap<String, usize>,
    /// Detected hub nodes.
    pub hubs: BTreeSet<String>,
}

/// The pure core: claims + scan → findings. Separate from `run()` so
/// tests can drive it without process-level IO.
pub fn analyze(
    cfg: &DriftConfig,
    root: &Path,
    scan: &ScanData,
    claims: &[CoreDependency],
    test_globs: &[glob::Pattern],
) -> Outcome {
    let files: BTreeSet<PathBuf> = scan.files.iter().map(|f| f.rel_path.clone()).collect();
    let dirs: BTreeSet<PathBuf> = scan
        .directories
        .iter()
        .map(|d| d.rel_path.clone())
        .collect();
    let claimed = compare::normalize_claims(claims, &files, &dirs, root);

    let endpoints: Vec<claims::Endpoint> = claimed
        .iter()
        .flat_map(|c| [c.from_ep.clone(), c.to_ep.clone()])
        .collect();
    let code_files: Vec<(PathBuf, imports::Lang)> = scan
        .files
        .iter()
        .map(|f| {
            (
                f.rel_path.clone(),
                f.extension
                    .as_deref()
                    .map(imports::Lang::from_extension)
                    .unwrap_or(imports::Lang::Other),
            )
        })
        .collect();
    let nodes = graph::build_nodes(&endpoints, &code_files);

    let fg = resolve::build_file_graph(cfg, root, scan, test_globs);
    // `mod x;` edges are containment evidence only (g_all); G_full for
    // reachability/direct/reversed excludes them.
    let g_all = graph::lift(&fg.edges, &nodes, true);
    let g_full = graph::lift(&fg.edges, &nodes, false);
    let hubs = g_full.hubs(
        nodes.kinds.len(),
        cfg.hub_in_degree_ratio,
        cfg.hub_min_nodes,
    );

    let ctx = compare::CompareCtx {
        cfg,
        nodes: &nodes,
        files: &fg,
        g_full: &g_full,
        g_all: &g_all,
        hubs: &hubs,
    };
    let mut findings: Vec<Finding> = claimed
        .iter()
        .map(|c| compare::classify_claim(&ctx, c))
        .collect();

    let mut filtered = BTreeMap::new();
    findings.extend(undocumented::find(
        cfg,
        &nodes,
        &g_full,
        &hubs,
        &claimed,
        &mut filtered,
    ));
    findings.sort_by(|a, b| a.id.cmp(&b.id));

    Outcome {
        findings,
        filtered,
        hubs,
    }
}

/// Assemble the serializable report from an analysis outcome.
fn build_report(claims_path: &Path, claims: &[CoreDependency], out: Outcome) -> DriftReport {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in &out.findings {
        *counts.entry(f.class.as_str().to_string()).or_default() += 1;
    }
    DriftReport {
        schema_version: report::REPORT_VERSION,
        claims_source: claims_path.display().to_string(),
        claims_total: claims.len(),
        counts,
        hubs: out.hubs.iter().cloned().collect(),
        filtered: out.filtered,
        findings: out.findings,
        baseline: None,
    }
}
