//! `kumikit serve`: open the kernel, listen on the local transport, serve connections
//! until asked to stop.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kumikit_kernel::{Kernel, KernelConfig};
use kumikit_plugin_host::engine::EngineConfig;
use kumikit_plugin_host::lifecycle::{HostConfig, PluginHost};
use kumikit_plugin_host::manager::{Dirs, Manager};
use kumikit_plugin_host::runtime::Runtime;

use crate::lock::DataDirLock;
use crate::rpc::{self, ServerInfo};
use crate::transport::{self, Listener};

/// Where the core keeps its data and listens.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Data directory (created if missing; 0700 on Unix).
    pub data_dir: PathBuf,
    /// Socket path (Unix) or pipe name (Windows).
    pub address: String,
    /// A directory of bundled `.kkp` files to preinstall once
    /// (`docs/spec/10-distribution.md#bundled-core-plugins`).
    pub bundled_plugins: Option<PathBuf>,
    /// Remote access (`--listen`); off when `None`.
    pub remote: Option<RemoteOptions>,
}

/// Remote access settings for one run (`docs/spec/02-protocol.md#remote-transport`).
#[derive(Debug, Clone)]
pub struct RemoteOptions {
    /// `--listen`: `host[:port]`.
    pub listen: String,
    /// `--tls-cert` and `--tls-key`; a generated certificate when `None`.
    pub certificate: Option<(PathBuf, PathBuf)>,
    /// Time limits.
    pub limits: crate::remote::Limits,
    /// `--web`: the browser client's files, served on the remote port.
    pub web: Option<PathBuf>,
}

/// What `serve` tells its caller once it accepts connections.
#[derive(Debug, Clone)]
pub struct Ready {
    /// The local socket or pipe.
    pub address: String,
    /// With remote access: the URL and the certificate fingerprint.
    pub remote: Option<(String, String)>,
}

/// Database file name inside the data directory.
pub const DATABASE: &str = "core.sqlite";

/// Runs the core. Calls `on_ready` once connections are accepted, and returns after
/// `shutdown` resolves (or a client calls `core.shutdown`) and the kernel has closed.
pub async fn serve(
    options: ServeOptions,
    on_ready: impl FnOnce(&Ready),
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    prepare_data_dir(&options.data_dir)?;
    // A second core on this data directory fails here, before it binds or touches the
    // database. Held until `serve` returns.
    let _lock = DataDirLock::acquire(&options.data_dir)?;
    // Bind next: a second core on this address fails here.
    #[cfg(unix)]
    let listener = Listener::bind(std::path::Path::new(&options.address)).await?;
    #[cfg(windows)]
    let mut listener = Listener::bind(&options.address).await?;

    // Then the remote port and certificate, so a busy port or a bad certificate fails
    // before the core opens anything else.
    let remote = match prepare_remote(&options).await {
        Ok(r) => r,
        Err(e) => {
            listener.remove();
            return Err(e);
        }
    };
    let host = match open_core(&options.data_dir).await {
        Ok(h) => h,
        Err(e) => {
            listener.remove();
            return Err(e);
        }
    };
    let kernel = host.manager().kernel().clone();
    let mut info = ServerInfo::current();
    info.registry = Some(Arc::new(crate::registry::CoreRegistry::new(
        &options.data_dir,
        &crate::paths::process_env,
    )));
    let remote_task = remote.map(|(tcp, tls, state)| {
        let state = Arc::new(state);
        info.remote = Some(Arc::clone(&state));
        (tcp, tls, state)
    });
    let info = Arc::new(info);
    let remote_task = remote_task.map(|(tcp, tls, state)| {
        tokio::spawn(crate::remote::run(
            tcp,
            tls,
            host.clone(),
            Arc::clone(&info),
            state,
        ))
    });
    on_ready(&Ready {
        address: options.address.clone(),
        remote: info.remote.as_ref().map(|r| {
            (
                format!("wss://{}{}", r.bound, kumikit_protocol::remote::RPC_PATH),
                r.fingerprint.clone(),
            )
        }),
    });
    // Seeding compiles components, so it runs after the core accepts connections;
    // clients learn about the new plugins through `core.pluginsChanged`.
    let seeding = options
        .bundled_plugins
        .clone()
        .map(|dir| tokio::spawn(crate::bundled::seed(host.clone(), dir)));

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    tokio::spawn(rpc::serve_connection(host.clone(), Arc::clone(&info), stream));
                }
                Err(e) => tracing::warn!(error = %e, "accept failed"),
            },
            () = &mut shutdown => break,
            () = info.shutdown.notified() => break,
        }
    }
    if let Some(task) = seeding {
        task.abort();
    }
    if let Some(task) = remote_task {
        task.abort();
    }
    listener.remove();
    kernel.close().await;
    Ok(())
}

/// Binds the remote port and loads the certificate, if remote access is on.
async fn prepare_remote(
    options: &ServeOptions,
) -> io::Result<
    Option<(
        tokio::net::TcpListener,
        Arc<rustls::ServerConfig>,
        crate::remote::Remote,
    )>,
> {
    let Some(r) = &options.remote else {
        return Ok(None);
    };
    let tcp = crate::remote::bind(&r.listen).await?;
    let bound = tcp.local_addr()?;
    let source = match &r.certificate {
        Some((cert, key)) => crate::remote::tls::CertSource::Given {
            cert: cert.clone(),
            key: key.clone(),
        },
        None => crate::remote::tls::CertSource::Generated(options.data_dir.clone()),
    };
    let host_name = crate::paths::host_name();
    let tls = crate::remote::tls::load(&source, &host_name)?;
    let mut state = crate::remote::Remote::new(bound, tls.fingerprint, &host_name, r.limits);
    if let Some(web) = &r.web {
        if !web.join("index.html").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "--web {}: no index.html there (build it with node scripts/bundle.mjs)",
                    web.display()
                ),
            ));
        }
        state.web = Some(web.clone());
    }
    Ok(Some((tcp, tls.config, state)))
}

/// Opens the kernel, the installed plugins and the plugin host on a data directory.
/// Registers the enabled plugins' providers; instantiates nothing (unless a plugin asks
/// for `startup` activation). The caller must hold the data directory's lock.
pub async fn open_core(data_dir: &Path) -> io::Result<PluginHost> {
    let kernel = Kernel::open(KernelConfig::new(data_dir.join(DATABASE)))
        .await
        .map_err(io::Error::other)?;
    let dirs = Dirs::new(data_dir);
    let runtime = Arc::new(Runtime::new(EngineConfig::default(), dirs.cache.clone()));
    let manager = Manager::open(kernel, dirs, runtime)
        .await
        .map_err(io::Error::other)?;
    Ok(PluginHost::open(manager, HostConfig::default()).await)
}

/// Creates the data directory if needed (0700 on Unix; an existing one must be private).
pub fn prepare_data_dir(dir: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        transport::ensure_private_dir(dir, transport::current_uid())
    }
    #[cfg(not(unix))]
    {
        let _ = transport::current_uid;
        std::fs::create_dir_all(dir)
    }
}
