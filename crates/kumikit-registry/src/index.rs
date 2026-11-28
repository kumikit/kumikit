//! The registry's files and the `index.json` clients read
//! (`docs/spec/10-distribution.md#registry`).
//!
//! A registry repository holds `publishers/<publisher>.toml` and
//! `plugins/<publisher>/<name>.toml` ([`PublisherFile`], [`PluginFile`]); its CI builds
//! one [`Index`] from them. Clients read only the index.

use std::collections::BTreeMap;

use kumikit_protocol::methods::{FilesAccess, Limits, Permissions, WorkspacesAccess};
use serde::{Deserialize, Serialize};

/// The index format this code reads and writes.
pub const FORMAT: u32 = 1;

/// `index.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    /// Always [`FORMAT`]; another value is refused.
    pub format: u32,
    /// The registry's name, shown next to search results.
    pub name: String,
    /// When the index was built (RFC 3339).
    pub generated: String,
    /// Ids offered on the first-run screen.
    #[serde(default)]
    pub starter: Vec<String>,
    /// Publishers by name.
    pub publishers: BTreeMap<String, Publisher>,
    /// Every plugin.
    pub plugins: Vec<Plugin>,
}

/// A publisher in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publisher {
    /// Display name.
    pub name: String,
    /// The key that signs every one of its packages (`ed25519:…`).
    pub key: String,
}

/// A plugin in the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// `<publisher>.<name>`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Source repository URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// SPDX licence expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// The manifest's `kind`.
    pub kind: Vec<String>,
    /// Published versions, in any order.
    pub versions: Vec<Version>,
}

impl Plugin {
    /// The publisher part of the id.
    pub fn publisher(&self) -> &str {
        self.id.split('.').next().unwrap_or_default()
    }

    /// Whether it is a UI plugin (installed on a client, not a core).
    pub fn is_client_side(&self) -> bool {
        self.kind.len() == 1 && self.kind[0] == "ui"
    }
}

/// One published version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// Semver.
    pub version: String,
    /// Where the `.kkp` is.
    pub url: String,
    /// SHA-256 of the `.kkp`, lowercase hex.
    pub sha256: String,
    /// The manifest's `api` range.
    pub api: String,
    /// A core plugin's requested permissions, as the manifest says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<Permissions>,
    /// A UI plugin's isolation and needs, as the manifest says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiNeeds>,
    /// Withdrawn: never offered as an install or update.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub yanked: bool,
}

/// What a UI plugin asks for (07-ui-plugins): how its code runs and what it reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiNeeds {
    /// `inline` or `iframe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    /// Hosts a panel may contact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    /// Core plugins it needs (`<id>@<range>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_core: Vec<String>,
}

/// `publishers/<publisher>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherFile {
    /// Display name.
    pub name: String,
    /// `ed25519:…`.
    pub key: String,
    /// GitHub logins that may change this publisher's files.
    #[serde(default)]
    pub github: Vec<String>,
    /// Accepted before the interfaces freeze at 1.0 (ADR 0020).
    #[serde(default)]
    pub early: bool,
}

/// `plugins/<publisher>/<name>.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginFile {
    /// `<publisher>.<name>`, matching the file's path.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line.
    #[serde(default)]
    pub description: Option<String>,
    /// Source repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// SPDX licence expression.
    #[serde(default)]
    pub license: Option<String>,
    /// The manifest's `kind`.
    pub kind: Vec<String>,
    /// Published versions.
    #[serde(default)]
    pub versions: Vec<VersionFile>,
}

/// A `[[versions]]` entry of a plugin file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct VersionFile {
    /// Semver.
    pub version: String,
    /// Where the `.kkp` is.
    pub url: String,
    /// SHA-256 of the `.kkp`.
    pub sha256: String,
    /// Copied from the manifest.
    pub api: String,
    /// Copied from the manifest's `[permissions]` (core plugins).
    #[serde(default)]
    pub permissions: Option<PermissionsFile>,
    /// Copied from the manifest's `[limits]` (core plugins).
    #[serde(default)]
    pub limits: Option<LimitsFile>,
    /// Copied from the manifest's `[ui]` isolation (UI plugins).
    #[serde(default)]
    pub isolation: Option<String>,
    /// Copied from the manifest's `[ui] network` (UI plugins).
    #[serde(default)]
    pub network: Vec<String>,
    /// Copied from the manifest's `[requires] core` (UI plugins).
    #[serde(default)]
    pub requires_core: Vec<String>,
    /// Withdrawn.
    #[serde(default)]
    pub yanked: bool,
}

/// The `permissions` table of a version, in the manifest's own words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsFile {
    /// Programs.
    #[serde(default)]
    pub spawn: Vec<String>,
    /// Hosts.
    #[serde(default)]
    pub network: Vec<String>,
    /// `none`, `read-workspace` or `workspace`.
    #[serde(default)]
    pub files: Option<String>,
    /// Secret names.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Other plugins' tools.
    #[serde(default)]
    pub tools: bool,
    /// Client operations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client: Vec<String>,
    /// `none`, `linked` or `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<String>,
    /// Runs on a schedule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub schedules: bool,
}

