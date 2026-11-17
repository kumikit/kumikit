//! What the shell's profile commands do (`docs/spec/06-client.md#shell-rust-kumikit-shell`):
//! list, probe, pair, add an SSH destination, remove, and connect a remote profile to the
//! bridge. Tauri's commands and the test relay both call this, so it is tested without a
//! window. The local profile is connected by [`crate::local`].

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::bridge::{Bridge, Incoming};
use crate::profiles::{Kind, LOCAL, Profile, ProfileError, ProfileInfo, Profiles};
use crate::remote::{self, RemoteError};
use crate::secrets::TokenStore;
use crate::ssh;

/// Keepalive of remote connections.
#[derive(Debug, Clone, Copy)]
pub struct Keepalive {
    /// Ping interval.
    pub ping: Duration,
    /// Silence after which the core is treated as gone.
    pub idle: Duration,
}

impl Default for Keepalive {
    fn default() -> Self {
        Keepalive {
            ping: Duration::from_secs(20),
            idle: Duration::from_secs(45),
        }
    }
}

/// An error the webview acts on: a stable code and a message in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    /// `unauthorized`, `certificate_changed`, `unreachable`, `bad_address`, `protocol`,
    /// `invalid` (bad input) or `internal`.
    pub code: &'static str,
    /// What happened.
    pub message: String,
}

impl CommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        CommandError {
            code,
            message: message.into(),
        }
    }
}

impl From<RemoteError> for CommandError {
    fn from(e: RemoteError) -> Self {
        CommandError::new(e.code(), e.to_string())
    }
}

impl From<ProfileError> for CommandError {
    fn from(e: ProfileError) -> Self {
        let code = match e {
            ProfileError::Io(_) => "internal",
            _ => "invalid",
        };
        CommandError::new(code, e.to_string())
    }
}

impl From<crate::secrets::SecretError> for CommandError {
    fn from(e: crate::secrets::SecretError) -> Self {
        CommandError::new("internal", e.to_string())
    }
}

/// `profile_pair`'s request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    /// The new profile's name.
    pub name: String,
    /// Its label.
    pub label: String,
    /// `host[:port]`.
    pub address: String,
    /// The fingerprint the user confirmed.
    pub fingerprint: String,
    /// The pairing code.
    pub code: String,
    /// This device's name; the host name by default.
    #[serde(default)]
    pub device_name: Option<String>,
}

/// `profile_add_ssh`'s request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshRequest {
    /// The new profile's name.
    pub name: String,
    /// Its label.
    pub label: String,
    /// The SSH destination.
    pub destination: String,
    /// The remote command; `kumikit bridge` by default.
    #[serde(default)]
    pub command: Option<String>,
}

/// `profile_remove`'s answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    /// Whether the core was reached and the device revoked there.
    pub revoked: bool,
    /// The device id to revoke by hand (`kumikit devices revoke <id>`) if not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// The profile commands' state.
#[derive(Debug, Clone)]
pub struct Connections {
    /// Saved profiles.
    pub profiles: Profiles,
    /// Device tokens.
    pub secrets: TokenStore,
    /// The `ssh` program.
    pub ssh: PathBuf,
    /// Keepalive of TLS connections.
    pub keepalive: Keepalive,
    /// The default device name (this computer's host name).
    pub device_name: String,
}

impl Connections {
    /// `profiles_list`: `local` first, then the saved ones.
    pub fn list(&self) -> Result<Vec<ProfileInfo>, CommandError> {
        let mut all = vec![ProfileInfo {
            name: LOCAL.into(),
            label: "This computer".into(),
            kind: "local",
        }];
        all.extend(self.profiles.list()?.iter().map(Profile::info));
        Ok(all)
    }

    /// `profile_probe`: the fingerprint of the core at `address`.
    pub async fn probe(&self, address: &str) -> Result<String, CommandError> {
        Ok(remote::probe(address).await?)
    }

