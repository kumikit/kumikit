//! The structure of UI plugin manifests (`docs/spec/07-ui-plugins.md#manifest`),
//! checked where packages are installed or published: the desktop shell's client plugin
//! store, `kumikit plugin pack` and the registry checker. The UI kernel checks the
//! contents (slot names) when it loads a plugin.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The UI SDK version this Kumikit provides; manifests' `api` ranges must accept it.
pub const UI_SDK_VERSION: &str = "0.1.7";

/// Slots a sandboxed panel may contribute to (07-ui-plugins, Level 2).
pub const PANEL_SLOTS: &[&str] = &["sidebar.section", "inspector.tab", "view", "settings.page"];

/// The manifest file name.
pub const MANIFEST: &str = "plugin.toml";

// Fields are declared so unknown ones are refused; the UI kernel reads most of them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    kind: Vec<String>,
    api: String,
    #[serde(default)]
    ui: Option<UiSection>,
    #[serde(default)]
    requires: Option<toml::Value>,
    #[serde(default)]
    contributes: Option<Contributes>,
}

// Fields are declared so unknown ones are refused; the UI kernel reads most of them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiSection {
    entry: String,
    #[serde(default = "inline")]
    isolation: String,
    /// Hosts a sandboxed panel may contact (`https` only).
    #[serde(default)]
    network: Vec<String>,
}

fn inline() -> String {
    "inline".into()
}

// Fields are declared so unknown ones are refused; the UI kernel reads most of them.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contributes {
    #[serde(default)]
    slots: Vec<toml::Value>,
    #[serde(default)]
    commands: Vec<toml::Value>,
    #[serde(default)]
    settings: Option<String>,
}

pub fn valid_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

/// A path inside the package that stays inside it.
fn inside(root: &Path, rel: &str, what: &str, problems: &mut Vec<String>) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        problems.push(format!(
            "{what} `{rel}` must be a relative path inside the package"
        ));
        return None;
    }
    let full = root.join(p);
    if !full.is_file() {
        problems.push(format!("{what} `{rel}` is missing from the package"));
        return None;
    }
    Some(full)
}

/// Reads and checks the manifest in an unpacked plugin directory. Every problem is
/// reported at once, joined with `; `.
pub fn check(dir: &Path) -> Result<(String, String, serde_json::Value), String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|_| format!("the package has no {MANIFEST}"))?;
    let value: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{MANIFEST} is not valid TOML: {e}"))?;
    let m: Manifest = value
        .clone()
        .try_into()
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
    let mut problems = Vec::new();
    if !valid_id(&m.id) {
        problems.push(format!(
            "id `{}` must be <publisher>.<name> in lowercase letters, digits and dashes",
            m.id
        ));
    }
    if m.name.trim().is_empty() {
        problems.push("name must not be empty".into());
    }
    if semver::Version::parse(&m.version).is_err() {
        problems.push(format!("version `{}` is not a semver version", m.version));
    }
    match semver::VersionReq::parse(&m.api) {
        Ok(req) => {
            let sdk = semver::Version::parse(UI_SDK_VERSION).map_err(|e| e.to_string())?;
            if !req.matches(&sdk) {
                problems.push(format!(
                    "api `{}` does not accept the UI SDK {UI_SDK_VERSION} this app provides",
                    m.api
                ));
            }
        }
        Err(_) => problems.push(format!("api `{}` is not a version range", m.api)),
    }
    let is_ui = match m.kind.as_slice() {
        [k] if k == "ui" => true,
        [k] if k == "theme" => {
            problems.push(
                "themes are plain .json files, not plugins; add them in Settings -> Appearance"
                    .into(),
            );
            false
        }
        _ => {
            if m.kind.iter().any(|k| k != "ui") {
                problems
                    .push("this is a core plugin; install it on a core from Manage plugins".into());
            } else {
                problems.push("kind must be [\"ui\"]".into());
            }
            false
        }
    };
    if is_ui {
        match &m.ui {
            None => problems.push("a UI plugin needs a [ui] section with an entry".into()),
            Some(ui) => {
                inside(dir, &ui.entry, "the entry", &mut problems);
                match ui.isolation.as_str() {
                    "inline" => {
                        if !ui.network.is_empty() {
                            problems.push(
                                "[ui] network is only for sandboxed panels (isolation = \"iframe\")"
                                    .into(),
                            );
                        }
                    }
                    "iframe" => {
                        if !ui.entry.ends_with(".html") {
                            problems.push("a sandboxed panel's entry must be an .html file".into());
                        }
                        for host in &ui.network {
                            if !crate::valid_network_host(host) {
                                problems.push(format!(
                                    "[ui] network: `{host}` is not a host name (write it like api.example.com)"
                                ));
                            }
                        }
                    }
                    other => problems.push(format!("isolation `{other}` is not known")),
                }
            }
        }
    }
    let iframe = m.ui.as_ref().is_some_and(|u| u.isolation == "iframe");
    if iframe && let Some(c) = &m.contributes {
        for entry in &c.slots {
            let slot = entry
                .get("slot")
                .and_then(toml::Value::as_str)
                .unwrap_or_default();
            if !PANEL_SLOTS.contains(&slot) {
                problems.push(format!(
                    "a sandboxed panel cannot contribute to `{slot}`; it can use {}",
                    PANEL_SLOTS.join(", ")
                ));
            }
        }
    }
    if let Some(c) = &m.contributes
        && let Some(s) = &c.settings
    {
        inside(dir, s, "the settings schema", &mut problems);
    }
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    let json = serde_json::to_value(&value).map_err(|e| e.to_string())?;
    Ok((m.id, m.version, json))
}

/// For a sandboxed panel (`isolation = "iframe"`), the hosts it may contact; `None` for
/// any other plugin, or a manifest that cannot be read.
pub fn panel_network(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let manifest: Manifest = toml::from_str(&text).ok()?;
    let ui = manifest.ui?;
    (ui.isolation == "iframe").then_some(ui.network)
}
