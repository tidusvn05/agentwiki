//! Pipeline orchestration: Preprocess → Research → Compose → Write → Verify.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::agent::registry;
use crate::agent::spec::Phase;
use crate::agent::{ExecKind, ResearchContext, run_spec};
use crate::backend::{AgentBackend, BackendKind};
use crate::cache::Cache;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::prompt::PromptLoader;
use crate::quota::Quota;
use crate::scanner::{self, ScanData};

/// Everything a spec instance needs, shared via `Arc`.
pub struct PipelineCtx {
    /// Resolved config.
    pub config: Config,
    /// Phase-0 scan output.
    pub scan: ScanData,
    /// Research/compose result store.
    pub ctx: ResearchContext,
    /// Content-hash cache.
    pub cache: Cache,
    /// Daily cap + audit log.
    pub quota: Quota,
    /// Prompt templates.
    pub prompts: PromptLoader,
    /// Bounds concurrent CLI calls.
    pub semaphore: Semaphore,
    /// Run statistics for the summary report.
    pub stats: Mutex<RunStats>,
    /// Sanitized cwd for embedded-mode calls.
    pub empty_cwd: PathBuf,
    /// Terminal progress display (hidden on non-TTY).
    pub progress: crate::progress::Progress,
    /// Cooperative shutdown — cancelled by the signal handler in `main`.
    pub cancel: CancellationToken,
    /// Constructed backends by kind.
    backends: HashMap<BackendKind, Arc<dyn AgentBackend>>,
}

/// Bookkeeping for the summary report.
#[derive(Debug, Default)]
pub struct RunStats {
    /// Cache hits.
    pub cache_hits: u32,
    /// Real CLI calls made.
    pub cli_calls: u32,
    /// Seconds saved by cache hits.
    pub saved_secs: f64,
    /// Per-spec wall times.
    pub timings: Vec<(String, f64)>,
}

impl RunStats {
    /// A cache hit saved `secs` of backend time.
    pub fn cache_hit(&mut self, secs: f64) {
        self.cache_hits += 1;
        self.saved_secs += secs;
    }

    /// A real CLI call happened.
    pub fn cli_call(&mut self) {
        self.cli_calls += 1;
    }

    /// Per-spec timing.
    pub fn record(&mut self, spec: &str, dur: Duration) {
        self.timings.push((spec.to_string(), dur.as_secs_f64()));
    }
}

impl PipelineCtx {
    /// Build the context: scan → cache/quota/prompts → backends.
    ///
    /// `backends` may be injected (tests); `None` constructs real CLI
    /// backends for the kinds referenced by the configured model strings.
    pub async fn new(
        config: Config,
        backends: Option<HashMap<BackendKind, Arc<dyn AgentBackend>>>,
    ) -> Result<Arc<Self>> {
        let scan = scanner::scan(&config)?;

        let internal = config.internal_path.clone();
        std::fs::create_dir_all(&internal).map_err(|e| Error::io(&internal, e))?;
        let empty_cwd = internal.join("empty-cwd");
        std::fs::create_dir_all(&empty_cwd).map_err(|e| Error::io(&empty_cwd, e))?;

        let backends = match backends {
            Some(b) => b,
            None => default_backends(&config)?,
        };

        Ok(Arc::new(Self {
            cache: Cache::new(&internal, config.no_cache, config.force_regenerate),
            quota: Quota::new(&internal, config.limits.daily_cap),
            prompts: PromptLoader::new(config.prompts_dir.clone()),
            semaphore: Semaphore::new(config.max_parallels),
            stats: Mutex::new(RunStats::default()),
            progress: crate::progress::Progress::new(),
            cancel: CancellationToken::new(),
            ctx: ResearchContext::new(),
            empty_cwd,
            backends,
            scan,
            config,
        }))
    }

    /// Look up a constructed backend.
    pub fn backend(&self, kind: BackendKind) -> Result<Arc<dyn AgentBackend>> {
        self.backends
            .get(&kind)
            .cloned()
            .ok_or_else(|| Error::BackendNotAvailable(kind.as_str().to_string()))
    }
}

