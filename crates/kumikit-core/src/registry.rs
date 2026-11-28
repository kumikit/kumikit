//! Core plugins from registries (`docs/spec/10-distribution.md#install`, `#updates`):
//! search, install by id, updates and the registry list, behind `registry.*`,
//! `plugin.install { registry }`, `plugin.updates` and `plugin.update`, and the CLI.
//!
//! Nothing here runs at startup. Registries are read only when one of these is called,
//! which happens only when the user asks.

use std::path::Path;

use kumikit_kernel::grants::Grants;
use kumikit_kernel::plugins::PluginSource;
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_plugin_host::manager::{Expected, PluginView, protocol_permissions};
use kumikit_plugin_host::manifest::PLUGIN_API;
use kumikit_protocol::methods::{
    AvailableUpdate, InstalledPlugin, Permissions, RegistryInfo, RegistryRef, RegistryResult,
    RegistrySearchResult,
};
use kumikit_protocol::{ErrorCode, RpcError};
use kumikit_registry::client::{FetchedIndex, default_registry};
use kumikit_registry::fetch::{Fetcher, Limits};
use kumikit_registry::index::{Index, Plugin, Version, normalized};
use kumikit_registry::resolve::{self, ResolveError};
use kumikit_registry::{Freshness, RegistryClient, RegistryError, RegistryList};

/// The core's registries: their list and a cached client.
#[derive(Debug)]
pub struct CoreRegistry {
    list: RegistryList,
    client: RegistryClient,
}

/// A registry error as a protocol error.
pub fn registry_err(e: RegistryError) -> RpcError {
    match e {
        RegistryError::Unavailable(m) => RpcError::new(ErrorCode::Unavailable, m),
        RegistryError::Untrusted(m) => RpcError::new(ErrorCode::Untrusted, m),
        RegistryError::Invalid(m) => RpcError::new(ErrorCode::InvalidParams, m),
        RegistryError::NotFound(m) => RpcError::new(ErrorCode::NotFound, m),
    }
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Internal, e.to_string())
}

