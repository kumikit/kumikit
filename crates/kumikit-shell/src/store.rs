//! The client plugin store (ADR 0013; `docs/spec/07-ui-plugins.md#manifest`): UI plugins
//! and themes installed into the client data directory, next to the read-only ones
//! bundled with the app. The shell checks a manifest's structure and that its files
//! exist; the UI kernel checks the contents when it loads them.

use std::path::{Path, PathBuf};

use serde::Serialize;

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
    /// Reading or writing files failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
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
            Ok((id, version, manifest)) if seen.insert(id.clone()) => out.push(Entry {
                id,
                version,
                source,
                manifest,
            }),
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

    /// Unpacks and checks a package, then moves it into place. A bundled id cannot be
    /// replaced, and an installed id must be removed first (no updates before M4).
    pub fn install(&self, package: &Path) -> Result<Entry, StoreError> {
        std::fs::create_dir_all(&self.installed)?;
        let staging = tempfile::tempdir_in(&self.installed)?;
        kumikit_package::unpack(package, staging.path())
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let (id, version, manifest) = check(staging.path())?;
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
        std::fs::rename(staging.keep(), target.join(&version))?;
        Ok(Entry {
            id,
            version,
            source: "installed",
            manifest,
        })
    }

    /// Removes an installed plugin (bundled ones can only be disabled).
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
                "does not accept the UI SDK 0.1.0",
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
