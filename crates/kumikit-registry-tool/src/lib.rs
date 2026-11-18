//! The registry's checker and index builder (`docs/spec/10-distribution.md#registry`).
//!
//! `kumikit-registry check <registry> [--base <main>]` is what the registry's CI runs on a
//! pull request: it downloads every new version and checks its hash, its signature by the
//! publisher's key, its manifest, the fields the registry copies from it, key continuity,
//! immutable versions, and confusable ids. `build-index` writes the `index.json` clients
//! read. The fixture registries of Kumikit's own tests use the same code.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kumikit_package::PublicKey;
use kumikit_plugin_host::manifest::Manifest;
use kumikit_protocol::methods::Permissions;
use kumikit_registry::confusable;
use kumikit_registry::fetch::{Fetched, Fetcher};
use kumikit_registry::index::{
    FORMAT, Index, PermissionsFile, Plugin, PluginFile, Publisher, PublisherFile, UiNeeds, Version,
    VersionFile, normalized,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// What a package says about itself: the fields a registry entry copies.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    /// `id`.
    pub id: String,
    /// `version`.
    pub version: String,
    /// `kind`.
    pub kind: Vec<String>,
    /// `api`.
    pub api: String,
    /// A core plugin's requested permissions.
    pub requested: Option<Permissions>,
    /// A UI plugin's or theme's needs.
    pub ui: Option<UiNeeds>,
    /// Who signed it.
    pub key: Option<PublicKey>,
}

#[derive(Deserialize)]
struct ClientManifest {
    id: String,
    version: String,
    kind: Vec<String>,
    api: String,
    #[serde(default)]
    ui: Option<ClientUi>,
    #[serde(default)]
    requires: Option<ClientRequires>,
}

#[derive(Deserialize)]
struct ClientUi {
    #[serde(default)]
    isolation: Option<String>,
    #[serde(default)]
    network: Vec<String>,
}

#[derive(Deserialize)]
struct ClientRequires {
    #[serde(default)]
    core: Vec<String>,
}

fn is_client_kind(kind: &[String]) -> bool {
    !kind.is_empty() && kind.iter().all(|k| k == "ui" || k == "theme")
}

/// Reads an unpacked package: verifies its signature and validates its manifest (core
/// plugins with the core's own validation).
pub fn facts(dir: &Path) -> Result<Facts, String> {
    let key = kumikit_package::signature::verify_dir(dir)
        .map_err(|e| e.to_string())?
        .key();
    let text = std::fs::read_to_string(dir.join("plugin.toml"))
        .map_err(|e| format!("plugin.toml: {e}"))?;
    #[derive(Deserialize)]
    struct Kind {
        kind: Vec<String>,
    }
    let kind: Kind = toml::from_str(&text).map_err(|e| format!("plugin.toml: {e}"))?;
    if is_client_kind(&kind.kind) {
        let m: ClientManifest = toml::from_str(&text).map_err(|e| format!("plugin.toml: {e}"))?;
        let mut network = m.ui.as_ref().map(|u| u.network.clone()).unwrap_or_default();
        network.sort();
        let mut requires_core = m.requires.map(|r| r.core).unwrap_or_default();
        requires_core.sort();
        Ok(Facts {
            id: m.id,
            version: m.version,
            kind: m.kind,
            api: m.api,
            requested: None,
            ui: Some(UiNeeds {
                isolation: m.ui.and_then(|u| u.isolation.or(Some("inline".into()))),
                network,
                requires_core,
            }),
            key,
        })
    } else {
        let m = Manifest::load(dir).map_err(|e| e.to_string())?;
        Ok(Facts {
            id: m.id.clone(),
            version: m.version.clone(),
            kind: kind.kind,
            api: m.api.clone(),
            requested: Some(m.requested()),
            ui: None,
            key,
        })
    }
}

/// Reads a `.kkp`'s bytes: unpacks them into a temporary directory and returns its facts.
pub fn facts_of_package(bytes: &[u8]) -> Result<Facts, String> {
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = work.path().join("p.kkp");
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let dir = work.path().join("p");
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    kumikit_package::unpack(&file, &dir).map_err(|e| e.to_string())?;
    facts(&dir)
}

