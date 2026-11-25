//! Connection profiles (`docs/spec/06-client.md#shell-rust-kumikit-shell`,
//! [ADR 0017](../../../docs/decisions/0017-shell-remote-profiles-and-ssh.md)), kept by the
//! shell in `<client data>/profiles.json`. The webview names profiles; it never reads
//! their addresses, and tokens live in [`crate::secrets`], not here.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// The built-in profile of the local core.
pub const LOCAL: &str = "local";

/// A saved profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The connection id in addresses: `[a-z0-9-]{1,32}`.
    pub name: String,
    /// What the user calls it.
    pub label: String,
    /// How to reach it.
    #[serde(flatten)]
    pub kind: Kind,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// How a profile reaches its core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    /// WebSocket over TLS, with a device token in the client secrets.
    Tls {
        /// `host:port` as the user gave it.
        address: String,
        /// The pinned certificate fingerprint (`SHA256:…`).
        fingerprint: String,
        /// The device id the core gave at pairing.
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    /// The system `ssh`, running a bridge on the far side.
    Ssh {
        /// A destination as `ssh` understands it.
        destination: String,
        /// The remote command; `kumikit bridge` when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// Automatically download and install matching `kumikit` over SSH when missing or mismatched.
        #[serde(default, rename = "autoSync", skip_serializing_if = "is_false")]
        auto_sync: bool,
    },
}

/// What the webview may see of a profile: no address, host or fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    /// The connection id.
    pub name: String,
    /// The label.
    pub label: String,
    /// `local`, `tls` or `ssh`.
    pub kind: &'static str,
    /// Present for `ssh` profiles: whether matching `kumikit` is synced automatically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_sync: Option<bool>,
}

impl Profile {
    /// The webview's view of it.
    pub fn info(&self) -> ProfileInfo {
        ProfileInfo {
            name: self.name.clone(),
            label: self.label.clone(),
            kind: match self.kind {
                Kind::Tls { .. } => "tls",
                Kind::Ssh { .. } => "ssh",
            },
            auto_sync: match self.kind {
                Kind::Ssh { auto_sync, .. } => Some(auto_sync),
                Kind::Tls { .. } => None,
            },
        }
    }
}

/// Errors of the profile store.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The name breaks the naming rule or is reserved.
    #[error("a core's name must be 1–32 lowercase letters, digits or dashes, and not `local`")]
    BadName,
    /// The label is empty or too long.
    #[error("a core's label must be 1–64 characters without control characters")]
    BadLabel,
    /// The name is taken.
    #[error("a core named {0} already exists")]
    Exists(String),
    /// No such profile.
    #[error("no core named {0}")]
    Unknown(String),
    /// The file could not be read or written.
    #[error("the profiles file: {0}")]
    Io(#[from] io::Error),
}

/// Whether a profile name is acceptable.
pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name != LOCAL
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_label(label: &str) -> bool {
    let n = label.chars().count();
    (1..=64).contains(&n) && !label.chars().any(char::is_control) && !label.trim().is_empty()
}

/// The profiles file. Cheap to clone; writes are serialized.
#[derive(Debug, Clone)]
pub struct Profiles {
    path: PathBuf,
    write: Arc<Mutex<()>>,
}

impl Profiles {
    /// Profiles kept in `<client data>/profiles.json`.
    pub fn new(client_data: &Path) -> Self {
        Profiles {
            path: client_data.join("profiles.json"),
            write: Arc::new(Mutex::new(())),
        }
    }

