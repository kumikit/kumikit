//! `kumikit plugin add <id> | search | update | registry | lock`
//! (`docs/spec/10-distribution.md#install`, `#updates`, `#lockfiles`): the same calls as
//! the protocol's, against the running core or, when none runs, the data directory.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use kumikit_plugin_host::package::PublicKey;
use kumikit_protocol::lockfile;
use kumikit_protocol::methods::{
    AvailableUpdate, InstalledPlugin, Permissions, PluginInstall, PluginInstallParams, PluginLock,
    PluginState, PluginUpdate, PluginUpdateParams, PluginUpdates, PluginUpdatesParams, RegistryAdd,
    RegistryInfo, RegistryList, RegistryRef, RegistryRemove, RegistrySearch, RegistrySearchParams,
    RegistrySearchResult, RegistryUrl,
};
use kumikit_protocol::{ErrorCode, RpcError};

use super::{Backend, Out, Target, confirm, describe, rpc_err};

impl Backend {
    async fn install_from(&mut self, r: &RegistryRef) -> Result<InstalledPlugin, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginInstall>(&PluginInstallParams {
                    path: None,
                    package: None,
                    registry: Some(r.clone()),
                })
                .await?
                .plugin),
            Backend::Local { host, registry, .. } => registry.install(host, r).await,
        }
    }

    async fn search(&mut self, query: &str) -> Result<RegistrySearchResult, RpcError> {
        let query = (!query.is_empty()).then(|| query.to_owned());
        match self {
            Backend::Remote(rpc) => {
                rpc.call::<RegistrySearch>(&RegistrySearchParams {
                    query,
                    refresh: Some(true),
                })
                .await
            }
            Backend::Local { registry, .. } => registry.search(query.as_deref(), true).await,
        }
    }

    async fn updates(&mut self) -> Result<Vec<AvailableUpdate>, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginUpdates>(&PluginUpdatesParams {
                    refresh: Some(true),
                })
                .await?
                .updates),
            Backend::Local { host, registry, .. } => registry.updates(host, true).await,
        }
    }

    async fn update(
        &mut self,
        id: &str,
        version: &str,
        grant: Option<&Permissions>,
    ) -> Result<InstalledPlugin, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginUpdate>(&PluginUpdateParams {
                    plugin: id.to_owned(),
                    version: version.to_owned(),
                    grant: grant.cloned(),
                })
                .await?
                .plugin),
            Backend::Local { host, registry, .. } => {
                registry.update(host, id, version, grant).await
            }
        }
    }

    async fn registries(&mut self) -> Result<Vec<RegistryInfo>, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<RegistryList>(&Default::default())
                .await?
                .registries),
            Backend::Local { registry, .. } => registry.list().await,
        }
    }

    async fn registry_change(&mut self, url: &str, add: bool) -> Result<(), RpcError> {
        let params = RegistryUrl {
            url: url.to_owned(),
        };
        match (self, add) {
            (Backend::Remote(rpc), true) => rpc.call::<RegistryAdd>(&params).await.map(|_| ()),
            (Backend::Remote(rpc), false) => rpc.call::<RegistryRemove>(&params).await.map(|_| ()),
            (Backend::Local { registry, .. }, true) => registry.add(url).await,
            (Backend::Local { registry, .. }, false) => registry.remove(url).await,
        }
    }

    async fn lock(&mut self) -> Result<String, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc.call::<PluginLock>(&Default::default()).await?.lock),
            Backend::Local { host, .. } => Ok(host.manager().lock_text()),
        }
    }
}

/// Whether `add`'s argument names a package file rather than a registry id.
pub fn is_package_path(arg: &str) -> bool {
    arg.ends_with(".kkp") || Path::new(arg).exists()
}

fn key_line(p: &InstalledPlugin) -> String {
    match &p.publisher {
        Some(pubinfo) => {
            let key = PublicKey::parse(&pubinfo.key)
                .map(|k| k.grouped())
                .unwrap_or_else(|| pubinfo.key.clone());
            match &pubinfo.registry {
                Some(r) => format!("Signed by key {key}, the publisher's key in the registry {r}."),
                None => format!("Signed by key {key}; not checked against a registry."),
            }
        }
        None => "This package is not signed; Kumikit cannot check who made it.".into(),
    }
}

