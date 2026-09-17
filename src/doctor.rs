//! `agentwiki doctor` — read-only environment health check:
//! agent CLIs on PATH, running/orphaned agent processes, `.agentwiki/`
//! state (lock, quota, cache, research), config sanity, and project shape.
//!
//! Exit code: `1` when any check fails, `0` on ok/warnings only.
//! `--fix` additionally removes stale run locks and leftover temp files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::backend::BackendKind;
use crate::config::{CliOverrides, Config};
use crate::sys;

/// Severity of one check line; ordered so `max` gives the overall result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Status {
    /// Neutral information.
    #[default]
    Info,
    /// Check passed.
    Ok,
    /// Suspicious but not blocking.
    Warn,
    /// Blocking problem.
    Fail,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Ok => "  ok  ",
            Status::Info => " info ",
            Status::Warn => " warn ",
            Status::Fail => " FAIL ",
        }
    }
}

/// Accumulates check lines and the worst status.
#[derive(Default)]
struct Report {
    lines: Vec<(Status, String)>,
    worst: Status,
    fixes: Vec<String>,
}

impl Report {
    fn add(&mut self, status: Status, msg: impl Into<String>) {
        self.worst = self.worst.max(status);
        self.lines.push((status, msg.into()));
    }

    fn fixed(&mut self, msg: impl Into<String>) {
        self.fixes.push(msg.into());
    }
}

/// One named section of check lines.
fn print_section(name: &str, r: &Report) {
    println!("{name}:");
    for (s, m) in &r.lines {
        println!("  {} {m}", s.tag());
    }
    println!();
}