    /// Every saved profile, in the order added.
    pub fn list(&self) -> Result<Vec<Profile>, ProfileError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// One profile.
    pub fn get(&self, name: &str) -> Result<Profile, ProfileError> {
        self.list()?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| ProfileError::Unknown(name.to_owned()))
    }

    /// Checks that a new profile could be added, before any work is done for it.
    pub fn check_new(&self, name: &str, label: &str) -> Result<(), ProfileError> {
        if !valid_name(name) {
            return Err(ProfileError::BadName);
        }
        if !valid_label(label) {
            return Err(ProfileError::BadLabel);
        }
        if self.list()?.iter().any(|p| p.name == name) {
            return Err(ProfileError::Exists(name.to_owned()));
        }
        Ok(())
    }

    /// Adds a profile.
    pub fn add(&self, profile: Profile) -> Result<(), ProfileError> {
        let _guard = self.write.lock().unwrap_or_else(|p| p.into_inner());
        self.check_new(&profile.name, &profile.label)?;
        let mut all = self.list()?;
        all.push(profile);
        self.save(&all)
    }

    /// Removes a profile; returns it.
    pub fn remove(&self, name: &str) -> Result<Profile, ProfileError> {
        let _guard = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let mut all = self.list()?;
        let i = all
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| ProfileError::Unknown(name.to_owned()))?;
        let removed = all.remove(i);
        self.save(&all)?;
        Ok(removed)
    }

    /// Updates an SSH profile's optional command and/or `auto_sync` flag.
    pub fn update_ssh(
        &self,
        name: &str,
        command: Option<Option<String>>,
        auto_sync: Option<bool>,
    ) -> Result<Profile, ProfileError> {
        let _guard = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let mut all = self.list()?;
        let profile = all
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| ProfileError::Unknown(name.to_owned()))?;
        match &mut profile.kind {
            Kind::Ssh {
                command: cur_cmd,
                auto_sync: cur_sync,
                ..
            } => {
                if let Some(next_cmd) = command {
                    *cur_cmd = next_cmd;
                }
                if let Some(next_sync) = auto_sync {
                    *cur_sync = next_sync;
                }
            }
            Kind::Tls { .. } => return Err(ProfileError::Unknown(name.to_owned())),
        }
        let updated = profile.clone();
        self.save(&all)?;
        Ok(updated)
    }

    fn save(&self, all: &[Profile]) -> Result<(), ProfileError> {
        let bytes = serde_json::to_vec_pretty(all).map_err(io::Error::other)?;
        write_private(&self.path, &bytes)?;
        Ok(())
    }
}

/// Writes a file only this user can read (0600 on Unix), replacing it atomically and
/// creating its directory if needed.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tls(name: &str) -> Profile {
        Profile {
            name: name.into(),
            label: "Dev box".into(),
            kind: Kind::Tls {
                address: "devbox:7463".into(),
                fingerprint: "SHA256:00".into(),
                device_id: "D1".into(),
            },
        }
    }

    #[test]
    fn names() {
        assert!(valid_name("devbox"));
        assert!(valid_name("vm-3"));
        assert!(!valid_name("local"));
        assert!(!valid_name("Dev"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(33)));
    }

    #[test]
    fn add_list_remove_and_the_file_format() {
        let dir = tempfile::tempdir().unwrap();
        let p = Profiles::new(dir.path());
        assert_eq!(p.list().unwrap(), vec![]);
        p.add(tls("devbox")).unwrap();
        p.add(Profile {
            name: "vm-3".into(),
            label: "VM 3".into(),
            kind: Kind::Ssh {
                destination: "me@vm-3".into(),
                command: None,
                auto_sync: false,
            },
        })
        .unwrap();
        assert!(matches!(p.add(tls("devbox")), Err(ProfileError::Exists(_))));
        assert!(matches!(p.add(tls("local")), Err(ProfileError::BadName)));
        let names: Vec<_> = p.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["devbox", "vm-3"]);

        let text = std::fs::read_to_string(dir.path().join("profiles.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v[0]["kind"], "tls");
        assert_eq!(v[0]["deviceId"], "D1");
        assert_eq!(v[1]["kind"], "ssh");
        assert!(v[1].get("command").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("profiles.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        assert_eq!(p.remove("devbox").unwrap().name, "devbox");
        assert!(matches!(p.remove("devbox"), Err(ProfileError::Unknown(_))));
        assert_eq!(p.get("vm-3").unwrap().info().kind, "ssh");
    }

    #[test]
    fn the_webview_view_has_no_address() {
        let info = serde_json::to_value(tls("devbox").info()).unwrap();
        assert_eq!(
            info,
            serde_json::json!({"name": "devbox", "label": "Dev box", "kind": "tls"})
        );
    }
}
