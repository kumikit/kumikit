-- Plugin settings (docs/spec/03-kernel.md#plugin-settings): one JSON object per plugin,
-- validated by the plugin host against the plugin's schema before it is stored.
CREATE TABLE plugin_settings (
    plugin TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
) STRICT;