/// Run every check. Returns the process exit code.
pub async fn run(project_path: Option<PathBuf>, config_path: Option<PathBuf>, fix: bool) -> i32 {
    println!(
        "agentwiki {} — environment check\n",
        env!("CARGO_PKG_VERSION")
    );

    // Config — keep going with defaults if it fails so other checks still run.
    let ov = CliOverrides {
        project_path: project_path.clone(),
        ..Default::default()
    };
    let cfg_result = Config::load(&ov, config_path.as_deref());
    let cfg = match &cfg_result {
        Ok(c) => c.clone(),
        Err(_) => {
            let mut c = Config::default();
            if let Some(p) = project_path {
                c.internal_path = p.join(&c.internal_path);
                c.project_path = p;
            }
            c
        }
    };

    let mut config = Report::default();
    match &cfg_result {
        Ok(_) => {
            let mut srcs = Vec::new();
            if let Some(g) = Config::global_config_file() {
                srcs.push(g.display().to_string());
            }
            if let Some(p) = &config_path {
                srcs.push(p.display().to_string());
            } else {
                let proj = cfg.project_path.join("agentwiki.toml");
                if proj.is_file() {
                    srcs.push(proj.display().to_string());
                }
            }
            config.add(
                Status::Ok,
                format!(
                    "config resolved ({}); profile={}, lang={:?}, mode={:?}",
                    if srcs.is_empty() {
                        "defaults".to_string()
                    } else {
                        srcs.join(" + ")
                    },
                    cfg.profile.as_deref().unwrap_or("-"),
                    cfg.target_language,
                    cfg.mode,
                ),
            );
        }
        Err(e) => config.add(Status::Fail, format!("config load failed: {e}")),
    }
    // Model strings must parse into known backends.
    let mut required: HashSet<BackendKind> = HashSet::new();
    for (tier, m) in [
        ("efficient", &cfg.models.efficient),
        ("powerful", &cfg.models.powerful),
    ] {
        match BackendKind::parse(m) {
            Ok((k, _)) => {
                if k != BackendKind::Mock {
                    required.insert(k);
                }
            }
            Err(e) => config.add(Status::Fail, format!("models.{tier}='{m}': {e}")),
        }
    }
    config.add(
        Status::Info,
        format!(
            "models: efficient={} powerful={}",
            cfg.models.efficient, cfg.models.powerful
        ),
    );
    print_section("config", &config);

    // Agent CLIs on PATH (+ version probe).
    let mut clis = Report::default();
    for kind in [BackendKind::Devin, BackendKind::Claude, BackendKind::Codex] {
        let name = kind.as_str();
        match sys::find_on_path(name) {
            Some(path) => match probe_version(&path).await {
                Some(v) => clis.add(Status::Ok, format!("{name} {v} ({})", path.display())),
                None => clis.add(
                    Status::Warn,
                    format!("{name} found at {} but `--version` failed", path.display()),
                ),
            },
            None if required.contains(&kind) => clis.add(
                Status::Fail,
                format!("{name} not on PATH — required by configured models"),
            ),
            None => clis.add(
                Status::Info,
                format!("{name} not on PATH (not required by configured models)"),
            ),
        }
    }
    if cfg.verify.mermaid_fixer {
        match sys::find_on_path("mermaid-fixer") {
            Some(p) => clis.add(Status::Ok, format!("mermaid-fixer ({})", p.display())),
            None => clis.add(
                Status::Info,
                "mermaid-fixer not on PATH — mermaid lint will be skipped".to_string(),
            ),
        }
    }
    print_section("agent CLIs", &clis);

    // Running agent processes — the main question doctor answers.
    let mut procs = Report::default();
    let me = std::process::id();
    let all = sys::list_processes();
    let agents: Vec<(String, &sys::ProcInfo)> = all
        .iter()
        .filter(|p| p.pid != me)
        .filter_map(|p| sys::agent_name(p).map(|n| (n.to_string(), p)))
        .collect();
    if agents.is_empty() {
        procs.add(Status::Ok, "no agent processes running".to_string());
    } else {
        for (name, p) in &agents {
            let elapsed = p
                .etime
                .as_deref()
                .map(|e| format!(", elapsed {e}"))
                .unwrap_or_default();
            if name == "agentwiki" {
                procs.add(
                    Status::Warn,
                    format!(
                        "agentwiki pid {} alive{elapsed} — a concurrent run is active",
                        p.pid
                    ),
                );
            } else {
                procs.add(
                    Status::Warn,
                    format!(
                        "{name} pid {} alive{elapsed} — orphan from a killed run? `kill {}` if unexpected",
                        p.pid, p.pid
                    ),
                );
            }
        }
    }
    print_section("processes", &procs);

    // `.agentwiki/` state.
    let mut state = Report::default();
    let mut run_active = false;
    let internal = &cfg.internal_path;
    if !internal.is_dir() {
        state.add(
            Status::Info,
            format!(
                "{} does not exist yet (created on first run)",
                internal.display()
            ),
        );
    } else {
        // run.lock — live pid means a run is in progress.
        let lock = internal.join("run.lock");
        if lock.is_file() {
            let pid = std::fs::read_to_string(&lock)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());
            match pid {
                Some(pid) if sys::pid_alive(pid) => {
                    run_active = true;
                    state.add(
                        Status::Info,
                        format!("run.lock held by live pid {pid} — a run is in progress"),
                    );
                }
                _ => {
                    let what = pid
                        .map(|p| format!("dead pid {p}"))
                        .unwrap_or_else(|| "unreadable".to_string());
                    if fix && std::fs::remove_file(&lock).is_ok() {
                        state.fixed(format!("removed stale {}", lock.display()));
                    }
                    state.add(
                        Status::Warn,
                        format!(
                            "stale run.lock ({what}) — auto-reclaimed on next run{}",
                            if fix { "" } else { "; --fix removes it now" }
                        ),
                    );
                }
            }
        } else {
            state.add(Status::Ok, "run.lock: none".to_string());
        }

        quota_state(internal, cfg.limits.daily_cap, &mut state);
        cache_state(internal, &mut state);
        research_state(internal, &mut state);
        calls_state(internal, &mut state);
    }
    temp_state(fix, run_active, &mut state);
    print_section(&format!("state ({})", internal.display()), &state);

    // Project shape.
    let mut project = Report::default();
    let pp = &cfg.project_path;
    if !pp.is_dir() {
        project.add(
            Status::Fail,
            format!("project path {} is not a directory", pp.display()),
        );
    } else {
        let git = pp.join(".git").exists();
        if cfg.scan.git_tracked_only {
            if !git {
                project.add(
                    Status::Warn,
                    "not a git repo — git_tracked_only falls back to a full-tree scan".to_string(),
                );
            } else if sys::find_on_path("git").is_none() {
                project.add(
                    Status::Warn,
                    "`git` not on PATH — git_tracked_only falls back to a full-tree scan"
                        .to_string(),
                );
            } else {
                project.add(Status::Ok, format!("{} (git repo)", pp.display()));
            }
        } else {
            project.add(Status::Ok, format!("{} (git repo: {git})", pp.display()));
        }
        match writable_probe(pp) {
            true => {}
            false => project.add(Status::Fail, format!("{} is not writable", pp.display())),
        }
    }
    let out = &cfg.output_path;
    if out.exists() {
        project.add(Status::Ok, format!("output {} exists", out.display()));
    } else {
        project.add(
            Status::Info,
            format!("output {} will be created on run", out.display()),
        );
    }
    print_section("project", &project);

    // Fixes applied by --fix.
    let fixes: Vec<String> = [
        config.fixes,
        clis.fixes,
        procs.fixes,
        state.fixes,
        project.fixes,
    ]
    .concat();
    for f in &fixes {
        println!("  fixed {f}");
    }
    if !fixes.is_empty() {
        println!();
    }

    let errors = [
        config.worst,
        clis.worst,
        procs.worst,
        state.worst,
        project.worst,
    ]
    .iter()
    .filter(|s| **s == Status::Fail)
    .count();
    let warns = [
        config.worst,
        clis.worst,
        procs.worst,
        state.worst,
        project.worst,
    ]
    .iter()
    .filter(|s| **s == Status::Warn)
    .count();
    println!("result: {errors} section(s) failing, {warns} with warnings");
    i32::from(errors > 0)
}

/// `<cli> --version` with a short timeout; first non-empty output line.
async fn probe_version(path: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    };
    String::from_utf8_lossy(text)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// `.agentwiki/state.json` → calls used today.
