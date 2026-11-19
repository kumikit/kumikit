//! UI plugins and themes from registries (`docs/spec/06-client.md#shell-rust-kumikit-shell`,
//! [ADR 0018](../../../docs/decisions/0018-package-signing-and-registry.md)): each side
//! fetches what it installs, so the shell fetches these for its client plugin store, with
//! the same checks as a core. Registries are read only when the user asks.

use std::path::Path;

use kumikit_registry::client::{FetchedIndex, check_url, default_registry};
use kumikit_registry::fetch::{Fetcher, Limits};
use kumikit_registry::index::{Index, Plugin, UiNeeds, Version};
use kumikit_registry::resolve;
use kumikit_registry::{Freshness, RegistryClient, RegistryError, RegistryList};
use serde::{Deserialize, Serialize};

use crate::store::{Entry, Expected, Store, StoreError, UI_SDK_VERSION, widens};

/// A UI plugin or theme a registry offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientResult {
    /// Plugin id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `["ui"]` or `["theme"]`.
    pub kind: Vec<String>,
    /// The publisher's display name.
    pub publisher: String,
    /// The publisher's key id.
    pub key: String,
    /// The registry's index URL.
    pub registry: String,
    /// The registry's name.
    pub registry_name: String,
    /// Source repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The latest version this app can run.
    pub version: String,
    /// Its isolation, network hosts and core needs.
    pub ui: UiNeeds,
    /// Whether it is in the registry's starter pack.
    pub starter: bool,
}

/// An update a registry offers for an installed UI plugin or theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientUpdate {
    /// Plugin id.
    pub plugin: String,
    /// Installed version.
    pub from: String,
    /// Offered version.
    pub to: String,
    /// The registry offering it.
    pub registry: String,
    /// What the new version needs.
    pub ui: UiNeeds,
    /// Whether it needs more (new hosts, or inline code): confirm before applying.
    pub widens: bool,
}

/// A plugin to install from a registry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientRef {
    /// Plugin id.
    pub id: String,
    /// A version; default: the latest this app can run.
    #[serde(default)]
    pub version: Option<String>,
    /// The SHA-256 it must have (from a lockfile).
    #[serde(default)]
    pub sha256: Option<String>,
    /// One registry's index URL instead of the list.
    #[serde(default)]
    pub registry: Option<String>,
}

/// A failure, as the UI kernel shows it: a code it can act on and a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientError {
    /// `unavailable`, `untrusted`, `invalid`, `not_found`, `conflict` or `internal`.
    pub code: &'static str,
    /// For the user.
    pub message: String,
}

impl From<RegistryError> for ClientError {
    fn from(e: RegistryError) -> Self {
        let code = match &e {
            RegistryError::Unavailable(_) => "unavailable",
            RegistryError::Untrusted(_) => "untrusted",
            RegistryError::Invalid(_) => "invalid",
            RegistryError::NotFound(_) => "not_found",
        };
        ClientError {
            code,
            message: e.to_string(),
        }
    }
}

impl From<StoreError> for ClientError {
    fn from(e: StoreError) -> Self {
        let code = match &e {
            StoreError::Invalid(_) => "invalid",
            StoreError::Conflict(_) => "conflict",
            StoreError::NotFound(_) => "not_found",
            StoreError::Untrusted(_) => "untrusted",
            StoreError::Io(_) => "internal",
        };
        ClientError {
            code,
            message: e.to_string(),
        }
    }
}

fn error(code: &'static str, message: impl Into<String>) -> ClientError {
    ClientError {
        code,
        message: message.into(),
    }
}

/// The client's registries: their list, a cached client, and the store it installs into.
#[derive(Debug, Clone)]
pub struct ClientRegistry {
    list: RegistryList,
    client: RegistryClient,
    store: Store,
}

