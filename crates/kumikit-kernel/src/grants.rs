//! Granted permissions: what the user approved for a plugin (`docs/spec/04-plugin-host.md`,
//! permissions). The kernel's services check calls against them; the plugin host builds
//! them from the manifest.

use std::collections::BTreeSet;

/// Workspace file access a plugin may have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesGrant {
    /// No workspace files.
    #[default]
    None,
    /// Read files in the workspaces it acts for.
    ReadWorkspace,
    /// Read and write files in the workspaces it acts for.
    Workspace,
}

/// Where a plugin's client operations may act.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacesGrant {
    /// Nowhere.
    #[default]
    None,
    /// The workspaces the user linked to the plugin.
    Linked,
    /// Every workspace.
    All,
}

/// The permissions granted to a plugin. The default grants nothing. Stored as JSON in
/// the `plugins` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Grants {
    /// Program names it may start.
    pub spawn: BTreeSet<String>,
    /// Hosts it may connect to.
    pub network: BTreeSet<String>,
    /// Workspace file access.
    pub files: FilesGrant,
    /// Secret names it may read.
    pub secrets: BTreeSet<String>,
    /// Whether it may hand tools contributed by other plugins to its agent.
    pub tools: bool,
    /// Client operations it may perform (`kumikit:host/client`).
    pub client: BTreeSet<String>,
    /// Where those operations may act.
    pub workspaces: WorkspacesGrant,
    /// Whether it may run on a schedule.
    pub schedules: bool,
}

impl Grants {
    /// Whether `program` (a bare name) may be started.
    pub fn may_spawn(&self, program: &str) -> bool {
        self.spawn.contains(program)
    }

    /// Whether the client operation `op` (`threads.send`, …) is granted.
    pub fn may_client(&self, op: &str) -> bool {
        self.client.contains(op)
    }

    /// Whether the secret `name` may be read.
    pub fn may_read_secret(&self, name: &str) -> bool {
        self.secrets.contains(name)
    }
}
