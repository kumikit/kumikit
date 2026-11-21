//! `plugins.lock` (`docs/spec/10-distribution.md#lockfiles`): what a core or a client has
//! installed, so a setup can be reproduced elsewhere. Pure parsing and rendering; the core
//! and the shell write the files.

use serde::{Deserialize, Serialize};

use crate::methods::Permissions;

/// The lockfile format this code reads and writes.
pub const LOCK_FORMAT: u32 = 1;

const HEADER: &str =
    "# Written by Kumikit. Export it to reproduce these plugins on another core.\n";

/// A lockfile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockFile {
    /// Always [`LOCK_FORMAT`].
    pub format: u32,
    /// Installed plugins, by id.
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<LockEntry>,
}

/// One installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockEntry {
    /// Plugin id.
    pub id: String,
    /// Installed version.
    pub version: String,
    /// SHA-256 of the package.
    pub sha256: String,
    /// `registry+<index URL>`, `file` or `bundled`.
    pub source: String,
    /// The signing key's id, when signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// A core plugin's state: `installed`, `enabled` or `disabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// A core plugin's granted permissions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<Permissions>,
    /// A UI plugin's enabled flag (client lockfiles, on export).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Whether a UI plugin with inline code was trusted (client lockfiles, on export).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted: Option<bool>,
}

impl LockEntry {
    /// The registry's index URL, for an entry installed from one.
    pub fn registry(&self) -> Option<&str> {
        self.source.strip_prefix("registry+")
    }
}

/// The file's text, with a header comment; entries sorted by id.
pub fn render(lock: &LockFile) -> String {
    let mut lock = lock.clone();
    lock.format = LOCK_FORMAT;
    lock.plugins.sort_by(|a, b| a.id.cmp(&b.id));
    // Serializing plain data to TOML cannot fail; an empty string would be refused by `parse`.
    let body = toml::to_string(&lock).unwrap_or_default();
    format!("{HEADER}{body}")
}

/// Reads a lockfile, refusing other formats.
pub fn parse(text: &str) -> Result<LockFile, String> {
    let lock: LockFile =
        toml::from_str(text).map_err(|e| format!("not a Kumikit lockfile: {e}"))?;
    if lock.format != LOCK_FORMAT {
        return Err(format!(
            "the lockfile has format {}; this Kumikit reads format {LOCK_FORMAT}",
            lock.format
        ));
    }
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::FilesAccess;

    #[test]
    fn round_trip() {
        let lock = LockFile {
            format: LOCK_FORMAT,
            plugins: vec![
                LockEntry {
                    id: "b.two".into(),
                    version: "1.0.0".into(),
                    sha256: "ab".into(),
                    source: "file".into(),
                    key: None,
                    state: Some("installed".into()),
                    granted: None,
                    enabled: None,
                    trusted: None,
                },
                LockEntry {
                    id: "a.one".into(),
                    version: "0.1.0".into(),
                    sha256: "cd".into(),
                    source: "registry+https://r.example/index.json".into(),
                    key: Some("ed25519:x".into()),
                    state: Some("enabled".into()),
                    granted: Some(Permissions {
                        spawn: vec!["agent".into()],
                        files: FilesAccess::ReadWorkspace,
                        client: vec!["threads.send".into()],
                        workspaces: crate::methods::WorkspacesAccess::Linked,
                        schedules: true,
                        ..Permissions::default()
                    }),
                    enabled: None,
                    trusted: None,
                },
            ],
        };
        let text = render(&lock);
        assert!(text.starts_with("# Written by Kumikit"));
        assert!(
            text.find("a.one").unwrap() < text.find("b.two").unwrap(),
            "{text}"
        );
        assert!(
            text.contains("tools = false\nclient = [\"threads.send\"]\nworkspaces = \"linked\"\nschedules = true\n"),
            "web/kernel/tests/registry.test.ts copies this text:\n{text}"
        );
        let back = parse(&text).unwrap();
        assert_eq!(back.plugins[0].id, "a.one");
        assert_eq!(
            back.plugins[0].registry(),
            Some("https://r.example/index.json")
        );
        assert_eq!(
            back.plugins[0].granted.as_ref().unwrap().files,
            FilesAccess::ReadWorkspace
        );
        let g = back.plugins[0].granted.as_ref().unwrap();
        assert_eq!(g.client, ["threads.send"]);
        assert_eq!(g.workspaces, crate::methods::WorkspacesAccess::Linked);
        assert!(g.schedules);
        assert_eq!(back.plugins[1].registry(), None);
        assert!(parse("format = 2\n").unwrap_err().contains("format 2"));
        assert!(parse("not toml [").is_err());
        assert!(parse("format = 1\nsurprise = true\n").is_err());
    }
}