/// Reads a `workspaces` value in the manifest's words.
pub fn workspaces_access(value: Option<&str>) -> Result<WorkspacesAccess, String> {
    match value.unwrap_or("none") {
        "none" => Ok(WorkspacesAccess::None),
        "linked" => Ok(WorkspacesAccess::Linked),
        "all" => Ok(WorkspacesAccess::All),
        other => Err(format!("workspaces = `{other}` is not none, linked or all")),
    }
}

/// A `workspaces` value in the manifest's words.
pub fn workspaces_word(w: WorkspacesAccess) -> &'static str {
    match w {
        WorkspacesAccess::None => "none",
        WorkspacesAccess::Linked => "linked",
        WorkspacesAccess::All => "all",
    }
}

/// The `limits` table of a version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LimitsFile {
    /// MiB.
    #[serde(default)]
    pub memory_mb: Option<u32>,
    /// Seconds.
    #[serde(default)]
    pub call_timeout_s: Option<u32>,
}

/// Reads a `files` value in the manifest's words.
pub fn files_access(value: Option<&str>) -> Result<FilesAccess, String> {
    match value.unwrap_or("none") {
        "none" => Ok(FilesAccess::None),
        "read-workspace" => Ok(FilesAccess::ReadWorkspace),
        "workspace" => Ok(FilesAccess::Workspace),
        other => Err(format!(
            "files = `{other}` is not none, read-workspace or workspace"
        )),
    }
}

impl VersionFile {
    /// The index entry for this version of a plugin of `kind`.
    pub fn to_index(&self, client_side: bool) -> Result<Version, String> {
        let requested = if client_side {
            None
        } else {
            let p = self.permissions.clone().unwrap_or_default();
            let limits = self.limits.and_then(|l| {
                (l.memory_mb.is_some() || l.call_timeout_s.is_some()).then_some(Limits {
                    memory_mb: l.memory_mb,
                    call_timeout_s: l.call_timeout_s,
                })
            });
            Some(normalized(Permissions {
                spawn: p.spawn,
                network: p.network,
                files: files_access(p.files.as_deref())?,
                secrets: p.secrets,
                tools: p.tools,
                client: p.client,
                workspaces: workspaces_access(p.workspaces.as_deref())?,
                schedules: p.schedules,
                limits,
            }))
        };
        let ui = client_side.then(|| UiNeeds {
            isolation: self.isolation.clone(),
            network: sorted(self.network.clone()),
            requires_core: sorted(self.requires_core.clone()),
        });
        Ok(Version {
            version: self.version.clone(),
            url: self.url.clone(),
            sha256: self.sha256.clone(),
            api: self.api.clone(),
            requested,
            ui,
            yanked: self.yanked,
        })
    }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

/// Lists sorted and deduplicated, so two copies of the same permissions compare equal.
pub fn normalized(mut p: Permissions) -> Permissions {
    p.spawn = sorted(p.spawn);
    p.network = sorted(p.network);
    p.secrets = sorted(p.secrets);
    p
}

/// Reads an index, refusing other formats.
pub fn parse_index(bytes: &[u8]) -> Result<Index, String> {
    #[derive(Deserialize)]
    struct Head {
        format: u32,
    }
    let head: Head =
        serde_json::from_slice(bytes).map_err(|e| format!("the index is not valid JSON: {e}"))?;
    if head.format != FORMAT {
        return Err(format!(
            "the index has format {}; this Kumikit reads format {FORMAT}",
            head.format
        ));
    }
    serde_json::from_slice(bytes).map_err(|e| format!("the index is malformed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLUGIN: &str = r#"
id = "acme.probe"
name = "Probe"
kind = ["provider"]

[[versions]]
version = "0.1.0"
url = "https://example.com/acme.probe-0.1.0.kkp"
sha256 = "00"
api = "^0.1"
permissions = { spawn = ["b", "a", "a"], files = "read-workspace" }
limits = { memory-mb = 128 }
"#;

    #[test]
    fn a_plugin_file_becomes_an_index_entry() {
        let f: PluginFile = toml::from_str(PLUGIN).unwrap();
        let v = f.versions[0].to_index(false).unwrap();
        let p = v.requested.unwrap();
        assert_eq!(p.spawn, ["a", "b"]);
        assert_eq!(p.files, FilesAccess::ReadWorkspace);
        assert_eq!(p.limits.unwrap().memory_mb, Some(128));
        assert!(v.ui.is_none());
        let bad = PLUGIN.replace("read-workspace", "everything");
        let f: PluginFile = toml::from_str(&bad).unwrap();
        assert!(f.versions[0].to_index(false).is_err());
        assert!(toml::from_str::<PluginFile>(&PLUGIN.replace("kind", "kinds")).is_err());
    }

    #[test]
    fn index_format_is_checked() {
        let index = Index {
            format: FORMAT,
            name: "test".into(),
            generated: "2026-11-18T00:00:00Z".into(),
            starter: vec![],
            publishers: BTreeMap::new(),
            plugins: vec![],
        };
        let json = serde_json::to_vec(&index).unwrap();
        assert_eq!(parse_index(&json).unwrap(), index);
        let other = String::from_utf8(json)
            .unwrap()
            .replace("\"format\":1", "\"format\":2");
        assert!(
            parse_index(other.as_bytes())
                .unwrap_err()
                .contains("format 2")
        );
        assert!(parse_index(b"<html>").is_err());
    }
}