fn quota_state(internal: &Path, cap: u32, r: &mut Report) {
    let p = internal.join("state.json");
    let Ok(text) = std::fs::read_to_string(&p) else {
        r.add(Status::Info, format!("quota: 0/{cap} calls used today"));
        return;
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            r.add(
                Status::Warn,
                format!("{} is corrupt (will reset on next run)", p.display()),
            );
            return;
        }
    };
    let count = v["count"].as_u64().unwrap_or(0);
    let date = v["date"].as_str().unwrap_or("?");
    if date == crate::quota::today() {
        r.add(Status::Ok, format!("quota: {count}/{cap} calls used today"));
    } else {
        r.add(
            Status::Info,
            format!("quota: 0/{cap} calls today (last activity {date}, {count} calls)"),
        );
    }
}

/// `.agentwiki/cache/` → entry count + bytes.
fn cache_state(internal: &Path, r: &mut Report) {
    let dir = internal.join("cache");
    if !dir.is_dir() {
        r.add(Status::Info, "cache: empty".to_string());
        return;
    }
    let mut n = 0u64;
    let mut bytes = 0u64;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "json") {
            n += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    r.add(
        Status::Ok,
        format!("cache: {n} entries, {:.1} MB", bytes as f64 / 1e6),
    );
}

/// `.agentwiki/research.json` presence — enables `--skip-research`.
fn research_state(internal: &Path, r: &mut Report) {
    let p = internal.join("research.json");
    if p.is_file() {
        r.add(
            Status::Ok,
            "research.json present — --skip-research is usable".to_string(),
        );
    } else {
        r.add(
            Status::Info,
            "no research.json yet — --skip-research unavailable".to_string(),
        );
    }
}

/// `.agentwiki/calls.jsonl` → record count + last record.
fn calls_state(internal: &Path, r: &mut Report) {
    let p = internal.join("calls.jsonl");
    let Ok(text) = std::fs::read_to_string(&p) else {
        r.add(Status::Info, "calls.jsonl: no calls logged yet".to_string());
        return;
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let last = lines
        .last()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok());
    match last {
        Some(v) => r.add(
            Status::Ok,
            format!(
                "calls.jsonl: {} records; last: {} {} ({})",
                lines.len(),
                v["agent"].as_str().unwrap_or("?"),
                v["status"].as_str().unwrap_or("?"),
                v["ts"].as_str().unwrap_or("?"),
            ),
        ),
        None => r.add(Status::Ok, format!("calls.jsonl: {} records", lines.len())),
    }
}

/// `agentwiki-*` prompt/output tempfiles left by interrupted runs.
/// Never cleaned while a run is active — they may belong to it.
fn temp_state(fix: bool, run_active: bool, r: &mut Report) {
    let dir = std::env::temp_dir();
    let mut stale: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            // Only files: leftover tempfiles are NamedTempFile artifacts;
            // an `agentwiki-*` directory is a user output dir, not debris.
            if e.path().is_file() && e.file_name().to_string_lossy().starts_with("agentwiki-") {
                stale.push(e.path());
            }
        }
    }
    if stale.is_empty() {
        r.add(Status::Ok, "no leftover agentwiki tempfiles".to_string());
        return;
    }
    let n = stale.len();
    if fix && run_active {
        r.add(
            Status::Warn,
            format!(
                "{n} agentwiki-* tempfile(s) in {} — skipped cleanup: a run is active and may own them",
                dir.display()
            ),
        );
        return;
    }
    if fix {
        let mut removed = 0;
        for p in stale {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
        }
        r.fixed(format!(
            "removed {removed}/{n} agentwiki-* tempfiles in {}",
            dir.display()
        ));
    }
    r.add(
        Status::Warn,
        format!(
            "{n} leftover agentwiki-* tempfile(s) in {} from interrupted runs{}",
            dir.display(),
            if fix { "" } else { " — --fix removes them" },
        ),
    );
}

/// Can we create a file inside `dir` (or its nearest existing ancestor)?
fn writable_probe(dir: &Path) -> bool {
    let mut d = dir;
    loop {
        if d.is_dir() {
            return tempfile::NamedTempFile::new_in(d).is_ok();
        }
        match d.parent() {
            Some(p) => d = p,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_probe_on_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable_probe(dir.path()));
        // Non-existent deep path still resolves via ancestors.
        assert!(writable_probe(&dir.path().join("a/b/c")));
        assert!(!writable_probe(Path::new("/definitely/not/here/xyz")));
    }

    #[tokio::test]
    async fn doctor_smoke_on_empty_project() {
        // An empty temp project: no CLIs required (models unparsed? default
        // requires devin → would FAIL on machines without it). Point models at
        // mock so the check is hermetic.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("agentwiki.toml"),
            "[models]\nefficient = \"mock:x\"\npowerful = \"mock:x\"\n",
        )
        .unwrap();
        let code = run(Some(dir.path().to_path_buf()), None, false).await;
        assert_eq!(code, 0);
    }
}
