//! The client plugin store (ADR 0013; `docs/spec/07-ui-plugins.md#manifest`): UI plugins
//! and themes installed into the client data directory, next to the read-only ones
//! bundled with the app. The shell checks a manifest's structure and that its files
//! exist; the UI kernel checks the contents when it loads them.

use std::path::{Path, PathBuf};

use kumikit_registry::index::UiNeeds;
use serde::{Deserialize, Serialize};

pub use kumikit_assets::ui_manifest::{MANIFEST, UI_SDK_VERSION, valid_id};

/// Reads and checks the manifest in an unpacked plugin directory (the shared check in
/// `kumikit_assets::ui_manifest`).
pub fn check(dir: &Path) -> Result<(String, String, serde_json::Value), StoreError> {
    kumikit_assets::ui_manifest::check(dir).map_err(StoreError::Invalid)
}

/// A store error, in plain words for the user.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The package or its manifest is not acceptable.
    #[error("{0}")]
    Invalid(String),
    /// Something with this id is already installed or bundled.
    #[error("{0}")]
    Conflict(String),
    /// No installed plugin has this id.
    #[error("{0}")]
    NotFound(String),
    /// The package failed verification: its signature, a pinned key, or (from a
    /// registry) its manifest against the entry.
    #[error("{0}")]
    Untrusted(String),
    /// Reading or writing files failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Who signed an installed plugin, and the registry it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Publisher {
    /// The signing key's id.
    pub key: String,
    /// The registry's index URL, for a registry install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

/// What the store remembers about an installed plugin (`<installed>/<id>/install.toml`):
/// the installed version, its package's hash, where it came from and who signed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// Installed version.
    pub version: String,
    /// SHA-256 of the package.
    pub sha256: String,
    /// `file` or `registry+<index URL>`.
    pub source: String,
    /// The signing key's id, pinned for later versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// What a registry's entry says a package is (10-distribution, Install).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// The publisher's key, which must have signed it.
    pub key: String,
    /// `id`.
    pub id: String,
    /// `version`.
    pub version: String,
    /// `api`.
    pub api: String,
    /// Its isolation, network hosts and core needs, as the entry copies them.
    pub ui: UiNeeds,
}

/// A UI plugin's needs as its manifest (JSON) says.
pub fn ui_needs(manifest: &serde_json::Value) -> UiNeeds {
    let strings = |v: &serde_json::Value| -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    };
    let ui = &manifest["ui"];
    UiNeeds {
        isolation: (!ui.is_null()).then(|| ui["isolation"].as_str().unwrap_or("inline").to_owned()),
        network: strings(&ui["network"]),
        requires_core: strings(&manifest["requires"]["core"]),
    }
}

/// Whether `new` needs more than `old` (10-distribution, Updates): a new network host, or
/// inline code where there was none.
pub fn widens(old: &UiNeeds, new: &UiNeeds) -> bool {
    new.network.iter().any(|h| !old.network.contains(h))
        || (new.isolation.as_deref() == Some("inline")
            && old.isolation.as_deref() != Some("inline"))
}

/// What the UI kernel gets for each plugin.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub version: String,
    /// `bundled` (read-only) or `installed`.
    pub source: &'static str,
    /// The manifest as JSON.
    pub manifest: serde_json::Value,
    /// Who signed it, if it was signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Publisher>,
}

struct Staged {
    sha256: String,
    staging: tempfile::TempDir,
    id: String,
    version: String,
    manifest: serde_json::Value,
    key: Option<String>,
}

fn publisher(meta: &Meta) -> Option<Publisher> {
    Some(Publisher {
        key: meta.key.clone()?,
        registry: meta.source.strip_prefix("registry+").map(str::to_owned),
    })
}

/// Installed and bundled UI plugins.
#[derive(Debug, Clone)]
pub struct Store {
    /// Read-only plugins shipped with the app: `<bundled>/<id>/`.
    pub bundled: Option<PathBuf>,
    /// The user's plugins: `<installed>/<id>/<version>/`.
    pub installed: PathBuf,
}

impl Store {
    /// The directory a plugin's files are served from: an installed version wins over a
    /// bundled one.
    pub fn dir(&self, id: &str) -> Option<PathBuf> {
        if !valid_id(id) {
            return None;
        }
        self.installed_version(id)
            .map(|v| self.installed.join(id).join(v))
            .or_else(|| {
                self.bundled
                    .as_ref()
                    .map(|b| b.join(id))
                    .filter(|d| d.is_dir())
            })
    }

    /// For a sandboxed panel plugin, the hosts it may contact; `None` for any other plugin
    /// (and for an unknown or broken one).
    pub fn panel_network(&self, id: &str) -> Option<Vec<String>> {
        kumikit_assets::ui_manifest::panel_network(&self.dir(id)?)
    }

