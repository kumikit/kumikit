//! Preinstalling bundled core plugins (`kumikit serve --bundled-plugins <dir>`,
//! `docs/spec/10-distribution.md#bundled-core-plugins`). Names no plugin: the directory
//! decides what is seeded.

use std::path::{Path, PathBuf};

use kumikit_kernel::grants::Grants;
use kumikit_kernel::plugins::PluginSource;
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_plugin_host::manager::PluginView;
use kumikit_plugin_host::manifest::Manifest;

/// What happened to one bundled package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seeded {
    /// Installed and enabled (it requests no permissions).
    Enabled(String),
    /// Installed; it waits for the user to grant its permissions.
    Installed(String),
    /// A newer bundled version replaced the bundled copy; enabled again with the same
    /// grant (`true`), or waiting for a grant because it asks for more (`false`).
    Updated(String, bool),
    /// Seeded before (possibly removed since) or already installed; left alone.
    Skipped(String),
    /// Could not be read or installed; logged.
    Failed(PathBuf, String),
}

/// Seeds every `.kkp` in `dir`, in file-name order. Problems with one package are
/// logged and do not stop the others.
pub async fn seed(host: PluginHost, dir: PathBuf) -> Vec<Seeded> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "kkp"))
            .collect(),
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "cannot read bundled plugins");
            return Vec::new();
        }
    };
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let outcome = match seed_one(&host, &file).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(package = %file.display(), error = %e, "cannot seed a bundled plugin");
                Seeded::Failed(file, e)
            }
        };
        out.push(outcome);
    }
    out
}

async fn seed_one(host: &PluginHost, file: &Path) -> Result<Seeded, String> {
    let manifest = {
        let file = file.to_owned();
        tokio::task::spawn_blocking(move || crate::plugins::peek_manifest(&file))
            .await
            .map_err(|e| e.to_string())??
    };
    let kernel = host.manager().kernel().clone();
    let id = manifest.id.clone();
    if kernel
        .plugin_was_seeded(&id)
        .await
        .map_err(|e| e.to_string())?
    {
        return match host.manager().get(&id) {
            Ok(view)
                if view.record.source == PluginSource::Bundled
                    && newer(&manifest.version, &view.record.version) =>
            {
                update(host, file, &manifest, &view).await
            }
            _ => Ok(Seeded::Skipped(id)),
        };
    }
    if host.manager().get(&id).is_ok() {
        // Installed by the user before the bundle reached it: theirs to manage.
        kernel
            .plugin_mark_seeded(&id, &manifest.version)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(Seeded::Skipped(id));
    }
    let view = host
        .install(file, PluginSource::Bundled)
        .await
        .map_err(|e| e.to_string())?;
    kernel
        .plugin_mark_seeded(&id, &manifest.version)
        .await
        .map_err(|e| e.to_string())?;
    if view.requested() == Grants::default() {
        host.enable(&id, &Grants::default())
            .await
            .map_err(|e| e.to_string())?;
        tracing::info!(plugin = %id, "seeded and enabled a bundled plugin");
        Ok(Seeded::Enabled(id))
    } else {
        tracing::info!(plugin = %id, "seeded a bundled plugin; it waits for a grant");
        Ok(Seeded::Installed(id))
    }
}

fn newer(candidate: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(installed),
    ) {
        (Ok(c), Ok(i)) => c > i,
        _ => false,
    }
}

/// Replaces the bundled copy with a newer bundled version
/// (`docs/spec/10-distribution.md#bundled-core-plugins`): the grant is kept when the new
/// version asks for no more; otherwise it is installed without one and waits for the user.
async fn update(
    host: &PluginHost,
    file: &Path,
    manifest: &Manifest,
    view: &PluginView,
) -> Result<Seeded, String> {
    let id = manifest.id.clone();
    let requested = manifest.requested();
    let widens =
        kumikit_registry::resolve::widens(&crate::registry::current_permissions(view), &requested);
    let grant = match (&view.record.granted, widens) {
        (Some(_), false) => Some(manifest.permissions.to_grants()),
        _ => None,
    };
    let kept = grant.is_some();
    host.update(&id, file, PluginSource::Bundled, None, grant)
        .await
        .map_err(|e| e.to_string())?;
    host.manager()
        .kernel()
        .plugin_mark_seeded(&id, &manifest.version)
        .await
        .map_err(|e| e.to_string())?;
    if kept {
        tracing::info!(plugin = %id, version = %manifest.version, "updated a bundled plugin");
    } else {
        tracing::info!(plugin = %id, version = %manifest.version, "updated a bundled plugin; it waits for a grant");
    }
    Ok(Seeded::Updated(id, kept))
}
