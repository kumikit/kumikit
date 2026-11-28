//! The `plugin.*` methods and the plugin parts of `core.hello`: the plugin host's views
//! in protocol types (`docs/spec/02-protocol.md`).

use std::path::Path;

use kumikit_kernel::grants::{FilesGrant, Grants};
use kumikit_kernel::plugins::{PluginSource, PluginState as KState};
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_plugin_host::manager::{HostError, PluginView};
use kumikit_plugin_host::manifest::{MANIFEST_FILE, Manifest};
use kumikit_plugin_host::package;
use kumikit_protocol::methods::{
    Contributions, DeclaredProvider, FilesAccess, InstalledPlugin, Limits, Permissions,
    PluginFailure, PluginInfo, PluginSource as ProtocolSource, PluginState, PublisherInfo,
};
use kumikit_protocol::{ErrorCode, RpcError};

/// A plugin-host error as a protocol error.
pub fn host_err(e: HostError) -> RpcError {
    if e.code() == ErrorCode::Internal {
        tracing::error!(error = %e, "plugin request failed");
    }
    RpcError::new(e.code(), e.to_string())
}

fn limits(m: &Manifest) -> Option<Limits> {
    let l = m.limits;
    (l.memory_mb.is_some() || l.call_timeout_s.is_some()).then_some(Limits {
        memory_mb: l.memory_mb,
        call_timeout_s: l.call_timeout_s,
    })
}

/// Grants in protocol form, with the manifest's raised limits.
pub fn permissions(g: &Grants, manifest: Option<&Manifest>) -> Permissions {
    kumikit_plugin_host::manager::protocol_permissions(g, manifest)
}

/// A protocol grant as kernel grants. Fails with `conflict` if its limits are not the
/// ones the plugin asks for: raised limits are part of what the user approves.
pub fn grants(p: &Permissions, manifest: Option<&Manifest>) -> Result<Grants, RpcError> {
    if p.limits != manifest.and_then(limits) {
        return Err(RpcError::new(
            ErrorCode::Conflict,
            "the grant's limits do not match the limits the plugin asks for",
        ));
    }
    Ok(grants_of(p))
}

/// Protocol permissions as kernel grants, limits aside.
pub fn grants_of(p: &Permissions) -> Grants {
    Grants {
        spawn: p.spawn.iter().cloned().collect(),
        network: p.network.iter().cloned().collect(),
        files: match p.files {
            FilesAccess::None => FilesGrant::None,
            FilesAccess::ReadWorkspace => FilesGrant::ReadWorkspace,
            FilesAccess::Workspace => FilesGrant::Workspace,
        },
        secrets: p.secrets.iter().cloned().collect(),
        tools: p.tools,
        client: p.client.iter().cloned().collect(),
        workspaces: match p.workspaces {
            kumikit_protocol::methods::WorkspacesAccess::None => {
                kumikit_kernel::grants::WorkspacesGrant::None
            }
            kumikit_protocol::methods::WorkspacesAccess::Linked => {
                kumikit_kernel::grants::WorkspacesGrant::Linked
            }
            kumikit_protocol::methods::WorkspacesAccess::All => {
                kumikit_kernel::grants::WorkspacesGrant::All
            }
        },
        schedules: p.schedules,
    }
}

