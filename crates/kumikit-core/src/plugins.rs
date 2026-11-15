//! The `plugin.*` methods and the plugin parts of `core.hello`: the plugin host's views
//! in protocol types (`docs/spec/02-protocol.md`).

use std::path::Path;

use kumikit_kernel::grants::{FilesGrant, Grants};
use kumikit_kernel::plugins::PluginState as KState;
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_plugin_host::manager::{HostError, PluginView};
use kumikit_plugin_host::manifest::Manifest;
use kumikit_protocol::methods::{
    Contributions, FilesAccess, InstalledPlugin, Limits, Permissions, PluginFailure, PluginInfo,
    PluginState,
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
    Permissions {
        spawn: g.spawn.iter().cloned().collect(),
        network: g.network.iter().cloned().collect(),
        files: match g.files {
            FilesGrant::None => FilesAccess::None,
            FilesGrant::ReadWorkspace => FilesAccess::ReadWorkspace,
            FilesGrant::Workspace => FilesAccess::Workspace,
        },
        secrets: g.secrets.iter().cloned().collect(),
        tools: g.tools,
        limits: manifest.and_then(limits),
    }
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
    Ok(Grants {
        spawn: p.spawn.iter().cloned().collect(),
        network: p.network.iter().cloned().collect(),
        files: match p.files {
            FilesAccess::None => FilesGrant::None,
            FilesAccess::ReadWorkspace => FilesGrant::ReadWorkspace,
            FilesAccess::Workspace => FilesGrant::Workspace,
        },
        secrets: p.secrets.iter().cloned().collect(),
        tools: p.tools,
    })
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
    }
}

/// One installed plugin for `plugin.list` and friends.
pub fn installed(host: &PluginHost, view: &PluginView) -> InstalledPlugin {
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
        signed: false,
        failure,
        error: view.error.clone(),
        id,
    }
}

/// Every installed plugin, by id.
pub fn list(host: &PluginHost) -> Vec<InstalledPlugin> {
    host.manager()
        .list()
        .iter()
        .map(|v| installed(host, v))
        .collect()
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
            },
        })
        .collect()
}

/// Installs a package file.
pub async fn install(host: &PluginHost, path: &str) -> Result<InstalledPlugin, RpcError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(RpcError::new(
            ErrorCode::InvalidParams,
            "path must be absolute (a file on the core's machine)",
        ));
    }
    let view = host.install(path).await.map_err(host_err)?;
    Ok(installed(host, &view))
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
    Ok(installed(host, &view))
}

/// Disables a plugin.
pub async fn disable(host: &PluginHost, id: &str) -> Result<InstalledPlugin, RpcError> {
    let view = host.disable(id).await.map_err(host_err)?;
    Ok(installed(host, &view))
}