/// Prints who made a plugin and what it may do.
fn show(p: &InstalledPlugin, out: &mut dyn Out) {
    out.line(&format!("{} {} ({})", p.name, p.version, p.id));
    if let Some(d) = &p.description {
        out.line(d);
    }
    out.line(&format!(
        "Publisher: {}",
        p.id.split('.').next().unwrap_or_default()
    ));
    if let Some(r) = &p.repository {
        out.line(&format!("Repository: {r}"));
    }
    out.line(&key_line(p));
    out.line("It asks for these permissions. It:");
    for line in describe(&p.requested) {
        out.line(&format!("  - {line}"));
    }
}

/// `kumikit plugin add <id>[@<version>] [--registry <url>] [--yes]`.
pub async fn add_by_id(
    target: &Target,
    spec: &str,
    registry: Option<&str>,
    yes: bool,
    out: &mut dyn Out,
) -> Result<()> {
    let (id, version) = match spec.split_once('@') {
        Some((i, v)) => (i, Some(v.to_owned())),
        None => (spec, None),
    };
    let mut backend = Backend::open(target).await?;
    let r = RegistryRef {
        id: id.to_owned(),
        version,
        sha256: None,
        registry: registry.map(str::to_owned),
    };
    let plugin = match backend.install_from(&r).await {
        Ok(p) => p,
        Err(e) if e.data.code == ErrorCode::Conflict && e.message.contains("already installed") => {
            let existing = backend
                .list()
                .await?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or_else(|| rpc_err(e))?;
            if existing.state == PluginState::Enabled {
                out.line(&format!(
                    "{} {} is already installed and enabled. `kumikit plugin update {}` checks for a newer version.",
                    existing.id, existing.version, existing.id
                ));
                return Ok(());
            }
            existing
        }
        Err(e) => return Err(rpc_err(e)),
    };
    show(&plugin, out);
    if !yes && !confirm("Grant these permissions and enable it?") {
        out.line(&format!(
            "{} is installed but not enabled. Run `kumikit plugin add {} --yes` to grant and enable it.",
            plugin.id, plugin.id
        ));
        return Ok(());
    }
    let enabled = backend.enable(&plugin.id, &plugin.requested).await?;
    out.line(&format!("Enabled {}.", enabled.id));
    for p in &enabled.contributes.providers {
        out.line(&format!("Provider: {p}"));
    }
    Ok(())
}

/// `kumikit plugin search <words>`.
pub async fn search(target: &Target, words: &[String], out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let found = backend.search(&words.join(" ")).await.map_err(rpc_err)?;
    if found.results.is_empty() {
        out.line("No core plugins match.");
    }
    for r in &found.results {
        let description = r.description.as_deref().unwrap_or_default();
        out.line(&format!(
            "{} {}  {}: {description}",
            r.id, r.version, r.name
        ));
        out.line(&format!(
            "  publisher {} (key {}), from {}",
            r.publisher, r.key, r.registry_name
        ));
        for line in describe(&r.requested) {
            out.line(&format!("  - {line}"));
        }
    }
    Ok(())
}

/// `kumikit plugin update [<id>] [--yes] [--allow-wider]`: lists the updates, then applies
/// each one the user agrees to. `--yes` never widens permissions; that needs an
/// interactive yes or `--allow-wider`.
pub async fn update(
    target: &Target,
    only: Option<&str>,
    yes: bool,
    allow_wider: bool,
    out: &mut dyn Out,
) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let updates: Vec<AvailableUpdate> = backend
        .updates()
        .await
        .map_err(rpc_err)?
        .into_iter()
        .filter(|u| only.is_none_or(|id| u.plugin == id))
        .collect();
    if updates.is_empty() {
        out.line("Everything is up to date.");
        return Ok(());
    }
    for u in &updates {
        out.line(&format!("{} {} → {}", u.plugin, u.from, u.to));
        if u.widens {
            out.line("  It asks for more permissions. The new version:");
            for line in describe(&u.requested) {
                out.line(&format!("    - {line}"));
            }
        }
        let go = if u.widens {
            allow_wider || confirm("  Grant these permissions and update?")
        } else {
            yes || confirm("  Update?")
        };
        if !go {
            out.line(if u.widens {
                "  Not updated: it stays on the installed version until you grant the new permissions (--allow-wider)."
            } else {
                "  Not updated."
            });
            continue;
        }
        let grant = u.widens.then_some(&u.requested);
        match backend.update(&u.plugin, &u.to, grant).await {
            Ok(p) => out.line(&format!("  Updated {} to {}.", p.id, p.version)),
            Err(e) => out.line(&format!("  Not updated: {}", e.message)),
        }
    }
    Ok(())
}