fn time(t: std::time::SystemTime) -> String {
    let t: time::OffsetDateTime = t.into();
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The API version manifests' `api` ranges are checked against.
fn api() -> String {
    PLUGIN_API.to_string()
}

/// Protocol permissions as kernel grants (limits are part of the version, not the grant).
pub fn to_grants(p: &Permissions) -> Grants {
    crate::plugins::grants_of(p)
}

/// What an installed plugin may do now: its grant, or what it asks for if never granted.
pub(crate) fn current_permissions(view: &PluginView) -> Permissions {
    match (&view.record.granted, &view.manifest) {
        (Some(g), m) => protocol_permissions(g, m.as_deref()),
        (None, Some(m)) => m.requested(),
        (None, None) => Permissions::default(),
    }
}

impl CoreRegistry {
    /// The registries of the core whose data directory is `data_dir`: the list in
    /// `registries.toml` (default: `KUMIKIT_DEFAULT_REGISTRY` or the official one) and
    /// the cache in `registry-cache/`. Nothing is read until a method is called.
    pub fn new(data_dir: &Path, env: &dyn Fn(&str) -> Option<String>) -> CoreRegistry {
        CoreRegistry {
            list: RegistryList::new(data_dir.join("registries.toml"), default_registry(env)),
            client: RegistryClient::new(
                Fetcher::new(Limits::default()),
                data_dir.join("registry-cache"),
            ),
        }
    }

    /// `registry.list`: the list, with names from cached indexes (no request is made).
    pub async fn list(&self) -> Result<Vec<RegistryInfo>, RpcError> {
        let mut out = Vec::new();
        for url in self.list.get().await.map_err(registry_err)? {
            let name = self.client.cached_index(&url).await.map(|i| i.index.name);
            out.push(RegistryInfo { url, name });
        }
        Ok(out)
    }

    /// `registry.add`.
    pub async fn add(&self, url: &str) -> Result<(), RpcError> {
        self.list.add(url).await.map_err(registry_err)
    }

    /// `registry.remove`.
    pub async fn remove(&self, url: &str) -> Result<(), RpcError> {
        self.list.remove(url).await.map_err(registry_err)
    }

    /// Reads every registry; failures are kept so a caller can say why nothing was found.
    async fn indexes(
        &self,
        only: Option<&str>,
        freshness: Freshness,
    ) -> Result<(Vec<FetchedIndex>, Vec<String>), RpcError> {
        let urls = match only {
            Some(u) => vec![u.to_owned()],
            None => self.list.get().await.map_err(registry_err)?,
        };
        let mut got = Vec::new();
        let mut failed = Vec::new();
        for url in urls {
            match self.client.index(&url, freshness).await {
                Ok(i) => got.push(i),
                Err(RegistryError::Invalid(m)) => {
                    return Err(RpcError::new(ErrorCode::InvalidParams, m));
                }
                Err(e) => {
                    tracing::info!(registry = %url, error = %e, "registry unavailable");
                    failed.push(e.to_string());
                }
            }
        }
        Ok((got, failed))
    }

    /// `registry.search`: core plugins whose id, name or description holds every word.
    pub async fn search(
        &self,
        query: Option<&str>,
        refresh: bool,
    ) -> Result<RegistrySearchResult, RpcError> {
        let freshness = if refresh {
            Freshness::Revalidate
        } else {
            Freshness::Cached
        };
        let (indexes, failed) = self.indexes(None, freshness).await?;
        if indexes.is_empty() && !failed.is_empty() {
            return Err(RpcError::new(ErrorCode::Unavailable, failed.join("; ")));
        }
        let words: Vec<String> = query
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let api = api();
        let mut results = Vec::new();
        for fi in &indexes {
            for p in fi.index.plugins.iter().filter(|p| !p.is_client_side()) {
                let haystack = format!(
                    "{} {} {}",
                    p.id,
                    p.name,
                    p.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if !words.iter().all(|w| haystack.contains(w)) {
                    continue;
                }
                let Some(publisher) = fi.index.publishers.get(p.publisher()) else {
                    continue;
                };
                let Ok(v) = resolve::choose(p, None, &api) else {
                    continue;
                };
                results.push(RegistryResult {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                    kind: p.kind.clone(),
                    publisher: publisher.name.clone(),
                    key: publisher.key.clone(),
                    registry: fi.url.clone(),
                    registry_name: fi.index.name.clone(),
                    repository: p.repository.clone(),
                    version: v.version.clone(),
                    requested: v.requested.clone().unwrap_or_default(),
                    starter: fi.index.starter.contains(&p.id),
                });
            }
        }
        let fetched = indexes.iter().map(|i| i.fetched).min().map(time);
        Ok(RegistrySearchResult { results, fetched })
    }

    /// Finds `id` in the registries (or in `only`), revalidating each index.
    async fn locate(
        &self,
        id: &str,
        only: Option<&str>,
    ) -> Result<(FetchedIndex, Plugin), RpcError> {
        let (indexes, failed) = self.indexes(only, Freshness::Revalidate).await?;
        for fi in indexes {
            if let Some(p) = resolve::find(&fi.index, id).cloned() {
                return Ok((fi, p));
            }
        }
        if failed.is_empty() {
            Err(RpcError::new(
                ErrorCode::NotFound,
                format!("no registry lists {id}"),
            ))
        } else {
            Err(RpcError::new(
                ErrorCode::Unavailable,
                format!(
                    "{id} was not found, and some registries could not be read: {}",
                    failed.join("; ")
                ),
            ))
        }
    }

    /// Downloads a version into a private temporary file.
    async fn fetch(
        &self,
        version: &Version,
        sha256: Option<&str>,
    ) -> Result<tempfile::TempDir, RpcError> {
        let bytes = self
            .client
            .download(version, sha256)
            .await
            .map_err(registry_err)?;
        tokio::task::spawn_blocking(move || -> std::io::Result<tempfile::TempDir> {
            let dir = tempfile::tempdir()?;
            std::fs::write(dir.path().join("plugin.kkp"), &bytes)?;
            Ok(dir)
        })
        .await
        .map_err(internal)?
        .map_err(internal)
    }

    /// `plugin.install { registry }`: resolve, download, check the hash, the signature
    /// against the publisher's key and the manifest against the entry, then install.
    pub async fn install(
        &self,
        host: &PluginHost,
        r: &RegistryRef,
    ) -> Result<InstalledPlugin, RpcError> {
        if let Some(url) = &r.registry {
            kumikit_registry::client::check_url(url).map_err(registry_err)?;
        }
        let (fi, plugin) = self.locate(&r.id, r.registry.as_deref()).await?;
        if plugin.is_client_side() {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                format!(
                    "{} is a UI plugin or theme; install it in the app, not on a core",
                    r.id
                ),
            ));
        }
        let version =
            resolve::choose(&plugin, r.version.as_deref(), &api()).map_err(resolve_err)?;
        let expected = expected(&fi.index, &plugin, version)?;
        let file = self.fetch(version, r.sha256.as_deref()).await?;
        let view = host
            .install_checked(
                &file.path().join("plugin.kkp"),
                PluginSource::Registry(fi.url.clone()),
                Some(&expected),
            )
            .await
            .map_err(crate::plugins::host_err)?;
        Ok(crate::plugins::installed(host, &view).await)
    }

    /// `plugin.updates`: for each plugin installed from a registry, the highest version
    /// above it that its registry offers and this core can run. Applies nothing.
    pub async fn updates(
        &self,
        host: &PluginHost,
        refresh: bool,
    ) -> Result<Vec<AvailableUpdate>, RpcError> {
        let freshness = if refresh {
            Freshness::Revalidate
        } else {
            Freshness::Cached
        };
        let api = api();
        let mut out = Vec::new();
        let mut failed = Vec::new();
        for view in host.manager().list() {
            let PluginSource::Registry(url) = &view.record.source else {
                continue;
            };
            let fi = match self.client.index(url, freshness).await {
                Ok(i) => i,
                Err(e) => {
                    failed.push(e.to_string());
                    continue;
                }
            };
            let Some(plugin) = resolve::find(&fi.index, &view.record.id) else {
                continue;
            };
            let Some(v) = resolve::update(plugin, &view.record.version, &api) else {
                continue;
            };
            let requested = normalized(v.requested.clone().unwrap_or_default());
            out.push(AvailableUpdate {
                plugin: view.record.id.clone(),
                from: view.record.version.clone(),
                to: v.version.clone(),
                registry: url.clone(),
                widens: resolve::widens(&current_permissions(&view), &requested),
                requested,
            });
        }
        if out.is_empty() && !failed.is_empty() {
            return Err(RpcError::new(ErrorCode::Unavailable, failed.join("; ")));
        }
        Ok(out)
    }

    /// `plugin.update`: install `version` of a plugin from the registry it came from.
    /// Wider permissions need `grant` equal to the new requested set; nothing changes
    /// otherwise.
    pub async fn update(
        &self,
        host: &PluginHost,
        id: &str,
        version: &str,
        grant: Option<&Permissions>,
    ) -> Result<InstalledPlugin, RpcError> {
        let view = host.manager().get(id).map_err(crate::plugins::host_err)?;
        let PluginSource::Registry(url) = view.record.source.clone() else {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                format!(
                    "{id} was not installed from a registry; install a new package file instead"
                ),
            ));
        };
        let newer = match (
            semver::Version::parse(version),
            semver::Version::parse(&view.record.version),
        ) {
            (Ok(new), Ok(old)) => new > old,
            _ => false,
        };
        if !newer {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                format!(
                    "{id} {version} is not newer than the installed {}",
                    view.record.version
                ),
            ));
        }
        let (fi, plugin) = self.locate(id, Some(&url)).await?;
        let v = resolve::choose(&plugin, Some(version), &api()).map_err(resolve_err)?;
        let requested = normalized(v.requested.clone().unwrap_or_default());
        let widens = resolve::widens(&current_permissions(&view), &requested);
        if widens && grant.map(|g| normalized(g.clone())) != Some(requested.clone()) {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                format!("{id} {version} asks for more permissions; grant them to update"),
            ));
        }
        let new_grant = view.record.granted.as_ref().map(|_| to_grants(&requested));
        let expected = expected(&fi.index, &plugin, v)?;
        let file = self.fetch(v, None).await?;
        let updated = host
            .update(
                id,
                &file.path().join("plugin.kkp"),
                PluginSource::Registry(url),
                Some(&expected),
                new_grant,
            )
            .await
            .map_err(crate::plugins::host_err)?;
        Ok(crate::plugins::installed(host, &updated).await)
    }
}

fn resolve_err(e: ResolveError) -> RpcError {
    let code = match e {
        ResolveError::NoSuchVersion { .. } => ErrorCode::NotFound,
        ResolveError::Yanked { .. } | ResolveError::Incompatible { .. } => ErrorCode::Conflict,
    };
    RpcError::new(code, e.to_string())
}

/// What the registry says the package is.
fn expected(index: &Index, plugin: &Plugin, v: &Version) -> Result<Expected, RpcError> {
    let publisher = index.publishers.get(plugin.publisher()).ok_or_else(|| {
        RpcError::new(
            ErrorCode::Untrusted,
            format!("the registry has no key for the publisher of {}", plugin.id),
        )
    })?;
    Ok(Expected {
        key: publisher.key.clone(),
        id: plugin.id.clone(),
        version: v.version.clone(),
        api: v.api.clone(),
        requested: Some(v.requested.clone().unwrap_or_default()),
    })
}