/// The `[[versions]]` entry a plugin file holds for a package with these facts.
pub fn version_entry(facts: &Facts, url: &str, sha256: &str) -> VersionFile {
    let (permissions, limits) = match &facts.requested {
        Some(p) => (
            Some(PermissionsFile {
                spawn: p.spawn.clone(),
                network: p.network.clone(),
                files: Some(
                    match p.files {
                        kumikit_protocol::methods::FilesAccess::None => "none",
                        kumikit_protocol::methods::FilesAccess::ReadWorkspace => "read-workspace",
                        kumikit_protocol::methods::FilesAccess::Workspace => "workspace",
                    }
                    .to_owned(),
                ),
                secrets: p.secrets.clone(),
                tools: p.tools,
            }),
            p.limits.map(|l| kumikit_registry::index::LimitsFile {
                memory_mb: l.memory_mb,
                call_timeout_s: l.call_timeout_s,
            }),
        ),
        None => (None, None),
    };
    let ui = facts.ui.clone().unwrap_or_default();
    VersionFile {
        version: facts.version.clone(),
        url: url.to_owned(),
        sha256: sha256.to_owned(),
        api: facts.api.clone(),
        permissions,
        limits,
        isolation: ui.isolation,
        network: ui.network,
        requires_core: ui.requires_core,
        yanked: false,
    }
}

/// A registry directory, read.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// Publishers by name.
    pub publishers: BTreeMap<String, PublisherFile>,
    /// Plugin files by id.
    pub plugins: BTreeMap<String, PluginFile>,
    /// Starter ids.
    pub starter: Vec<String>,
    /// Reviewed confusable pairs (`id`, the id it resembles).
    pub allowed: BTreeSet<(String, String)>,
}

#[derive(Deserialize, Default)]
struct StarterFile {
    #[serde(default)]
    ids: Vec<String>,
}

#[derive(Deserialize, Default)]
struct AllowFile {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowEntry {
    id: String,
    like: String,
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path, errors: &mut Vec<String>) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            errors.push(format!("{}: {e}", path.display()));
            return None;
        }
    };
    match toml::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            errors.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    out.sort();
    out
}

/// Reads a registry directory; problems with its layout go into `errors`.
pub fn load(root: &Path, errors: &mut Vec<String>) -> Registry {
    let mut reg = Registry::default();
    for path in toml_files(&root.join("publishers")) {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if !valid_name(&name) {
            errors.push(format!(
                "publishers/{name}.toml: not a valid publisher name"
            ));
            continue;
        }
        if let Some(p) = read_toml::<PublisherFile>(&path, errors) {
            if PublicKey::parse(&p.key).is_none() {
                errors.push(format!(
                    "publishers/{name}.toml: `key` is not an ed25519 key id"
                ));
            }
            reg.publishers.insert(name, p);
        }
    }
    let plugins_dir = root.join("plugins");
    let mut publisher_dirs: Vec<PathBuf> = std::fs::read_dir(&plugins_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    publisher_dirs.sort();
    for pdir in publisher_dirs {
        let publisher = pdir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        for path in toml_files(&pdir) {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let at = format!("plugins/{publisher}/{name}.toml");
            let Some(f) = read_toml::<PluginFile>(&path, errors) else {
                continue;
            };
            if f.id != format!("{publisher}.{name}") || !valid_name(name) {
                errors.push(format!("{at}: id `{}` must be `{publisher}.{name}`", f.id));
                continue;
            }
            reg.plugins.insert(f.id.clone(), f);
        }
    }
    let starter = root.join("starter.toml");
    if starter.exists()
        && let Some(s) = read_toml::<StarterFile>(&starter, errors)
    {
        reg.starter = s.ids;
    }
    let allow = root.join("allow-confusable.toml");
    if allow.exists()
        && let Some(a) = read_toml::<AllowFile>(&allow, errors)
    {
        reg.allowed = a.allow.into_iter().map(|e| (e.id, e.like)).collect();
    }
    reg
}

/// Options of a check.
#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    /// The GitHub login that opened the pull request; it must be allowed to change every
    /// publisher's files it touches.
    pub author: Option<String>,
    /// Accept only publishers marked `early` (the official registry before 1.0).
    pub require_early: bool,
}

