//! The `kumikit` binary.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kumikit_core::paths::{self, Platform};
use kumikit_core::serve::{Ready, RemoteOptions, ServeOptions, serve};
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
        /// Turn on remote access for this run: listen for paired clients on
        /// host[:port] (default port 7463), for example 0.0.0.0 for every interface.
        #[arg(long, value_name = "HOST[:PORT]")]
        listen: Option<String>,
        /// PEM certificate to use instead of the generated one (needs --tls-key).
        #[arg(long, requires_all = ["tls_key", "listen"])]
        tls_cert: Option<PathBuf>,
        /// PEM private key of --tls-cert.
        #[arg(long, requires_all = ["tls_cert", "listen"])]
        tls_key: Option<PathBuf>,
        /// Serve the browser client from this directory on the remote port (needs --listen).
        #[arg(long, requires = "listen", value_name = "DIR")]
        web: Option<PathBuf>,
        /// Test only: milliseconds to authenticate a remote connection.
        #[arg(long, hide = true)]
        pre_auth_ms: Option<u64>,
        /// Test only: milliseconds between keepalive pings.
        #[arg(long, hide = true)]
        keepalive_ping_ms: Option<u64>,
        /// Test only: milliseconds of silence before a remote client is dropped.
        #[arg(long, hide = true)]
        keepalive_idle_ms: Option<u64>,
    },
    /// Show a one-time code to pair a device with this core (needs a running core
    /// started with --listen).
    Pair {
        /// The core's socket (Linux, macOS) or pipe (Windows). Default: per platform.
        #[arg(long)]
        socket: Option<String>,
        /// Print the code, expiry, fingerprint and address as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check a theme before publishing it.
    Theme {
        #[command(subcommand)]
        action: ThemeAction,
    },
    /// List or revoke paired devices.
    Devices {
        /// The core's socket (Linux, macOS) or pipe (Windows). Default: per platform.
        #[arg(long, global = true)]
        socket: Option<String>,
        /// The core's data directory, used when no core is running. Default: per platform.
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: DevicesAction,
    },
    /// Copy JSON-RPC lines between stdin/stdout and the core on this machine (what the
    /// desktop app runs over SSH).
    Bridge {
        /// The core's socket (Linux, macOS) or pipe (Windows). Default: per platform.
        #[arg(long)]
        socket: Option<String>,
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
enum DevicesAction {
    /// List paired devices.
    List,
    /// Unpair a device: its token stops working and its connections close.
    Revoke {
        /// Device id, from `kumikit devices list`.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ThemeAction {
    /// Token values, WCAG AA contrast and fonts, as the app checks them at install.
    Check {
        /// A theme directory (with plugin.toml) or a .kkp.
        theme: PathBuf,
        /// Treat contrast warnings as errors.
        #[arg(long)]
        strict: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RegistryAction {
    /// The registries, in the order ids are looked up.
    List,
    /// Add a registry by its index URL.
    Add {
        /// `https://…/index.json` (http:// only on this computer, or file:).
        url: String,
    },
    /// Remove a registry.
    Remove {
        /// Its index URL.
        url: String,
    },
}

#[derive(Debug, Subcommand)]
enum LockAction {
    /// Print the lockfile, or write it to a file.
    Export {
        /// Write it here.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Install what a lockfile lists and enable what was enabled.
    Import {
        /// The lockfile.
        file: PathBuf,
        /// Grant the permissions without asking.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum PluginAction {
    /// Install a plugin by id from a registry, or a .kkp file; show its permissions and,
    /// if you agree, enable it.
    Add {
        /// A plugin id (`acme.probe`, `acme.probe@0.2.0`) or a .kkp file.
        target: String,
        /// Use this registry (an index URL) instead of the configured ones.
        #[arg(long, value_name = "URL")]
        registry: Option<String>,
        /// Grant the permissions without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Search the registries for core plugins.
    Search {
        /// Words to look for in ids, names and descriptions.
        words: Vec<String>,
    },
    /// List the updates the registries offer and apply the ones you agree to.
    Update {
        /// Only this plugin.
        id: Option<String>,
        /// Apply updates that ask for no new permissions without asking.
        #[arg(long)]
        yes: bool,
        /// Also apply updates that ask for more permissions, granting them.
        #[arg(long)]
        allow_wider: bool,
    },
    /// List, add or remove plugin registries.
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },
    /// Export or import the lockfile.
    Lock {
        #[command(subcommand)]
        action: LockAction,
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
    /// Build a .kkp package from a plugin directory, signed with --key.
    Pack {
        /// Directory with plugin.toml and the files it names.
        dir: PathBuf,
        /// Output file. Default: <id>-<version>.kkp.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Sign with this private key file (or set KUMIKIT_SIGNING_KEY and pass --sign).
        #[arg(long, value_name = "FILE")]
        key: Option<PathBuf>,
        /// Sign with the key in KUMIKIT_SIGNING_KEY.
        #[arg(long, conflicts_with = "key")]
        sign: bool,
    },
    /// Make a publisher key pair; prints the public key.
    Keygen {
        /// Name of the key file in the keys directory.
        #[arg(long, default_value = "publisher")]
        name: String,
        /// Write the private key here instead.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Allow writing inside a git work tree.
        #[arg(long)]
        force: bool,
    },
    /// Sign a .kkp (replacing any signature) with a private key.
    Sign {
        /// The package; it is rewritten in place.
        file: PathBuf,
        /// Private key file; without it, KUMIKIT_SIGNING_KEY.
        #[arg(long, value_name = "FILE")]
        key: Option<PathBuf>,
    },
    /// Check a .kkp's signature and say who signed it.
    Verify {
        /// The package.
        file: PathBuf,
        /// Fail unless it is signed by this key (ed25519:…).
        #[arg(long, value_name = "KEY ID")]
        key: Option<String>,
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
            listen,
            tls_cert,
            tls_key,
            web,
            pre_auth_ms,
            keepalive_ping_ms,
            keepalive_idle_ms,
        } => {
            let remote = listen.map(|listen| {
                let mut limits = kumikit_core::remote::Limits::default();
                let ms = std::time::Duration::from_millis;
                if let Some(v) = pre_auth_ms {
                    limits.pre_auth = ms(v);
                }
                if let Some(v) = keepalive_ping_ms {
                    limits.ping = ms(v);
                }
                if let Some(v) = keepalive_idle_ms {
                    limits.idle = ms(v);
                }
                RemoteOptions {
                    listen,
                    certificate: tls_cert.zip(tls_key),
                    limits,
                    web,
                }
            });
            runtime.block_on(run_serve(socket, data_dir, bundled_plugins, remote))
        }
        Command::Plugin {
            socket,
            data_dir,
            action,
        } => runtime.block_on(run_plugin(socket, data_dir, action)),
        Command::Theme {
            action:
                ThemeAction::Check {
                    theme,
                    strict,
                    json,
                },
        } => {
            let report = kumikit_core::theme::check(&theme)?;
            if !kumikit_core::theme::print(&report, strict, json, &mut cli::Stdout) {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Pair { socket, json } => runtime
            .block_on(async { cli::pair(&target(socket, None)?, json, &mut cli::Stdout).await }),
        Command::Devices {
            socket,
            data_dir,
            action,
        } => runtime.block_on(async {
            let t = target(socket, data_dir)?;
            match action {
                DevicesAction::List => cli::devices_list(&t, &mut cli::Stdout).await,
                DevicesAction::Revoke { id } => {
                    cli::devices_revoke(&t, &id, &mut cli::Stdout).await
                }
            }
        }),
        Command::Bridge { socket } => {
            runtime.block_on(async { cli::bridge(&target(socket, None)?).await })
        }
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
        PluginAction::Pack {
            dir,
            output,
            key,
            sign,
        } => {
            let key = if key.is_some() || sign {
                Some(kumikit_core::signing::read_key(
                    key.as_deref(),
                    &paths::process_env,
                )?)
            } else {
                None
            };
            cli::pack(&dir, output.as_deref(), key.as_ref(), out)
        }
        PluginAction::Keygen {
            name,
            out: file,
            force,
        } => kumikit_core::signing::keygen(
            &name,
            file.as_deref(),
            force,
            Platform::current(),
            &paths::process_env,
            out,
        ),
        PluginAction::Sign { file, key } => {
            let key = kumikit_core::signing::read_key(key.as_deref(), &paths::process_env)?;
            kumikit_core::signing::sign(&file, &key, out)
        }
        PluginAction::Verify { file, key } => {
            kumikit_core::signing::verify(&file, key.as_deref(), out)
        }
        PluginAction::Add {
            target: what,
            registry,
            yes,
        } => {
            let t = target(socket, data_dir)?;
            if registry.is_none() && cli::is_package_path(&what) {
                cli::add(&t, std::path::Path::new(&what), yes, out).await
            } else {
                cli::add_by_id(&t, &what, registry.as_deref(), yes, out).await
            }
        }
        PluginAction::Search { words } => {
            cli::search(&target(socket, data_dir)?, &words, out).await
        }
        PluginAction::Update {
            id,
            yes,
            allow_wider,
        } => {
            cli::update(
                &target(socket, data_dir)?,
                id.as_deref(),
                yes,
                allow_wider,
                out,
            )
            .await
        }
        PluginAction::Registry { action } => {
            let t = target(socket, data_dir)?;
            match action {
                RegistryAction::List => cli::registry_list(&t, out).await,
                RegistryAction::Add { url } => cli::registry_add(&t, &url, out).await,
                RegistryAction::Remove { url } => cli::registry_remove(&t, &url, out).await,
            }
        }
        PluginAction::Lock { action } => {
            let t = target(socket, data_dir)?;
            match action {
                LockAction::Export { output } => cli::lock_export(&t, output.as_deref(), out).await,
                LockAction::Import { file, yes } => cli::lock_import(&t, &file, yes, out).await,
            }
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
    remote: Option<RemoteOptions>,
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
        remote,
    };
    serve(options, announce_ready, shutdown_signal())
        .await
        .context("kumikit serve")
}

/// Prints the startup lines scripts wait for (`docs/spec/02-protocol.md`): `remote <url>
/// <fingerprint>` with remote access, then `ready <address>` last.
#[allow(clippy::print_stdout)]
fn announce_ready(ready: &Ready) {
    use std::io::Write;
    if let Some((url, fingerprint)) = &ready.remote {
        println!("remote {url} {fingerprint}");
    }
    println!("ready {}", ready.address);
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
        .with(
            tracing_subscriber::fmt::layer().with_writer(kumikit_core::logwriter::RedactingStderr),
        )
        .with(filter)
        .init();
}
