//! Installing, enabling, disabling and removing plugins
//! (`docs/spec/04-plugin-host.md#lifecycle`, `docs/spec/10-distribution.md`).
//!
//! Files live in the core data directory:
//! - `plugins/<id>/<version>/`: the unpacked package (read-only use);
//! - `plugin-data/<id>/`: the plugin's private data directory;
//! - `cache/wasmtime/`: compiled components.
//!
//! The kernel's `plugins` table is the record of what is installed and granted. Only
//! enabled plugins' manifests are read at startup; nothing is instantiated.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kumikit_kernel::grants::Grants;
use kumikit_kernel::plugins::{PluginRecord, PluginSource, PluginState};
use kumikit_kernel::{Kernel, KernelError};
use kumikit_protocol::ErrorCode;
use tokio::sync::watch;

use crate::manifest::{MANIFEST_FILE, Manifest, ManifestError};
use crate::package::{self, PackageError};
use crate::runtime::{Prepared, Runtime};

/// A plugin-management failure.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The package or its manifest is invalid.
    #[error("{0}")]
    Invalid(String),
    /// The request contradicts current state.
    #[error("{0}")]
    Conflict(String),
    /// No such plugin.
    #[error("{0}")]
    NotFound(String),
    /// A plugin failed.
    #[error("{0}")]
    PluginFailed(String),
    /// A kernel call failed.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// File I/O failed.
    #[error("{0}")]
    Io(String),
    /// A package failed verification: its signature, a pinned key, or (from a registry)
    /// its hash or its entry.
    #[error("{0}")]
    Untrusted(String),
}

impl HostError {
    /// The protocol error code.
    pub fn code(&self) -> ErrorCode {
        match self {
            HostError::Invalid(_) => ErrorCode::InvalidParams,
            HostError::Conflict(_) => ErrorCode::Conflict,
            HostError::NotFound(_) => ErrorCode::NotFound,
            HostError::PluginFailed(_) => ErrorCode::PluginFailed,
            HostError::Kernel(e) => e.code(),
            HostError::Io(_) => ErrorCode::Internal,
            HostError::Untrusted(_) => ErrorCode::Untrusted,
        }
    }
}

impl From<ManifestError> for HostError {
    fn from(e: ManifestError) -> Self {
        HostError::Invalid(e.to_string())
    }
}

impl From<PackageError> for HostError {
    fn from(e: PackageError) -> Self {
        match e {
            PackageError::Invalid(m) => HostError::Invalid(format!("invalid package: {m}")),
            PackageError::Io(e) => HostError::Io(e.to_string()),
            PackageError::Untrusted(m) => HostError::Untrusted(format!("untrusted package: {m}")),
        }
    }
}

fn io(e: impl std::fmt::Display) -> HostError {
    HostError::Io(e.to_string())
}

/// Where the plugin host keeps its files.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// Unpacked packages.
    pub plugins: PathBuf,
    /// Plugins' private data directories.
    pub data: PathBuf,
    /// Compiled components.
    pub cache: PathBuf,
}

impl Dirs {
    /// The layout under a core data directory.
    pub fn new(data_dir: &Path) -> Dirs {
        Dirs {
            plugins: data_dir.join("plugins"),
            data: data_dir.join("plugin-data"),
            cache: data_dir.join("cache").join("wasmtime"),
        }
    }

    /// An installed version's directory.
    pub fn package(&self, id: &str, version: &str) -> PathBuf {
        self.plugins.join(id).join(version)
    }

    /// A plugin's private data directory.
    pub fn plugin_data(&self, id: &str) -> PathBuf {
        self.data.join(id)
    }
}

/// An installed plugin, as clients see it.
#[derive(Debug, Clone)]
pub struct PluginView {
    /// The kernel's record.
    pub record: PluginRecord,
    /// The manifest, if it could be read.
    pub manifest: Option<Arc<Manifest>>,
    /// Why the manifest could not be read.
    pub error: Option<String>,
}

