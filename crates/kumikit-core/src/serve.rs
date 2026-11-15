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
}

/// Database file name inside the data directory.
pub const DATABASE: &str = "core.sqlite";

/// Runs the core. Calls `on_ready` once connections are accepted, and returns after
/// `shutdown` resolves and the kernel has closed.
pub async fn serve(
    options: ServeOptions,
    on_ready: impl FnOnce(&str),
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

    let host = match open_core(&options.data_dir).await {
        Ok(h) => h,
        Err(e) => {
            listener.remove();
            return Err(e);
        }
    };
    let kernel = host.manager().kernel().clone();
    let info = Arc::new(ServerInfo::current());
    on_ready(&options.address);

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
        }
    }
    listener.remove();
    kernel.close().await;
    Ok(())
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

fn prepare_data_dir(dir: &std::path::Path) -> io::Result<()> {
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