/// `kumikit plugin registry list`.
pub async fn registry_list(target: &Target, out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let list = backend.registries().await.map_err(rpc_err)?;
    if list.is_empty() {
        out.line("No registries are configured.");
    }
    for r in list {
        match r.name {
            Some(n) => out.line(&format!("{}  ({n})", r.url)),
            None => out.line(&r.url),
        }
    }
    Ok(())
}

/// `kumikit plugin registry add <url>`.
pub async fn registry_add(target: &Target, url: &str, out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    backend.registry_change(url, true).await.map_err(rpc_err)?;
    out.line(&format!("Added {url}."));
    Ok(())
}

/// `kumikit plugin registry remove <url>`.
pub async fn registry_remove(target: &Target, url: &str, out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    backend.registry_change(url, false).await.map_err(rpc_err)?;
    out.line(&format!("Removed {url}."));
    Ok(())
}

/// `kumikit plugin lock export [-o <file>]`.
pub async fn lock_export(target: &Target, output: Option<&Path>, out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let text = backend.lock().await.map_err(rpc_err)?;
    match output {
        Some(path) => {
            std::fs::write(path, &text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            out.line(&format!("Wrote {}", path.display()));
        }
        None => {
            for line in text.lines() {
                out.line(line);
            }
        }
    }
    Ok(())
}

/// `kumikit plugin lock import <file> [--yes]`: installs what the lockfile lists, at the
/// locked versions and hashes, from registries already configured; then shows every
/// permission at once and enables what was enabled.
pub async fn lock_import(target: &Target, file: &Path, yes: bool, out: &mut dyn Out) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let lock = lockfile::parse(&text).map_err(|e| anyhow!("{}: {e}", file.display()))?;
    let mut backend = Backend::open(target).await?;
    let installed = backend.list().await?;
    let registries: Vec<String> = backend
        .registries()
        .await
        .map_err(rpc_err)?
        .into_iter()
        .map(|r| r.url)
        .collect();
    let mut to_enable: Vec<InstalledPlugin> = Vec::new();
    for entry in &lock.plugins {
        if let Some(have) = installed.iter().find(|p| p.id == entry.id) {
            if have.version == entry.version {
                out.line(&format!(
                    "{} {}: already installed.",
                    entry.id, entry.version
                ));
            } else {
                out.line(&format!(
                    "{}: {} is installed, the lockfile has {}; skipped (use `kumikit plugin update`).",
                    entry.id, have.version, entry.version
                ));
            }
            continue;
        }
        let Some(url) = entry.registry() else {
            out.line(&format!(
                "{} {}: installed from a {} on the other machine; install it from its file.",
                entry.id, entry.version, entry.source
            ));
            continue;
        };
        if !registries.iter().any(|r| r == url) {
            out.line(&format!(
                "{} {}: its registry {url} is not configured here; add it with `kumikit plugin registry add {url}` first.",
                entry.id, entry.version
            ));
            continue;
        }
        let r = RegistryRef {
            id: entry.id.clone(),
            version: Some(entry.version.clone()),
            sha256: Some(entry.sha256.clone()),
            registry: Some(url.to_owned()),
        };
        match backend.install_from(&r).await {
            Ok(p) => {
                out.line(&format!("Installed {} {}.", p.id, p.version));
                if entry.state.as_deref() == Some("enabled") {
                    to_enable.push(p);
                }
            }
            Err(e) => out.line(&format!(
                "{} {}: not installed: {}",
                entry.id, entry.version, e.message
            )),
        }
    }
    if to_enable.is_empty() {
        return Ok(());
    }
    out.line("These plugins were enabled on the other machine:");
    for p in &to_enable {
        out.line("");
        show(p, out);
    }
    if !yes && !confirm("Grant all these permissions and enable them?") {
        out.line("They are installed but not enabled.");
        return Ok(());
    }
    for p in &to_enable {
        let enabled = backend.enable(&p.id, &p.requested).await?;
        out.line(&format!("Enabled {}.", enabled.id));
    }
    Ok(())
}
