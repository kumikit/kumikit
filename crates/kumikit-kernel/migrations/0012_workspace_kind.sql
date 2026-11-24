-- Schema 12: distinguish project workspaces (tasks) from the core-managed chat workspace (chats).
ALTER TABLE workspaces ADD COLUMN kind TEXT NOT NULL DEFAULT 'project';
