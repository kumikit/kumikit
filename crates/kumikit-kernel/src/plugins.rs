//! Plugin records and per-plugin key-value stores (`docs/spec/03-kernel.md#storage`).
//!
//! The kernel stores what the plugin host decides: which plugins are installed, their
//! state and their granted permissions. It does not interpret manifests.

use kumikit_protocol::model::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{KernelError, KernelResult};
use crate::grants::Grants;
use crate::kernel::Kernel;

/// Longest kv key, in bytes.
pub const KV_KEY_MAX: usize = 1024;
/// Largest kv value, in bytes.
pub const KV_VALUE_MAX: usize = 1 << 20;

/// Whether the user enabled a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// Unpacked; permissions not granted.
    Installed,
    /// Granted and enabled.
    Enabled,
    /// Granted earlier, turned off by the user.
    Disabled,
}

impl PluginState {
    fn as_str(self) -> &'static str {
        match self {
            PluginState::Installed => "installed",
            PluginState::Enabled => "enabled",
            PluginState::Disabled => "disabled",
        }
    }

    fn parse(s: &str) -> KernelResult<Self> {
        match s {
            "installed" => Ok(PluginState::Installed),
            "enabled" => Ok(PluginState::Enabled),
            "disabled" => Ok(PluginState::Disabled),
            other => Err(KernelError::InvalidParams(format!(
                "unknown plugin state {other} in the database"
            ))),
        }
    }
}

/// Where an installed plugin came from (10-distribution, Lockfiles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    /// A local package file or folder.
    File,
    /// Preinstalled from the bundled directory.
    Bundled,
    /// A registry, by its index URL.
    Registry(String),
}

impl PluginSource {
    /// The stored form: `file`, `bundled` or `registry+<url>`.
    pub fn as_stored(&self) -> String {
        match self {
            PluginSource::File => "file".into(),
            PluginSource::Bundled => "bundled".into(),
            PluginSource::Registry(url) => format!("registry+{url}"),
        }
    }

    /// Reads the stored form; anything unknown reads as `file`.
    pub fn from_stored(s: &str) -> PluginSource {
        match s {
            "bundled" => PluginSource::Bundled,
            _ => match s.strip_prefix("registry+") {
                Some(url) => PluginSource::Registry(url.to_owned()),
                None => PluginSource::File,
            },
        }
    }
}

/// An installed plugin, as the kernel stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRecord {
    /// Plugin id.
    pub id: String,
    /// Installed version.
    pub version: String,
    /// State.
    pub state: PluginState,
    /// SHA-256 of the package file, hex.
    pub sha256: String,
    /// Granted permissions; `None` until granted.
    pub granted: Option<Grants>,
    /// When it was installed.
    pub installed: Timestamp,
    /// The id of the key that signed the package (`ed25519:…`), pinned for later
    /// versions; `None` for an unsigned package.
    pub publisher_key: Option<String>,
    /// Where it came from.
    pub source: PluginSource,
}

