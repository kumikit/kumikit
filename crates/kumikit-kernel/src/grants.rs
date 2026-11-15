//! Granted permissions: what the user approved for a plugin (`docs/spec/04-plugin-host.md`,
//! permissions). The kernel's services check calls against them; the plugin host builds
//! them from the manifest.

use std::collections::BTreeSet;

/// Workspace file access a plugin may have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FilesGrant {
    /// No workspace files.
    #[default]
    None,
    /// Read files in the workspaces it acts for.
    ReadWorkspace,
    /// Read and write files in the workspaces it acts for.
    Workspace,
}

/// The permissions granted to a plugin. The default grants nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    /// Program names it may start.
    pub spawn: BTreeSet<String>,
    /// Hosts it may connect to.
    pub network: BTreeSet<String>,
    /// Workspace file access.
    pub files: FilesGrant,
    /// Secret names it may read.
    pub secrets: BTreeSet<String>,
    /// Whether it may call tools contributed by other plugins.
    pub tools: bool,
}

impl Grants {
    /// Whether `program` (a bare name) may be started.
    pub fn may_spawn(&self, program: &str) -> bool {
        self.spawn.contains(program)
    }

    /// Whether the secret `name` may be read.
    pub fn may_read_secret(&self, name: &str) -> bool {
        self.secrets.contains(name)
    }
}
