//! The `kumikit` binary.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kumikit_core::paths::{self, Platform};
use kumikit_core::serve::{ServeOptions, serve};
use kumikit_core::transport;

/// Kumikit core: a headless server for working with coding agents.
#[derive(Debug, Parser)]
#[command(name = "kumikit", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the core on the local socket (named pipe on Windows).
    Serve {
        /// Socket path (Linux, macOS) or pipe name (Windows). Default: per platform.
        #[arg(long)]
        socket: Option<String>,
        /// Data directory. Default: per platform.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    init_logging();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    match cli.command {
        Command::Serve { socket, data_dir } => runtime.block_on(run_serve(socket, data_dir)),
    }
}

async fn run_serve(socket: Option<String>, data_dir: Option<PathBuf>) -> Result<()> {
    let platform = Platform::current();
    let data_dir = match data_dir {
        Some(d) => d,
        None => paths::data_dir(platform, &paths::process_env)
            .context("cannot find a data directory; pass --data-dir")?,
    };
    let address = socket.unwrap_or_else(|| {
        paths::default_address(platform, &paths::process_env, transport::current_uid())
    });
    let options = ServeOptions { data_dir, address };
    serve(options, announce_ready, shutdown_signal())
        .await
        .context("kumikit serve")
}

/// Prints the one stdout line scripts wait for (`docs/spec/02-protocol.md`).
#[allow(clippy::print_stdout)]
fn announce_ready(address: &str) {
    use std::io::Write;
    println!("ready {address}");
    let _ = std::io::stdout().flush();
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for SIGTERM");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Logs to stderr. `KUMIKIT_LOG` takes targets like `info` or `kumikit_core=debug`;
/// the default is `warn`. Message bodies are never logged at `info` or above.
fn init_logging() {
    use tracing_subscriber::filter::Targets;
    use tracing_subscriber::prelude::*;
    let filter: Targets = paths::process_env("KUMIKIT_LOG")
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| Targets::new().with_default(tracing::Level::WARN));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();
}