/// Every problem found; empty means the registry passes.
pub async fn check(
    root: &Path,
    base: Option<&Path>,
    options: &CheckOptions,
    fetcher: &Fetcher,
) -> Vec<String> {
    let mut errors = Vec::new();
    let reg = load(root, &mut errors);
    let mut base_errors = Vec::new();
    let old = base.map(|b| load(b, &mut base_errors)).unwrap_or_default();
    errors.extend(base_errors.into_iter().map(|e| format!("in the base: {e}")));

    // Publishers.
    for (name, p) in &reg.publishers {
        if name != "kumikit" && confusable::RESERVED_PUBLISHERS.contains(&name.as_str()) {
            errors.push(format!("publishers/{name}.toml: `{name}` is reserved"));
        }
        if options.require_early && !p.early {
            errors.push(format!(
                "publishers/{name}.toml: before 1.0 only early publishers are accepted (ADR 0020)"
            ));
        }
        if let Some(prev) = old.publishers.get(name) {
            if prev.key != p.key {
                errors.push(format!(
                    "publishers/{name}.toml: the publisher key changed; a key cannot be replaced"
                ));
            }
            if prev != p {
                author_may_change(options, name, prev, &mut errors);
            }
        } else {
            author_may_change(options, name, p, &mut errors);
        }
    }
    for name in old.publishers.keys() {
        if !reg.publishers.contains_key(name) {
            errors.push(format!("publishers/{name}.toml was removed"));
        }
    }

    // Plugins: immutable versions, authors, confusable new ids.
    let old_ids: Vec<String> = old.plugins.keys().cloned().collect();
    let mut known: Vec<String> = old_ids.clone();
    for (id, f) in &reg.plugins {
        let publisher = id.split('.').next().unwrap_or_default();
        let Some(pubfile) = reg.publishers.get(publisher) else {
            errors.push(format!("{id}: no publishers/{publisher}.toml"));
            continue;
        };
        let prev = old.plugins.get(id);
        if prev.is_none_or(|p| p != f) {
            let owner = old.publishers.get(publisher).unwrap_or(pubfile);
            author_may_change(options, publisher, owner, &mut errors);
        }
        if prev.is_none() {
            let allowed: Vec<String> = known
                .iter()
                .filter(|k| !reg.allowed.contains(&(id.clone(), (*k).clone())))
                .cloned()
                .collect();
            if let Some(why) = confusable::conflict(id, &allowed) {
                errors.push(format!(
                    "{id}: {why}; if this is intended, a maintainer adds it to allow-confusable.toml"
                ));
            }
            known.push(id.clone());
        }
        let mut seen = BTreeSet::new();
        for v in &f.versions {
            if !seen.insert(v.version.clone()) {
                errors.push(format!("{id}: version {} is listed twice", v.version));
            }
            if semver::Version::parse(&v.version).is_err() {
                errors.push(format!("{id}: `{}` is not a semver version", v.version));
            }
        }
        if let Some(p) = prev {
            for pv in &p.versions {
                match f.versions.iter().find(|v| v.version == pv.version) {
                    None => errors.push(format!(
                        "{id} {}: a published version cannot be removed (mark it yanked)",
                        pv.version
                    )),
                    Some(v) => {
                        let mut unyanked = v.clone();
                        unyanked.yanked = pv.yanked;
                        if unyanked != *pv || (pv.yanked && !v.yanked) {
                            errors.push(format!(
                                "{id} {}: a published version cannot change (only `yanked = true` may be added)",
                                pv.version
                            ));
                        }
                    }
                }
            }
        }
        // Download and check every version that is new.
        for v in &f.versions {
            if prev.is_some_and(|p| p.versions.iter().any(|o| o.version == v.version)) {
                continue;
            }
            if let Err(e) = check_version(id, f, v, pubfile, fetcher).await {
                errors.push(format!("{id} {}: {e}", v.version));
            }
        }
    }
    for id in &old_ids {
        if !reg.plugins.contains_key(id) {
            errors.push(format!("{id}: a published plugin cannot be removed"));
        }
    }
    for id in &reg.starter {
        if !reg.plugins.contains_key(id) {
            errors.push(format!("starter.toml: {id} is not in the registry"));
        }
    }
    errors
}