impl PluginView {
    /// The permissions the plugin asks for.
    pub fn requested(&self) -> Grants {
        self.manifest
            .as_ref()
            .map(|m| m.permissions.to_grants())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
struct Entry {
    record: PluginRecord,
    manifest: Result<Arc<Manifest>, String>,
    /// The compiled, linked component, once prepared in this process.
    prepared: Option<Prepared>,
}

impl Entry {
    fn view(&self) -> PluginView {
        PluginView {
            record: self.record.clone(),
            manifest: self.manifest.as_ref().ok().cloned(),
            error: self.manifest.as_ref().err().cloned(),
        }
    }
}

/// Installs and tracks plugins. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    kernel: Kernel,
    dirs: Dirs,
    runtime: Arc<Runtime>,
    entries: Mutex<BTreeMap<String, Entry>>,
    /// One management operation at a time.
    ops: tokio::sync::Mutex<()>,
    /// Bumped whenever the set of enabled plugins changes.
    changed: watch::Sender<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The map stays consistent if a holder panicked.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, HostError> + Send + 'static,
) -> Result<T, HostError> {
    tokio::task::spawn_blocking(f).await.map_err(io)?
}

impl Manager {
    /// Reads the installed plugins. Manifests are parsed (small TOML files); nothing is
    /// compiled or instantiated, and the runtime is not started.
    pub async fn open(
        kernel: Kernel,
        dirs: Dirs,
        runtime: Arc<Runtime>,
    ) -> Result<Manager, HostError> {
        let records = kernel.plugin_list().await?;
        let mut entries = BTreeMap::new();
        for record in records {
            let dir = dirs.package(&record.id, &record.version);
            let manifest = blocking(move || Ok(Manifest::load_installed(&dir)))
                .await?
                .map(Arc::new)
                .map_err(|e| e.to_string());
            if let Err(e) = &manifest {
                tracing::warn!(plugin = %record.id, error = %e, "installed plugin cannot be loaded");
            }
            entries.insert(
                record.id.clone(),
                Entry {
                    record,
                    manifest,
                    prepared: None,
                },
            );
        }
        Ok(Manager {
            inner: Arc::new(Inner {
                kernel,
                dirs,
                runtime,
                entries: Mutex::new(entries),
                ops: tokio::sync::Mutex::new(()),
                changed: watch::Sender::new(0),
            }),
        })
    }

    /// The directories in use.
    pub fn dirs(&self) -> &Dirs {
        &self.inner.dirs
    }

    /// The kernel.
    pub fn kernel(&self) -> &Kernel {
        &self.inner.kernel
    }

    /// The WASM runtime.
    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.inner.runtime
    }

    /// The plugin's compiled, linked component; compiled (or loaded from the compile
    /// cache) on first use in this process. Fails for a plugin without a component.
    pub async fn prepared(&self, id: &str) -> Result<Prepared, HostError> {
        let view = self.get(id)?;
        if let Some(p) = lock(&self.inner.entries)
            .get(id)
            .and_then(|e| e.prepared.clone())
        {
            return Ok(p);
        }
        let manifest = view
            .manifest
            .ok_or_else(|| HostError::PluginFailed(format!("plugin {id} cannot be loaded")))?;
        let (Some(world), Some(wasm)) = (manifest.world(), manifest.wasm()) else {
            return Err(HostError::Invalid(format!("plugin {id} has no component")));
        };
        let path = self
            .inner
            .dirs
            .package(&view.record.id, &view.record.version)
            .join(wasm);
        let prepared = self
            .inner
            .runtime
            .prepare(&path, world)
            .await
            .map_err(|e| HostError::Invalid(format!("plugin {id}: {e}")))?;
        if let Some(entry) = lock(&self.inner.entries).get_mut(id) {
            entry.prepared = Some(prepared.clone());
        }
        Ok(prepared)
    }

    /// Every installed plugin, by id.
    pub fn list(&self) -> Vec<PluginView> {
        lock(&self.inner.entries)
            .values()
            .map(Entry::view)
            .collect()
    }

    /// One installed plugin.
    pub fn get(&self, id: &str) -> Result<PluginView, HostError> {
        lock(&self.inner.entries)
            .get(id)
            .map(Entry::view)
            .ok_or_else(|| HostError::NotFound(format!("plugin {id} is not installed")))
    }

    /// Enabled plugins whose manifests loaded.
    pub fn enabled(&self) -> Vec<(PluginRecord, Arc<Manifest>)> {
        lock(&self.inner.entries)
            .values()
            .filter(|e| e.record.state == PluginState::Enabled)
            .filter_map(|e| Some((e.record.clone(), e.manifest.as_ref().ok()?.clone())))
            .collect()
    }