    fn installed_version(&self, id: &str) -> Option<String> {
        let dir = self.installed.join(id);
        std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .filter(|e| e.path().join(MANIFEST).is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .max_by(
                |a, b| match (semver::Version::parse(a), semver::Version::parse(b)) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => a.cmp(b),
                },
            )
    }

    /// Every plugin, bundled ones first; a broken one is skipped and logged.
    pub fn list(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut push = |dir: PathBuf, source: &'static str| match check(&dir) {
            Ok((id, version, manifest)) if seen.insert(id.clone()) => {
                let publisher = (source == "installed")
                    .then(|| self.meta(&id))
                    .flatten()
                    .and_then(|m| publisher(&m));
                out.push(Entry {
                    id,
                    version,
                    source,
                    manifest,
                    publisher,
                })
            }
            Ok(_) => {}
            Err(e) => {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("kumikit: skipping UI plugin in {}: {e}", dir.display());
                }
            }
        };
        let mut ids: Vec<String> = std::fs::read_dir(&self.installed)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        let installed: Vec<PathBuf> = ids
            .iter()
            .filter_map(|id| {
                self.installed_version(id)
                    .map(|v| self.installed.join(id).join(v))
            })
            .collect();
        if let Some(b) = &self.bundled {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(b)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join(MANIFEST).is_file())
                .collect();
            dirs.sort();
            for d in dirs {
                // An installed copy of a bundled id replaces it.
                let id = d
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_owned();
                if !ids.contains(&id) {
                    push(d, "bundled");
                }
            }
        }
        for d in installed {
            push(d, "installed");
        }
        out
    }

    /// What the store remembers about an installed plugin.
    pub fn meta(&self, id: &str) -> Option<Meta> {
        if !valid_id(id) {
            return None;
        }
        let text = std::fs::read_to_string(self.installed.join(id).join("install.toml")).ok()?;
        toml::from_str(&text).ok()
    }

    fn write_meta(&self, id: &str, meta: &Meta) -> Result<(), StoreError> {
        let text = toml::to_string(meta).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let path = self.installed.join(id).join("install.toml");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Unpacks into staging, verifies the signature, checks the manifest and, with
    /// `expected`, that it is what the registry's entry says.
    fn stage(&self, package: &Path, expected: Option<&Expected>) -> Result<Staged, StoreError> {
        std::fs::create_dir_all(&self.installed)?;
        let sha256 = kumikit_package::sha256_file(package)?;
        let staging = tempfile::tempdir_in(&self.installed)?;
        kumikit_package::unpack(package, staging.path())
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let key = kumikit_package::signature::verify_dir(staging.path())
            .map_err(|e| match e {
                kumikit_package::PackageError::Untrusted(m) => {
                    StoreError::Untrusted(format!("untrusted package: {m}"))
                }
                other => StoreError::Invalid(other.to_string()),
            })?
            .key()
            .map(|k| k.to_string());
        let (id, version, manifest) = check(staging.path())?;
        if let Some(e) = expected {
            let untrusted = |why: String| {
                StoreError::Untrusted(format!("untrusted package: {} {}: {why}", e.id, e.version))
            };
            match &key {
                None => return Err(untrusted("it is not signed".into())),
                Some(k) if *k != e.key => {
                    return Err(untrusted(format!(
                        "it is signed by {k}, not by the publisher's key {}",
                        e.key
                    )));
                }
                Some(_) => {}
            }
            let api = manifest["api"].as_str().unwrap_or_default();
            if id != e.id || version != e.version || api != e.api {
                return Err(untrusted(format!(
                    "its manifest says {id} {version} (api {api}), which differs from the registry's entry"
                )));
            }
            if ui_needs(&manifest) != e.ui {
                return Err(untrusted(
                    "its isolation, network hosts or core needs differ from the registry's entry"
                        .into(),
                ));
            }
        }
        Ok(Staged {
            sha256,
            staging,
            id,
            version,
            manifest,
            key,
        })
    }

    /// Installs a UI plugin or theme package from a file.
    pub fn install(&self, package: &Path) -> Result<Entry, StoreError> {
        self.install_checked(package, "file", None)
    }

    /// Installs a package; `source` is `file` or `registry+<url>`, and `expected` (from a
    /// registry) is checked before anything is put in place.
    pub fn install_checked(
        &self,
        package: &Path,
        source: &str,
        expected: Option<&Expected>,
    ) -> Result<Entry, StoreError> {
        let staged = self.stage(package, expected)?;
        let id = staged.id.clone();
        if self.bundled.as_ref().is_some_and(|b| b.join(&id).is_dir()) {
            return Err(StoreError::Conflict(format!(
                "{id} is bundled with the app; disable it instead"
            )));
        }
        let target = self.installed.join(&id);
        if target.exists() {
            return Err(StoreError::Conflict(format!(
                "{id} is already installed; remove it first"
            )));
        }
        std::fs::create_dir_all(&target)?;
        std::fs::rename(staged.staging.keep(), target.join(&staged.version))?;
        let meta = Meta {
            version: staged.version.clone(),
            sha256: staged.sha256,
            source: source.to_owned(),
            key: staged.key,
        };
        self.write_meta(&id, &meta)?;
        Ok(Entry {
            publisher: publisher(&meta),
            id,
            version: staged.version,
            source: "installed",
            manifest: staged.manifest,
        })
    }

    /// Replaces an installed plugin with another version (an update). A package signed by
    /// another key than the installed one's, or unsigned where it was signed, is untrusted.
    pub fn replace(
        &self,
        id: &str,
        package: &Path,
        source: &str,
        expected: Option<&Expected>,
    ) -> Result<Entry, StoreError> {
        let current = self
            .meta(id)
            .ok_or_else(|| StoreError::NotFound(format!("{id} is not installed")))?;
        let staged = self.stage(package, expected)?;
        if staged.id != id {
            return Err(StoreError::Invalid(format!(
                "the package is {}, not {id}",
                staged.id
            )));
        }
        if staged.version == current.version {
            return Err(StoreError::Conflict(format!(
                "{id} {} is already installed",
                current.version
            )));
        }
        if let Some(pinned) = &current.key
            && staged.key.as_ref() != Some(pinned)
        {
            return Err(StoreError::Untrusted(format!(
                "untrusted package: {id} {} is not signed by the key of the installed version ({pinned}); if you trust the new one, remove the plugin and install it again",
                staged.version
            )));
        }
        let dir = self.installed.join(id);
        std::fs::rename(staged.staging.keep(), dir.join(&staged.version))?;
        let meta = Meta {
            version: staged.version.clone(),
            sha256: staged.sha256,
            source: source.to_owned(),
            key: staged.key,
        };
        self.write_meta(id, &meta)?;
        let old = dir.join(&current.version);
        if old.is_dir() {
            std::fs::remove_dir_all(old)?;
        }
        Ok(Entry {
            publisher: publisher(&meta),
            id: id.to_owned(),
            version: staged.version,
            source: "installed",
            manifest: staged.manifest,
        })
    }

    /// The client lockfile's text: installed (not bundled) UI plugins and themes.
    pub fn lock_text(&self) -> String {
        let mut ids: Vec<String> = std::fs::read_dir(&self.installed)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        let plugins = ids
            .into_iter()
            .filter_map(|id| {
                let m = self.meta(&id)?;
                Some(kumikit_protocol::lockfile::LockEntry {
                    id,
                    version: m.version,
                    sha256: m.sha256,
                    source: m.source,
                    key: m.key,
                    state: None,
                    granted: None,
                    enabled: None,
                    trusted: None,
                })
            })
            .collect();
        kumikit_protocol::lockfile::render(&kumikit_protocol::lockfile::LockFile {
            format: kumikit_protocol::lockfile::LOCK_FORMAT,
            plugins,
        })
    }

    pub fn remove(&self, id: &str) -> Result<(), StoreError> {
        let target = self.installed.join(id);
        if !valid_id(id) || !target.is_dir() {
            return Err(StoreError::NotFound(format!("{id} is not installed")));
        }
        std::fs::remove_dir_all(target)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r#"
id = "community.night"
name = "Night"
version = "1.0.0"
kind = ["theme"]
api = "^0.1"
[theme]
variants = [{ id = "dark", appearance = "dark", tokens = "dark.json" }]
"#;

    const UI: &str = r#"
id = "community.panel"
name = "Panel"
version = "0.2.0"
kind = ["ui"]
api = "^0.1"
[ui]
entry = "dist/index.js"
[contributes]
slots = [{ slot = "inspector.tab", id = "p", title = "P" }]
"#;

    fn package(root: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) -> PathBuf {
        let src = root.join(format!("{name}-src"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join(MANIFEST), manifest).unwrap();
        for (p, text) in files {
            let f = src.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let out = root.join(format!("{name}.kkp"));
        kumikit_package::pack(&src, &out).unwrap();
        out
    }

    fn store(root: &Path) -> Store {
        let bundled = root.join("bundled");
        std::fs::create_dir_all(bundled.join("kumikit.base")).unwrap();
        std::fs::write(
            bundled.join("kumikit.base").join(MANIFEST),
            THEME.replace("community.night", "kumikit.base"),
        )
        .unwrap();
        std::fs::write(bundled.join("kumikit.base").join("dark.json"), "{}").unwrap();
        Store {
            bundled: Some(bundled),
            installed: root.join("client/ui-plugins"),
        }
    }

    #[test]
    fn installs_lists_serves_and_removes() {
        let root = tempfile::tempdir().unwrap();
        let s = store(root.path());
        let theme = package(
            root.path(),
            "night",
            THEME,
            &[("dark.json", r##"{"accent":"#f0f"}"##)],
        );
        let ui = package(
            root.path(),
            "panel",
            UI,
            &[("dist/index.js", "export default {}")],
        );
        assert_eq!(s.install(&theme).unwrap().id, "community.night");
        assert_eq!(s.install(&ui).unwrap().version, "0.2.0");
        let list: Vec<(String, &str)> = s.list().into_iter().map(|e| (e.id, e.source)).collect();
        assert_eq!(
            list,
            [
                ("kumikit.base".to_owned(), "bundled"),
                ("community.night".to_owned(), "installed"),
                ("community.panel".to_owned(), "installed"),
            ]
        );
        assert!(
            s.dir("community.panel")
                .unwrap()
                .ends_with("community.panel/0.2.0")
        );
        assert!(
            s.dir("kumikit.base")
                .unwrap()
                .ends_with("bundled/kumikit.base")
        );
        assert!(s.dir("../etc").is_none());
        assert!(matches!(s.install(&ui), Err(StoreError::Conflict(_))));
        s.remove("community.panel").unwrap();
        assert!(s.dir("community.panel").is_none());
        assert!(matches!(
            s.remove("kumikit.base"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn installs_a_sandboxed_panel_and_reports_its_hosts() {
        let root = tempfile::tempdir().unwrap();
        let s = store(root.path());
        let manifest = UI.replace(
            "entry = \"dist/index.js\"",
            "entry = \"dist/index.html\"\nisolation = \"iframe\"\nnetwork = [\"api.example.com\"]",
        );
        let p = package(
            root.path(),
            "panel",
            &manifest,
            &[("dist/index.html", "<!doctype html>")],
        );
        s.install(&p).unwrap();
        assert_eq!(
            s.panel_network("community.panel"),
            Some(vec!["api.example.com".to_owned()])
        );
        assert_eq!(s.panel_network("kumikit.base"), None, "a theme is no panel");
        assert_eq!(s.panel_network("nobody.here"), None);
    }

    #[test]
    fn refuses_what_it_cannot_load() {
        let root = tempfile::tempdir().unwrap();
        let s = store(root.path());
        let cases = [
            (
                "core",
                "id = \"a.b\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = [\"provider\"]\napi = \"^0.1\"\n",
                "install it on a core",
            ),
            (
                "iframe-js",
                &UI.replace(
                    "entry = \"dist/index.js\"",
                    "entry = \"dist/index.js\"\nisolation = \"iframe\"",
                ),
                "must be an .html file",
            ),
            (
                "iframe-host",
                &UI.replace(
                    "entry = \"dist/index.js\"",
                    "entry = \"dist/index.js\"\nisolation = \"iframe\"\nnetwork = [\"https://x.example\"]",
                ),
                "is not a host name",
            ),
            (
                "panel-slot",
                &UI.replace(
                    "entry = \"dist/index.js\"",
                    "entry = \"dist/index.html\"\nisolation = \"iframe\"",
                )
                .replace("inspector.tab", "transcript.renderer"),
                "cannot contribute to `transcript.renderer`",
            ),
            (
                "inline-network",
                &UI.replace(
                    "entry = \"dist/index.js\"",
                    "entry = \"dist/index.js\"\nnetwork = [\"x.example\"]",
                ),
                "only for sandboxed panels",
            ),
            ("missing", UI, "the entry `dist/index.js` is missing"),
            (
                "api",
                &UI.replace("^0.1", "^1.0"),
                "does not accept the UI SDK 0.1.2",
            ),
            (
                "bundled",
                &THEME.replace("community.night", "kumikit.base"),
                "bundled with the app",
            ),
            (
                "font",
                &THEME.replace("[theme]", "[theme]\nfonts = [\"a.ttf\"]"),
                "must be a .woff2 file",
            ),
        ];
        for (name, manifest, expected) in cases {
            let p = package(root.path(), name, manifest, &[("dark.json", "{}")]);
            let err = s.install(&p).unwrap_err().to_string();
            assert!(err.contains(expected), "{name}: {err}");
        }
        assert!(
            s.list().iter().all(|e| e.source == "bundled"),
            "nothing half-installed"
        );
    }
}
