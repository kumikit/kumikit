//! The `kumikit` binary.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kumikit_core::paths::{self, Platform};
use kumikit_core::serve::{ServeOptions, serve};
use kumikit_core::{cli, transport};

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
        /// Preinstall the .kkp files in this directory once: each id is seeded only the
        /// first time; plugins that request no permissions are also enabled.
        #[arg(long)]
        bundled_plugins: Option<PathBuf>,
    },
    /// Manage core plugins (local .kkp files).
    Plugin {
        /// The core's socket (Linux, macOS) or pipe (Windows). Default: per platform.
        #[arg(long, global = true)]
        socket: Option<String>,
        /// The core's data directory, used when no core is running. Default: per platform.
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: PluginAction,
    },
}

#[derive(Debug, Subcommand)]
enum PluginAction {
    /// Install a .kkp file, show its permissions and, if you agree, enable it.
    Add {
        /// The package.
        file: PathBuf,
        /// Grant the permissions without asking.
        #[arg(long)]
        yes: bool,
    },
    /// List installed plugins.
    List,
    /// Remove a plugin.
    Remove {
        /// Plugin id.
        id: String,
        /// Also delete its data without asking.
        #[arg(long, conflicts_with = "keep_data")]
        yes: bool,
        /// Keep its data without asking.
        #[arg(long)]
        keep_data: bool,
    },
    /// Build a .kkp package from a plugin directory.
    Pack {
        /// Directory with plugin.toml and the files it names.
        dir: PathBuf,
        /// Output file. Default: <id>-<version>.kkp.
        #[arg(short, long)]
        output: Option<PathBuf>,
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
        Command::Serve {
            socket,
            data_dir,
            bundled_plugins,
        } => runtime.block_on(run_serve(socket, data_dir, bundled_plugins)),
        Command::Plugin {
            socket,
            data_dir,
            action,
        } => runtime.block_on(run_plugin(socket, data_dir, action)),
    }
}

fn target(socket: Option<String>, data_dir: Option<PathBuf>) -> Result<cli::Target> {
    let platform = Platform::current();
    let data_dir = match data_dir {
        Some(d) => d,
        None => paths::data_dir(platform, &paths::process_env)
            .context("cannot find a data directory; pass --data-dir")?,
    };
    let address = socket.unwrap_or_else(|| {
        paths::default_address(platform, &paths::process_env, transport::current_uid())
    });
    Ok(cli::Target { data_dir, address })
}

async fn run_plugin(
    socket: Option<String>,
    data_dir: Option<PathBuf>,
    action: PluginAction,
) -> Result<()> {
    let out = &mut cli::Stdout;
    match action {
        PluginAction::Pack { dir, output } => cli::pack(&dir, output.as_deref(), out),
        PluginAction::Add { file, yes } => {
            cli::add(&target(socket, data_dir)?, &file, yes, out).await
        }
        PluginAction::List => cli::list(&target(socket, data_dir)?, out).await,
        PluginAction::Remove { id, yes, keep_data } => {
            cli::remove(&target(socket, data_dir)?, &id, yes, keep_data, out).await
        }
    }
}

async fn run_serve(
    socket: Option<String>,
    data_dir: Option<PathBuf>,
    bundled_plugins: Option<PathBuf>,
) -> Result<()> {
    let platform = Platform::current();
    let data_dir = match data_dir {
        Some(d) => d,
        None => paths::data_dir(platform, &paths::process_env)
            .context("cannot find a data directory; pass --data-dir")?,
    };
    let address = socket.unwrap_or_else(|| {
        paths::default_address(platform, &paths::process_env, transport::current_uid())
    });
    let options = ServeOptions {
        data_dir,
        address,
        bundled_plugins,
    };
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

/// Registers for SIGTERM and SIGINT (Ctrl+C on Windows) now, before `serve` prints
/// `ready`, and returns a future that completes on the first. Registering lazily, when
/// the future is first polled, left a window after `ready` in which a SIGTERM killed the
/// core without a clean shutdown.
fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let term = signal(SignalKind::terminate());
        let int = signal(SignalKind::interrupt());
        async move {
            match (term, int) {
                (Ok(mut term), Ok(mut int)) => {
                    tokio::select! {
                        _ = term.recv() => {}
                        _ = int.recv() => {}
                    }
                }
                (term, int) => {
                    for e in [term.err(), int.err()].into_iter().flatten() {
                        tracing::warn!(error = %e, "cannot listen for a shutdown signal");
                    }
                    std::future::pending::<()>().await;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let ctrl_c = tokio::signal::windows::ctrl_c();
        async move {
            match ctrl_c {
                Ok(mut c) => {
                    c.recv().await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "cannot listen for Ctrl+C");
                    std::future::pending::<()>().await;
                }
            }
        }
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