impl ClientRegistry {
    /// The registries of the client whose data directory is `client_dir`
    /// (`registries.toml`, `registry-cache/`); nothing is read until asked.
    pub fn new(
        client_dir: &Path,
        store: Store,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> ClientRegistry {
        ClientRegistry {
            list: RegistryList::new(client_dir.join("registries.toml"), default_registry(env)),
            client: RegistryClient::new(
                Fetcher::new(Limits::default()),
                client_dir.join("registry-cache"),
            ),
            store,
        }
    }

    /// The same, with another fetcher (tests use one that trusts their own certificate).
    pub fn with_client(mut self, client: RegistryClient) -> ClientRegistry {
        self.client = client;
        self
    }

    /// The registries, in order.
    pub async fn registries(&self) -> Result<Vec<String>, ClientError> {
        Ok(self.list.get().await?)
    }

    /// Replaces the list.
    pub async fn set_registries(&self, urls: &[String]) -> Result<(), ClientError> {
        Ok(self.list.set(urls).await?)
    }

    async fn indexes(
        &self,
        only: Option<&str>,
        f: Freshness,
    ) -> Result<Vec<FetchedIndex>, ClientError> {
        let urls = match only {
            Some(u) => {
                check_url(u)?;
                vec![u.to_owned()]
            }
            None => self.list.get().await?,
        };
        let mut got = Vec::new();
        let mut failed = Vec::new();
        for url in urls {
            match self.client.index(&url, f).await {
                Ok(i) => got.push(i),
                Err(RegistryError::Invalid(m)) => return Err(error("invalid", m)),
                Err(e) => failed.push(e.to_string()),
            }
        }
        if got.is_empty() && !failed.is_empty() {
            return Err(error("unavailable", failed.join("; ")));
        }
        Ok(got)
    }

    /// UI plugins and themes whose id, name or description holds every word.
    pub async fn search(
        &self,
        query: &str,
        refresh: bool,
    ) -> Result<Vec<ClientResult>, ClientError> {
        let f = if refresh {
            Freshness::Revalidate
        } else {
            Freshness::Cached
        };
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut out = Vec::new();
        for fi in self.indexes(None, f).await? {
            for p in fi.index.plugins.iter().filter(|p| p.is_client_side()) {
                let hay = format!(
                    "{} {} {}",
                    p.id,
                    p.name,
                    p.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if !words.iter().all(|w| hay.contains(w)) {
                    continue;
                }
                let Some(publisher) = fi.index.publishers.get(p.publisher()) else {
                    continue;
                };
                let Ok(v) = resolve::choose(p, None, UI_SDK_VERSION) else {
                    continue;
                };
                out.push(ClientResult {
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
                    ui: v.ui.clone().unwrap_or_default(),
                    starter: fi.index.starter.contains(&p.id),
                });
            }
        }
        Ok(out)
    }

    async fn locate(
        &self,
        id: &str,
        only: Option<&str>,
    ) -> Result<(FetchedIndex, Plugin), ClientError> {
        for fi in self.indexes(only, Freshness::Revalidate).await? {
            if let Some(p) = resolve::find(&fi.index, id).cloned() {
                return Ok((fi, p));
            }
        }
        Err(error("not_found", format!("no registry lists {id}")))
    }

    async fn download(
        &self,
        v: &Version,
        sha256: Option<&str>,
    ) -> Result<tempfile::TempDir, ClientError> {
        let bytes = self.client.download(v, sha256).await?;
        let dir = tempfile::tempdir().map_err(|e| error("internal", e.to_string()))?;
        tokio::fs::write(dir.path().join("plugin.kkp"), bytes)
            .await
            .map_err(|e| error("internal", e.to_string()))?;
        Ok(dir)
    }

    /// Installs a UI plugin or theme from a registry, checked as a core checks.
    pub async fn install(&self, r: &ClientRef) -> Result<Entry, ClientError> {
        let (fi, plugin) = self.locate(&r.id, r.registry.as_deref()).await?;
        if !plugin.is_client_side() {
            return Err(error(
                "invalid",
                format!("{} is a core plugin; install it on a core", r.id),
            ));
        }
        let v = resolve::choose(&plugin, r.version.as_deref(), UI_SDK_VERSION)
            .map_err(|e| error("conflict", e.to_string()))?;
        let expected = expected(&fi.index, &plugin, v)?;
        let file = self.download(v, r.sha256.as_deref()).await?;
        let store = self.store.clone();
        let source = format!("registry+{}", fi.url);
        tokio::task::spawn_blocking(move || {
            store.install_checked(&file.path().join("plugin.kkp"), &source, Some(&expected))
        })
        .await
        .map_err(|e| error("internal", e.to_string()))?
        .map_err(ClientError::from)
    }

    /// Updates the registries offer for installed UI plugins and themes; applies nothing.
    pub async fn updates(&self, refresh: bool) -> Result<Vec<ClientUpdate>, ClientError> {
        let f = if refresh {
            Freshness::Revalidate
        } else {
            Freshness::Cached
        };
        let mut out = Vec::new();
        for entry in self
            .store
            .list()
            .into_iter()
            .filter(|e| e.source == "installed")
        {
            let Some(meta) = self.store.meta(&entry.id) else {
                continue;
            };
            let Some(url) = meta.source.strip_prefix("registry+") else {
                continue;
            };
            let Ok(fi) = self.client.index(url, f).await else {
                continue;
            };
            let Some(p) = resolve::find(&fi.index, &entry.id) else {
                continue;
            };
            let Some(v) = resolve::update(p, &meta.version, UI_SDK_VERSION) else {
                continue;
            };
            let ui = v.ui.clone().unwrap_or_default();
            out.push(ClientUpdate {
                plugin: entry.id.clone(),
                from: meta.version.clone(),
                to: v.version.clone(),
                registry: url.to_owned(),
                widens: widens(&crate::store::ui_needs(&entry.manifest), &ui),
                ui,
            });
        }
        Ok(out)
    }

    /// Applies an update from the registry the plugin came from. The UI asks first when
    /// the update widens what the plugin needs.
    pub async fn update(&self, id: &str, version: &str) -> Result<Entry, ClientError> {
        let meta = self
            .store
            .meta(id)
            .ok_or_else(|| error("not_found", format!("{id} is not installed")))?;
        let url = meta
            .source
            .strip_prefix("registry+")
            .ok_or_else(|| {
                error(
                    "conflict",
                    format!("{id} was not installed from a registry"),
                )
            })?
            .to_owned();
        let newer = matches!(
            (semver::Version::parse(version), semver::Version::parse(&meta.version)),
            (Ok(n), Ok(o)) if n > o
        );
        if !newer {
            return Err(error(
                "conflict",
                format!(
                    "{id} {version} is not newer than the installed {}",
                    meta.version
                ),
            ));
        }
        let (fi, plugin) = self.locate(id, Some(&url)).await?;
        let v = resolve::choose(&plugin, Some(version), UI_SDK_VERSION)
            .map_err(|e| error("conflict", e.to_string()))?;
        let expected = expected(&fi.index, &plugin, v)?;
        let file = self.download(v, None).await?;
        let store = self.store.clone();
        let id = id.to_owned();
        let source = format!("registry+{url}");
        tokio::task::spawn_blocking(move || {
            store.replace(
                &id,
                &file.path().join("plugin.kkp"),
                &source,
                Some(&expected),
            )
        })
        .await
        .map_err(|e| error("internal", e.to_string()))?
        .map_err(ClientError::from)
    }
}

fn expected(index: &Index, plugin: &Plugin, v: &Version) -> Result<Expected, ClientError> {
    let publisher = index.publishers.get(plugin.publisher()).ok_or_else(|| {
        error(
            "untrusted",
            format!("the registry has no key for the publisher of {}", plugin.id),
        )
    })?;
    Ok(Expected {
        key: publisher.key.clone(),
        id: plugin.id.clone(),
        version: v.version.clone(),
        api: v.api.clone(),
        ui: v.ui.clone().unwrap_or_default(),
    })
}