fn row_to_record(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(PluginRecord, String, Option<String>)> {
    let state: String = r.get(2)?;
    let granted: Option<String> = r.get(4)?;
    Ok((
        PluginRecord {
            id: r.get(0)?,
            version: r.get(1)?,
            state: PluginState::Installed,
            sha256: r.get(3)?,
            granted: None,
            installed: Timestamp(r.get(5)?),
            publisher_key: r.get(6)?,
            source: PluginSource::from_stored(&r.get::<_, String>(7)?),
        },
        state,
        granted,
    ))
}

fn finish(parts: (PluginRecord, String, Option<String>)) -> KernelResult<PluginRecord> {
    let (mut rec, state, granted) = parts;
    rec.state = PluginState::parse(&state)?;
    rec.granted = granted.map(|g| serde_json::from_str(&g)).transpose()?;
    Ok(rec)
}

const SELECT: &str =
    "SELECT id, version, state, sha256, granted, installed, publisher_key, source FROM plugins";

fn plugin_get(c: &Connection, id: &str) -> KernelResult<Option<PluginRecord>> {
    c.query_row(&format!("{SELECT} WHERE id = ?1"), [id], row_to_record)
        .optional()?
        .map(finish)
        .transpose()
}

fn check_key(key: &str) -> KernelResult<()> {
    if key.len() > KV_KEY_MAX {
        return Err(KernelError::Limit(format!(
            "kv keys are at most {KV_KEY_MAX} bytes"
        )));
    }
    Ok(())
}

impl Kernel {
    /// Every installed plugin, by id.
    pub async fn plugin_list(&self) -> KernelResult<Vec<PluginRecord>> {
        self.inner
            .store
            .read(|c| {
                let mut stmt = c.prepare(&format!("{SELECT} ORDER BY id"))?;
                let rows = stmt.query_map([], row_to_record)?;
                rows.map(|r| finish(r?)).collect()
            })
            .await
    }

    /// Whether a bundled plugin with this id was ever seeded (installed from
    /// `--bundled-plugins`), even if it was removed since.
    pub async fn plugin_was_seeded(&self, id: &str) -> KernelResult<bool> {
        let id = id.to_owned();
        self.inner
            .store
            .read(move |c| {
                let n: i64 = c.query_row(
                    "SELECT count(*) FROM seeded_plugins WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )?;
                Ok(n > 0)
            })
            .await
    }

    /// Records that a bundled plugin was seeded.
    pub async fn plugin_mark_seeded(&self, id: &str, version: &str) -> KernelResult<()> {
        let (id, version, now) = (id.to_owned(), version.to_owned(), self.now().0);
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "INSERT INTO seeded_plugins (id, version, seeded) VALUES (?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET version = excluded.version",
                    [&id, &version, &now],
                )?;
                Ok(())
            })
            .await
    }

    /// One installed plugin.
    pub async fn plugin_get(&self, id: &str) -> KernelResult<Option<PluginRecord>> {
        let id = id.to_owned();
        self.inner.store.read(move |c| plugin_get(c, &id)).await
    }

    /// Records a newly installed plugin. Fails with `conflict` if the id is installed.
    pub async fn plugin_insert(&self, record: PluginRecord) -> KernelResult<()> {
        let granted = record
            .granted
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.inner
            .store
            .write(move |c| {
                if plugin_get(c, &record.id)?.is_some() {
                    return Err(KernelError::Conflict(format!(
                        "plugin {} is already installed",
                        record.id
                    )));
                }
                c.execute(
                    "INSERT INTO plugins
                       (id, version, state, sha256, granted, installed, publisher_key, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        record.id,
                        record.version,
                        record.state.as_str(),
                        record.sha256,
                        granted,
                        record.installed.0,
                        record.publisher_key,
                        record.source.as_stored()
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Sets a plugin's state and grants.
    pub async fn plugin_set_state(
        &self,
        id: &str,
        state: PluginState,
        granted: Option<Grants>,
    ) -> KernelResult<()> {
        let id = id.to_owned();
        let granted = granted.as_ref().map(serde_json::to_string).transpose()?;
        self.inner
            .store
            .write(move |c| {
                let n = c.execute(
                    "UPDATE plugins SET state = ?2, granted = ?3 WHERE id = ?1",
                    params![id, state.as_str(), granted],
                )?;
                if n == 0 {
                    return Err(KernelError::NotFound(format!("no plugin {id}")));
                }
                Ok(())
            })
            .await
    }

    /// Removes a plugin's record, and its kv store if `delete_data`.
    pub async fn plugin_delete(&self, id: &str, delete_data: bool) -> KernelResult<()> {
        let id = id.to_owned();
        self.inner
            .store
            .write(move |c| {
                let n = c.execute("DELETE FROM plugins WHERE id = ?1", [&id])?;
                if n == 0 {
                    return Err(KernelError::NotFound(format!("no plugin {id}")));
                }
                if delete_data {
                    c.execute("DELETE FROM plugin_kv WHERE plugin = ?1", [&id])?;
                }
                Ok(())
            })
            .await
    }

    // ---- kv ----

    /// The value of `key` in `plugin`'s store.
    pub async fn kv_get(&self, plugin: &str, key: &str) -> KernelResult<Option<Vec<u8>>> {
        check_key(key)?;
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT value FROM plugin_kv WHERE plugin = ?1 AND key = ?2",
                    [&plugin, &key],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await
    }

    /// Stores `value` under `key`.
    pub async fn kv_set(&self, plugin: &str, key: &str, value: Vec<u8>) -> KernelResult<()> {
        check_key(key)?;
        if value.len() > KV_VALUE_MAX {
            return Err(KernelError::Limit(format!(
                "kv values are at most {KV_VALUE_MAX} bytes"
            )));
        }
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "INSERT INTO plugin_kv (plugin, key, value) VALUES (?1, ?2, ?3)
                     ON CONFLICT (plugin, key) DO UPDATE SET value = excluded.value",
                    params![plugin, key, value],
                )?;
                Ok(())
            })
            .await
    }

    /// Removes `key`; removing a missing key succeeds.
    pub async fn kv_delete(&self, plugin: &str, key: &str) -> KernelResult<()> {
        check_key(key)?;
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "DELETE FROM plugin_kv WHERE plugin = ?1 AND key = ?2",
                    [&plugin, &key],
                )?;
                Ok(())
            })
            .await
    }

    /// Keys of `plugin`'s store that start with `prefix`, in byte order.
    pub async fn kv_list(&self, plugin: &str, prefix: &str) -> KernelResult<Vec<String>> {
        check_key(prefix)?;
        let (plugin, prefix) = (plugin.to_owned(), prefix.to_owned());
        self.inner
            .store
            .read(move |c| {
                // Scan the primary key from the prefix on; stop at the first key past it.
                let mut stmt = c.prepare(
                    "SELECT key FROM plugin_kv WHERE plugin = ?1 AND key >= ?2 ORDER BY key",
                )?;
                let mut rows = stmt.query([&plugin, &prefix])?;
                let mut keys = Vec::new();
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    if !key.starts_with(&prefix) {
                        break;
                    }
                    keys.push(key);
                }
                Ok(keys)
            })
            .await
    }
}

#[cfg(test)]
mod tests;
