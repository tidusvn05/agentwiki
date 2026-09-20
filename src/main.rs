//! `agentwiki` binary — thin CLI wrapper over the library.

use clap::Parser;
use tracing_subscriber::EnvFilter;

use agentwiki::cli::{Args, Command};
use agentwiki::config::{CliOverrides, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.verbose);

    // `doctor`/`drift` are read-only: they must not take the run lock or
    // install the cancellation handler. `drift` does scan (it needs the
    // file list) but never writes pipeline state.
    match args.command {
        Some(Command::Doctor(d)) => {
            let code = agentwiki::doctor::run(
                d.project_path.or(args.project_path),
                d.config.or(args.config),
                d.fix,
            )
            .await;
            std::process::exit(code);
        }
        Some(Command::Drift(mut d)) => {
            d.project_path = d.project_path.or(args.project_path);
            d.config = d.config.or(args.config);
            d.output_path = d.output_path.or(args.output_path);
            let code = agentwiki::drift::run(
                d.project_path.clone(),
                d.config.clone(),
                &d,
                args.verbose > 0,
            )
            .await;
            std::process::exit(code);
        }
        None => {}
    }

    let overrides = CliOverrides::from(&args);
    let config = Config::load(&overrides, args.config.as_deref())?;

    if args.dry_run {
        let scan = agentwiki::scanner::scan(&config)?;
        print!("{}", agentwiki::dry_run_report(&config, &scan));
        return Ok(());
    }

    let pctx = agentwiki::PipelineCtx::new(config, None).await?;

    // First SIGINT/SIGTERM → cooperative cancel (in-flight child CLIs are
    // killed as their futures drop). A second signal force-exits.
    let cancel = pctx.cancel.clone();
    tokio::spawn(async move {
        termination_signal().await;
        if !cancel.is_cancelled() {
            cancel.cancel();
            termination_signal().await;
        }
        std::process::exit(130);
    });

    let result = agentwiki::run(&pctx).await;
    if pctx.cancel.is_cancelled() {
        // Conventional SIGINT exit code; the bar already shows "cancelled".
        std::process::exit(130);
    }
    result.map_err(Into::into)
}

/// Wait for SIGINT, or SIGTERM on unix.
async fn termination_signal() {
    let int = tokio::signal::ctrl_c();
    #[cfg(unix)]
    if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = int => {}
            _ = term.recv() => {}
        }
        return;
    }
    let _ = int.await;
}

/// `-v` info, `-vv` debug, `-vvv` trace; `RUST_LOG` overrides.
fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("agentwiki={level}")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