fn author_may_change(
    options: &CheckOptions,
    publisher: &str,
    owner: &PublisherFile,
    errors: &mut Vec<String>,
) {
    if let Some(author) = &options.author
        && !owner.github.iter().any(|g| g.eq_ignore_ascii_case(author))
    {
        let message = format!("{author} may not change the files of publisher `{publisher}`");
        if !errors.contains(&message) {
            errors.push(message);
        }
    }
}

async fn check_version(
    id: &str,
    file: &PluginFile,
    v: &VersionFile,
    publisher: &PublisherFile,
    fetcher: &Fetcher,
) -> Result<(), String> {
    let bytes = match fetcher
        .get(&v.url, None, kumikit_registry::client::MAX_PACKAGE)
        .await
        .map_err(|e| e.to_string())?
    {
        Fetched::Body { bytes, .. } => bytes,
        Fetched::NotModified => return Err("the server answered 'not modified'".into()),
    };
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hash != v.sha256 {
        return Err(format!("the package's SHA-256 is {hash}, not {}", v.sha256));
    }
    let facts = facts_of_package(&bytes)?;
    let key = PublicKey::parse(&publisher.key).ok_or("the publisher's key is not valid")?;
    match facts.key {
        None => return Err("the package is not signed".into()),
        Some(k) if k != key => {
            return Err(format!("signed by {k}, not by the publisher's key {key}"));
        }
        Some(_) => {}
    }
    if facts.id != id {
        return Err(format!("the manifest's id is {}", facts.id));
    }
    if facts.version != v.version {
        return Err(format!("the manifest's version is {}", facts.version));
    }
    if facts.kind != file.kind {
        return Err(format!("the manifest's kind is {:?}", facts.kind));
    }
    let entry = v
        .to_index(is_client_kind(&file.kind))
        .map_err(|e| format!("bad entry: {e}"))?;
    if entry.api != facts.api {
        return Err(format!(
            "`api` must be copied from the manifest ({})",
            facts.api
        ));
    }
    if entry.requested.map(normalized) != facts.requested.map(normalized) {
        return Err("`permissions` and `limits` must be copied from the manifest".into());
    }
    if is_client_kind(&file.kind) && entry.ui != facts.ui {
        return Err(
            "`isolation`, `network` and `requires-core` must be copied from the manifest".into(),
        );
    }
    Ok(())
}

/// Builds the index from a registry directory.
pub fn build_index(root: &Path, name: &str, generated: &str) -> Result<Index, Vec<String>> {
    let mut errors = Vec::new();
    let reg = load(root, &mut errors);
    let mut plugins = Vec::new();
    for f in reg.plugins.values() {
        let client = is_client_kind(&f.kind);
        let mut versions: Vec<Version> = Vec::new();
        for v in &f.versions {
            match v.to_index(client) {
                Ok(e) => versions.push(e),
                Err(e) => errors.push(format!("{} {}: {e}", f.id, v.version)),
            }
        }
        plugins.push(Plugin {
            id: f.id.clone(),
            name: f.name.clone(),
            description: f.description.clone(),
            repository: f.repository.clone(),
            license: f.license.clone(),
            kind: f.kind.clone(),
            versions,
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Index {
        format: FORMAT,
        name: name.to_owned(),
        generated: generated.to_owned(),
        starter: reg.starter,
        publishers: reg
            .publishers
            .into_iter()
            .map(|(n, p)| {
                (
                    n,
                    Publisher {
                        name: p.name,
                        key: p.key,
                    },
                )
            })
            .collect(),
        plugins,
    })
}