    /// `profile_pair`: pairs, keeps the token, saves the profile.
    pub async fn pair(&self, req: PairRequest) -> Result<ProfileInfo, CommandError> {
        self.profiles.check_new(&req.name, &req.label)?;
        let device_name = req
            .device_name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.device_name.clone());
        let paired = remote::pair(&req.address, &req.fingerprint, &req.code, &device_name).await?;
        self.secrets.set(&req.name, &paired.token)?;
        let profile = Profile {
            name: req.name.clone(),
            label: req.label,
            kind: Kind::Tls {
                address: req.address,
                fingerprint: req.fingerprint,
                device_id: paired.device_id,
            },
        };
        let info = profile.info();
        if let Err(e) = self.profiles.add(profile) {
            let _ = self.secrets.delete(&req.name);
            return Err(e.into());
        }
        Ok(info)
    }

    /// `profile_add_ssh`: checks the destination reaches a core, then saves it.
    pub async fn add_ssh(&self, req: SshRequest) -> Result<ProfileInfo, CommandError> {
        self.profiles.check_new(&req.name, &req.label)?;
        let command = req.command.filter(|c| !c.trim().is_empty());
        let run = command.as_deref().unwrap_or(ssh::DEFAULT_COMMAND);
        let started = ssh::spawn(&self.ssh, &req.destination, run)
            .map_err(|e| CommandError::new("unreachable", e.to_string()))?;
        ssh::check(started.stdout, started.stdin, &started.stderr)
            .await
            .map_err(|m| CommandError::new("unreachable", m))?;
        let profile = Profile {
            name: req.name,
            label: req.label,
            kind: Kind::Ssh {
                destination: req.destination,
                command,
            },
        };
        let info = profile.info();
        self.profiles.add(profile)?;
        Ok(info)
    }

    /// `profile_remove`: revokes this device on the core if it answers within 2 s, then
    /// forgets the token and the profile.
    pub async fn remove(&self, name: &str) -> Result<Removed, CommandError> {
        let profile = self.profiles.get(name)?;
        let mut removed = Removed {
            revoked: false,
            device_id: None,
        };
        if let Kind::Tls {
            address,
            fingerprint,
            device_id,
        } = &profile.kind
        {
            if let Some(token) = self.secrets.get(name)? {
                removed.revoked = remote::revoke_self(
                    address,
                    fingerprint,
                    &token,
                    device_id,
                    Duration::from_secs(2),
                )
                .await;
            }
            if !removed.revoked {
                removed.device_id = Some(device_id.clone());
            }
            self.secrets.delete(name)?;
        } else {
            removed.revoked = true;
        }
        self.profiles.remove(name)?;
        Ok(removed)
    }

    /// `bridge_connect` for a saved profile: connects and attaches it to the bridge.
    pub async fn connect(
        &self,
        bridge: &Bridge,
        name: &str,
        sink: impl Fn(Incoming) + Send + Sync + 'static,
    ) -> Result<String, CommandError> {
        let profile = self
            .profiles
            .get(name)
            .map_err(|_| CommandError::new("unknown_profile", format!("no core named {name}")))?;
        match profile.kind {
            Kind::Tls {
                address,
                fingerprint,
                ..
            } => {
                let token = self.secrets.get(name)?.ok_or_else(|| {
                    CommandError::new(
                        "unauthorized",
                        "this computer has no token for the core; add it again",
                    )
                })?;
                let ws = remote::connect(&address, &fingerprint, &token).await?;
                Ok(bridge.attach_ws(ws, self.keepalive.ping, self.keepalive.idle, sink))
            }
            Kind::Ssh {
                destination,
                command,
            } => {
                let run = command.as_deref().unwrap_or(ssh::DEFAULT_COMMAND);
                let started = ssh::spawn(&self.ssh, &destination, run)
                    .map_err(|e| CommandError::new("unreachable", e.to_string()))?;
                let stderr = started.stderr;
                Ok(bridge.attach_halves(
                    started.stdout,
                    started.stdin,
                    sink,
                    move |reason| async move { stderr.explain(reason).await },
                ))
            }
        }
    }
}
