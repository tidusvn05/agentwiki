//! `agentwiki` binary — thin CLI wrapper over the library.

use clap::Parser;
use tracing_subscriber::EnvFilter;

use agentwiki::cli::Args;
use agentwiki::config::{CliOverrides, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.verbose);

    let overrides = CliOverrides::from(&args);
    let config = Config::load(&overrides, args.config.as_deref())?;

    if args.dry_run {
        let scan = agentwiki::scanner::scan(&config)?;
        print!("{}", agentwiki::dry_run_report(&config, &scan));
        return Ok(());
    }

    let pctx = agentwiki::PipelineCtx::new(config, None).await?;
    agentwiki::run(&pctx).await?;
    Ok(())
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
