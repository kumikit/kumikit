-- Workspaces the user linked to a plugin whose permissions.workspaces is `linked`
-- (docs/spec/03-kernel.md#client-interface).
CREATE TABLE plugin_workspaces (
    plugin    TEXT NOT NULL,
    workspace TEXT NOT NULL,
    PRIMARY KEY (plugin, workspace)
) STRICT, WITHOUT ROWID;