/// Build real CLI backends for the kinds named in `models.*`.
fn default_backends(config: &Config) -> Result<HashMap<BackendKind, Arc<dyn AgentBackend>>> {
    let mut map = HashMap::new();
    for m in [&config.models.efficient, &config.models.powerful] {
        let (kind, _) = BackendKind::parse(m)?;
        map.entry(kind)
            .or_insert_with(|| crate::backend::for_kind(kind));
    }
    Ok(map)
}

/// Guard holding `<internal>/run.lock`; removes the file on drop.
struct RunLock {
    path: PathBuf,
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `create_new` lock against double-runs. A stale lock (dead pid, or an
/// unreadable/truncated file) is reclaimed; a live one fails fast.
fn acquire_run_lock(internal: &std::path::Path) -> Result<RunLock> {
    let path = internal.join("run.lock");
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write as _;
                let _ = writeln!(f, "{}", std::process::id());
                return Ok(RunLock { path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(pid) = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    && crate::sys::pid_alive(pid)
                {
                    return Err(Error::AlreadyRunning { pid });
                }
                // Stale or unreadable lock — reclaim it.
                let _ = std::fs::remove_file(&path);
            }
            Err(e) => return Err(Error::io(&path, e)),
        }
    }
    Err(Error::Pipeline(format!(
        "could not acquire run lock {}",
        path.display()
    )))
}

/// Run the full pipeline, settling the progress bar on the way out.
pub async fn run(pctx: &Arc<PipelineCtx>) -> Result<()> {
    let _lock = acquire_run_lock(&pctx.config.internal_path)?;
    let result = run_pipeline(pctx).await;
    match &result {
        Ok(()) => pctx.progress.done(),
        Err(Error::Cancelled) => pctx.progress.cancel(),
        Err(e) => pctx.progress.fail(e),
    }
    result
}

/// The pipeline proper: research → compose → write → verify.
async fn run_pipeline(pctx: &Arc<PipelineCtx>) -> Result<()> {
    let started = Instant::now();

    if pctx.config.skip_research {
        let path = pctx.config.internal_path.join("research.json");
        pctx.load_research(&path).await?;
    } else {
        research(pctx).await?;
        // Persist research so `--skip-research` can reuse it later.
        let path = pctx.config.internal_path.join("research.json");
        pctx.ctx.save(&path).await?;
    }

    if pctx.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    if !pctx.config.skip_documentation {
        compose(pctx).await?;
        crate::output::write_docs(pctx).await?;
    }

    if pctx.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    if !pctx.config.skip_documentation {
        let report = crate::output::verify(pctx).await?;
        crate::output::write_summary(pctx, &report, started.elapsed()).await?;
        // Commit-able claims copy next to the docs — CI's `drift` finds
        // it without anyone remembering `--export-claims`. Non-fatal,
        // same spirit as `verify`.
        let dest = pctx
            .config
            .output_path
            .join(crate::drift::claims::CLAIMS_FILENAME);
        let research = pctx.config.internal_path.join("research.json");
        if let Err(e) = crate::drift::claims::export_claims(&research, &dest) {
            tracing::warn!("failed to write {}: {e}", dest.display());
        }
    }

    let stats = pctx.stats.lock().await;
    tracing::info!(
        cli_calls = stats.cli_calls,
        cache_hits = stats.cache_hits,
        secs = started.elapsed().as_secs_f64(),
        "pipeline complete"
    );
    Ok(())
}

/// Phase 1 — run research specs level by level (deps before dependents).
async fn research(pctx: &Arc<PipelineCtx>) -> Result<()> {
    run_level_order(&registry::research_specs(), pctx).await
}

/// Phase 2 — compose specs (LLM editors + deterministic renderers).
async fn compose(pctx: &Arc<PipelineCtx>) -> Result<()> {
    run_level_order(&registry::compose_specs(), pctx).await
}

/// Execute specs level by level; specs within a level run in parallel.
/// Cancellation aborts in-flight tasks (dropping them kills their child
/// CLIs) and refuses to start the next level.
async fn run_level_order(specs: &[crate::agent::AgentSpec], pctx: &Arc<PipelineCtx>) -> Result<()> {
    for (i, level) in registry::topo_levels(specs).iter().enumerate() {
        if pctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        tracing::info!(level = i, specs = level.len(), "DAG level");
        let mut set = tokio::task::JoinSet::new();
        for &idx in level {
            let (spec, pctx) = (specs[idx].clone(), Arc::clone(pctx));
            set.spawn(async move { run_spec(&spec, &pctx).await });
        }
        loop {
            tokio::select! {
                biased;
                r = set.join_next() => {
                    match r {
                        Some(r) => r.map_err(|e| Error::Pipeline(format!("join: {e}")))??,
                        None => break,
                    }
                }
                () = pctx.cancel.cancelled() => {
                    set.abort_all();
                    // Aborted tasks drop their backend futures, which kills
                    // the child CLIs (kill_on_drop). Bound the drain so a
                    // task stuck in a sync poll can't hang shutdown.
                    let drain = async { while set.join_next().await.is_some() {} };
                    let _ = tokio::time::timeout(Duration::from_secs(3), drain).await;
                    return Err(Error::Cancelled);
                }
            }
        }
    }
    Ok(())
}

impl PipelineCtx {
    /// `--skip-research`: hydrate ctx from a saved research.json.
    async fn load_research(&self, path: &std::path::Path) -> Result<()> {
        let loaded = ResearchContext::load(path).await?;
        for k in loaded.keys().await {
            if let Some(v) = loaded.get(&k).await {
                self.ctx.insert(&k, v).await;
            }
        }
        tracing::info!(path = %path.display(), "loaded saved research context");
        Ok(())
    }
}

/// Print the resolved config + DAG for `--dry-run`.
pub fn dry_run_report(config: &Config, scan: &ScanData) -> String {
    use std::fmt::Write as _;
    let mut s = format!(
        "Effective config:\n  project: {}\n  output: {}\n  internal: {}\n  profile: {}\n  language: {:?}\n  mode: {:?}\n  models: efficient={} powerful={}\n  max_parallels: {}\n  daily_cap: {}\n\nScan: {} files in {} directories\n\nTask DAG:\n",
        config.project_path.display(),
        config.output_path.display(),
        config.internal_path.display(),
        config.profile.as_deref().unwrap_or("-"),
        config.target_language,
        config.mode,
        config.models.efficient,
        config.models.powerful,
        config.max_parallels,
        config.limits.daily_cap,
        scan.files.len(),
        scan.directories.len(),
    );
    for phase in [Phase::Research, Phase::Compose] {
        let specs = match phase {
            Phase::Research => registry::research_specs(),
            Phase::Compose => registry::compose_specs(),
        };
        let _ = writeln!(s, "\n  [{:?}]", phase);
        for level in registry::topo_levels(&specs) {
            for idx in level {
                let sp = &specs[idx];
                let fan = match sp.fan_out {
                    Some(crate::agent::FanOut::PerDir) => {
                        format!(" ×{}", scan.directories.len())
                    }
                    Some(crate::agent::FanOut::PerDomain) => " ×N(domains)".to_string(),
                    None => String::new(),
                };
                let kind = match sp.exec {
                    ExecKind::Deterministic(_) => "deterministic",
                    ExecKind::Llm => "llm",
                };
                let _ = writeln!(
                    s,
                    "    {} [{}|{}]{} <- [{}]",
                    sp.name,
                    kind,
                    match sp.tier {
                        crate::config::ModelTier::Efficient => "efficient",
                        crate::config::ModelTier::Powerful => "powerful",
                    },
                    fan,
                    sp.deps.join(", ")
                );
            }
        }
    }
    s
}
