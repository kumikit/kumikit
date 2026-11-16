//! The secret store (`docs/spec/03-kernel.md#secrets`), file backend: a JSON object of
//! name → value in a file only the core's user can read (0600 on Linux and macOS; on
//! Windows the core data directory is already private to the user).
//!
//! Values are never logged. Writes replace the file atomically (write a temporary file,
//! then rename).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::{KernelError, KernelResult};

/// The secret store. Cheap to clone.
#[derive(Clone)]
pub struct SecretStore {
    path: Arc<PathBuf>,
    /// Serializes writers, so concurrent sets cannot lose each other's changes.
    write: Arc<Mutex<()>>,
}

impl std::fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn io(e: impl std::fmt::Display) -> KernelError {
    KernelError::Io(format!("secret store: {e}"))
}

fn load(path: &Path) -> KernelResult<BTreeMap<String, String>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(io(e)),
    }
}

fn save(path: &Path, map: &BTreeMap<String, String>) -> KernelResult<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(map).map_err(io)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = options.open(&tmp).map_err(io)?;
        f.write_all(&bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

impl SecretStore {
    /// A store kept in `path`. The file is created on the first write.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        SecretStore {
            path: Arc::new(path.into()),
            write: Arc::new(Mutex::new(())),
        }
    }

    /// The value of `name`, if set.
    pub async fn get(&self, name: &str) -> KernelResult<Option<String>> {
        let (path, name) = (Arc::clone(&self.path), name.to_owned());
        tokio::task::spawn_blocking(move || Ok(load(&path)?.remove(&name)))
            .await
            .map_err(io)?
    }

    /// Names that are set (never values).
    pub async fn names(&self) -> KernelResult<Vec<String>> {
        let path = Arc::clone(&self.path);
        tokio::task::spawn_blocking(move || Ok(load(&path)?.into_keys().collect()))
            .await
            .map_err(io)?
    }

    /// Sets `name`.
    pub async fn set(&self, name: &str, value: &str) -> KernelResult<()> {
        let _guard = self.write.lock().await;
        let (path, name, value) = (Arc::clone(&self.path), name.to_owned(), value.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut map = load(&path)?;
            map.insert(name, value);
            save(&path, &map)
        })
        .await
        .map_err(io)?
    }

    /// Deletes `name`; deleting a missing name succeeds.
    pub async fn delete(&self, name: &str) -> KernelResult<()> {
        let _guard = self.write.lock().await;
        let (path, name) = (Arc::clone(&self.path), name.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut map = load(&path)?;
            if map.remove(&name).is_some() {
                save(&path, &map)?;
            }
            Ok(())
        })
        .await
        .map_err(io)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_get_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.json"));
        assert_eq!(store.get("a").await.unwrap(), None);
        store.set("a", "1").await.unwrap();
        store.set("b", "2").await.unwrap();
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some("1"));
        assert_eq!(store.names().await.unwrap(), ["a", "b"]);
        store.delete("a").await.unwrap();
        store.delete("missing").await.unwrap();
        assert_eq!(store.get("a").await.unwrap(), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        SecretStore::new(&path).set("a", "1").await.unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