fn contributions(host: &PluginHost, id: &str, manifest: Option<&Manifest>) -> Contributions {
    let providers = host
        .enabled()
        .into_iter()
        .find(|p| p.record.id == id)
        .map(|p| p.providers)
        .unwrap_or_default();
    Contributions {
        providers,
        commands: manifest
            .map(|m| {
                m.contributes
                    .commands
                    .iter()
                    .map(|c| c.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
        settings: manifest.is_some_and(|m| m.contributes.settings.is_some()),
        methods: manifest
            .map(|m| {
                m.contributes
                    .methods
                    .iter()
                    .map(|x| x.name.clone())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// One installed plugin for `plugin.list` and friends.
pub async fn installed(host: &PluginHost, view: &PluginView) -> InstalledPlugin {
    let providers = host
        .declared_providers(view)
        .await
        .into_iter()
        .map(|p| DeclaredProvider {
            id: p.id,
            name: p.name,
            icon: p.icon,
        })
        .collect();
    let manifest = view.manifest.as_deref();
    let id = view.record.id.clone();
    let failure = host.failure(&id).map(|f| PluginFailure {
        message: f.message,
        retry_in_seconds: f.retry_in.map(|d| d.as_secs_f64().ceil() as u64),
    });
    InstalledPlugin {
        name: manifest.map_or_else(|| id.clone(), |m| m.name.clone()),
        version: view.record.version.clone(),
        description: manifest.and_then(|m| m.description.clone()),
        repository: manifest.and_then(|m| m.repository.clone()),
        state: match view.record.state {
            KState::Installed => PluginState::Installed,
            KState::Enabled => PluginState::Enabled,
            KState::Disabled => PluginState::Disabled,
        },
        requested: permissions(&view.requested(), manifest),
        granted: view
            .record
            .granted
            .as_ref()
            .map(|g| permissions(g, manifest)),
        contributes: contributions(host, &id, manifest),
        providers,
        signed: view.record.publisher_key.is_some(),
        publisher: view.record.publisher_key.clone().map(|key| PublisherInfo {
            key,
            registry: match &view.record.source {
                PluginSource::Registry(url) => Some(url.clone()),
                _ => None,
            },
        }),
        source: Some(match view.record.source {
            PluginSource::Registry(_) => ProtocolSource::Registry,
            PluginSource::File => ProtocolSource::File,
            PluginSource::Bundled => ProtocolSource::Bundled,
        }),
        failure,
        error: view.error.clone(),
        id,
    }
}

/// Every installed plugin, by id.
pub async fn list(host: &PluginHost) -> Vec<InstalledPlugin> {
    let views = host.manager().list();
    let mut out = Vec::with_capacity(views.len());
    for v in &views {
        out.push(installed(host, v).await);
    }
    out
}

/// Enabled plugins and their available contributions, for `core.hello` and
/// `core.pluginsChanged`.
pub fn infos(host: &PluginHost) -> Vec<PluginInfo> {
    host.enabled()
        .into_iter()
        .map(|p| PluginInfo {
            id: p.record.id.clone(),
            version: p.record.version.clone(),
            contributes: Contributions {
                providers: p.providers,
                commands: p
                    .manifest
                    .contributes
                    .commands
                    .iter()
                    .map(|c| c.id.clone())
                    .collect(),
                settings: p.manifest.contributes.settings.is_some(),
                methods: p
                    .manifest
                    .contributes
                    .methods
                    .iter()
                    .map(|m| m.name.clone())
                    .collect(),
            },
        })
        .collect()
}

/// Reads the manifest inside a package without installing it.
pub fn peek_manifest(file: &Path) -> Result<Manifest, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    package::unpack(file, dir.path()).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(dir.path().join(MANIFEST_FILE))
        .map_err(|_| format!("{MANIFEST_FILE} is missing from the package"))?;
    Manifest::parse(&text).map_err(|e| e.to_string())
}

/// Installs a package file, or a directory holding a `plugin.toml`, which is packed
/// first exactly as `kumikit plugin pack` would.
pub async fn install(host: &PluginHost, path: &str) -> Result<InstalledPlugin, RpcError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(RpcError::new(
            ErrorCode::InvalidParams,
            "path must be absolute (a file or directory on the core's machine)",
        ));
    }
    let view = if path.is_dir() {
        let packed = pack_dir(path.to_owned()).await?;
        host.install(&packed.path().join("plugin.kkp"), PluginSource::File)
            .await
            .map_err(host_err)?
    } else {
        host.install(path, PluginSource::File)
            .await
            .map_err(host_err)?
    };
    Ok(installed(host, &view).await)
}

/// Installs a `.kkp` sent as bytes (base64) by a client on another machine. It is
/// written to a private temporary file and installed like a file on this machine.
pub async fn install_package(host: &PluginHost, base64: &str) -> Result<InstalledPlugin, RpcError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64.trim())
        .map_err(|e| {
            RpcError::new(
                ErrorCode::InvalidParams,
                format!("package is not base64: {e}"),
            )
        })?;
    let dir = tokio::task::spawn_blocking(move || -> std::io::Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("plugin.kkp"), &bytes)?;
        Ok(dir)
    })
    .await
    .map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?
    .map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?;
    let view = host
        .install(&dir.path().join("plugin.kkp"), PluginSource::File)
        .await
        .map_err(host_err)?;
    Ok(installed(host, &view).await)
}

/// Packs a plugin directory into `plugin.kkp` in a new temporary directory.
async fn pack_dir(dir: std::path::PathBuf) -> Result<tempfile::TempDir, RpcError> {
    tokio::task::spawn_blocking(move || {
        if !dir.join(MANIFEST_FILE).is_file() {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                format!("{} has no {MANIFEST_FILE}", dir.display()),
            ));
        }
        let out =
            tempfile::tempdir().map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?;
        package::pack(&dir, &out.path().join("plugin.kkp"))
            .map_err(|e| RpcError::new(ErrorCode::InvalidParams, e.to_string()))?;
        Ok(out)
    })
    .await
    .map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?
}

/// Grants and enables a plugin.
pub async fn enable(
    host: &PluginHost,
    id: &str,
    grant: &Permissions,
) -> Result<InstalledPlugin, RpcError> {
    let view = host.manager().get(id).map_err(host_err)?;
    let grants = grants(grant, view.manifest.as_deref())?;
    let view = host.enable(id, &grants).await.map_err(host_err)?;
    Ok(installed(host, &view).await)
}

/// Disables a plugin.
pub async fn disable(host: &PluginHost, id: &str) -> Result<InstalledPlugin, RpcError> {
    let view = host.disable(id).await.map_err(host_err)?;
    Ok(installed(host, &view).await)
}