    /// A receiver that sees a new value whenever the enabled plugins change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    fn bump(&self) {
        self.inner.changed.send_modify(|v| *v += 1);
    }

    /// Installs a `.kkp` file: unpacks, verifies its signature if it has one, validates,
    /// records it as `installed` with its source and signing key. Nothing is granted or
    /// enabled.
    pub async fn install(
        &self,
        package: &Path,
        source: PluginSource,
    ) -> Result<PluginView, HostError> {
        let _op = self.inner.ops.lock().await;
        let package = package.to_path_buf();
        let plugins_dir = self.inner.dirs.plugins.clone();
        let (sha256, staging, manifest, verified) = blocking(move || {
            let sha256 = package::sha256_file(&package).map_err(|e| {
                HostError::Invalid(format!("cannot read {}: {e}", package.display()))
            })?;
            std::fs::create_dir_all(&plugins_dir).map_err(io)?;
            let staging = tempfile::Builder::new()
                .prefix(".incoming-")
                .tempdir_in(&plugins_dir)
                .map_err(io)?;
            package::unpack(&package, staging.path())?;
            let verified = package::signature::verify_dir(staging.path())?;
            if !staging.path().join(MANIFEST_FILE).is_file() {
                return Err(HostError::Invalid(format!(
                    "invalid package: {MANIFEST_FILE} is missing"
                )));
            }
            let manifest = Manifest::load(staging.path())?;
            Ok((sha256, staging, manifest, verified))
        })
        .await?;

        if lock(&self.inner.entries).contains_key(&manifest.id) {
            return Err(HostError::Conflict(format!(
                "plugin {} is already installed; remove it first",
                manifest.id
            )));
        }
        let target = self.inner.dirs.package(&manifest.id, &manifest.version);
        blocking(move || {
            if target.exists() {
                // Left over from an install that did not finish.
                std::fs::remove_dir_all(&target).map_err(io)?;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            let staged = staging.keep();
            std::fs::rename(&staged, &target).map_err(io)
        })
        .await?;

        let record = PluginRecord {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            state: PluginState::Installed,
            sha256,
            granted: None,
            installed: self.inner.kernel.now(),
            publisher_key: verified.key().map(|k| k.to_string()),
            source,
        };
        self.inner.kernel.plugin_insert(record.clone()).await?;
        tracing::info!(plugin = %record.id, version = %record.version, "plugin installed");
        let entry = Entry {
            record,
            manifest: Ok(Arc::new(manifest)),
            prepared: None,
        };
        let view = entry.view();
        lock(&self.inner.entries).insert(view.record.id.clone(), entry);
        Ok(view)
    }

    /// Grants the plugin's permissions and enables it. `grant` must equal what the
    /// plugin requests: permissions are granted as a whole.
    pub async fn enable(&self, id: &str, grant: &Grants) -> Result<PluginView, HostError> {
        let _op = self.inner.ops.lock().await;
        let view = self.get(id)?;
        let Some(manifest) = &view.manifest else {
            return Err(HostError::Conflict(format!(
                "plugin {id} cannot be enabled: {}",
                view.error.unwrap_or_default()
            )));
        };
        let requested = manifest.permissions.to_grants();
        if &requested != grant {
            return Err(HostError::Conflict(format!(
                "the grant for {id} does not match the permissions it requests; they are granted as a whole"
            )));
        }
        if view.record.state == PluginState::Enabled {
            return Ok(view);
        }
        // Compile now, so a broken or unsupported component fails here, not on first use.
        if manifest.wasm().is_some() {
            self.prepared(id).await?;
        }
        self.inner
            .kernel
            .plugin_set_state(id, PluginState::Enabled, Some(requested.clone()))
            .await?;
        let view = self.set(id, PluginState::Enabled, Some(requested))?;
        tracing::info!(plugin = %id, "plugin enabled");
        self.bump();
        Ok(view)
    }

    /// Disables a plugin. Its grants are kept, so enabling it again needs the same grant.
    pub async fn disable(&self, id: &str) -> Result<PluginView, HostError> {
        let _op = self.inner.ops.lock().await;
        let view = self.get(id)?;
        if view.record.state != PluginState::Enabled {
            return Ok(view);
        }
        let granted = view.record.granted.clone();
        self.inner
            .kernel
            .plugin_set_state(id, PluginState::Disabled, granted.clone())
            .await?;
        let view = self.set(id, PluginState::Disabled, granted)?;
        tracing::info!(plugin = %id, "plugin disabled");
        self.bump();
        Ok(view)
    }

    /// Removes a plugin's files and record; with `delete_data`, also its kv store and
    /// data directory.
    pub async fn uninstall(&self, id: &str, delete_data: bool) -> Result<(), HostError> {
        let _op = self.inner.ops.lock().await;
        let view = self.get(id)?;
        let was_enabled = view.record.state == PluginState::Enabled;
        self.inner.kernel.plugin_delete(id, delete_data).await?;
        lock(&self.inner.entries).remove(id);
        let dirs = self.inner.dirs.clone();
        let id_owned = id.to_owned();
        blocking(move || {
            let files = dirs.plugins.join(&id_owned);
            if files.exists() {
                std::fs::remove_dir_all(&files).map_err(io)?;
            }
            let data = dirs.plugin_data(&id_owned);
            if delete_data && data.exists() {
                std::fs::remove_dir_all(&data).map_err(io)?;
            }
            Ok(())
        })
        .await?;
        tracing::info!(plugin = %id, delete_data, "plugin removed");
        if was_enabled {
            self.bump();
        }
        Ok(())
    }

    fn set(
        &self,
        id: &str,
        state: PluginState,
        granted: Option<Grants>,
    ) -> Result<PluginView, HostError> {
        let mut entries = lock(&self.inner.entries);
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| HostError::NotFound(format!("plugin {id} is not installed")))?;
        entry.record.state = state;
        entry.record.granted = granted;
        Ok(entry.view())
    }
}

#[cfg(test)]
mod tests;
