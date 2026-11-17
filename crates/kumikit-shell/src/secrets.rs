//! Client secrets: device tokens of paired cores (`docs/spec/06-client.md#shell-rust-kumikit-shell`,
//! [ADR 0017](../../../docs/decisions/0017-shell-remote-profiles-and-ssh.md)).
//!
//! The macOS Keychain and the Windows Credential Manager where they exist; elsewhere, or
//! when `KUMIKIT_CLIENT_SECRETS=file`, a file only this user can read. No command returns
//! a token, and tokens are never logged.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use keyring_core::CredentialStore;

/// The keychain service name.
pub const SERVICE: &str = "dev.kumikit.client";

/// Where device tokens are kept.
#[derive(Clone)]
pub enum TokenStore {
    /// `secrets.json`, mode 0600.
    File(FileStore),
    /// An OS credential store.
    Keychain(Arc<CredentialStore>),
}

impl std::fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenStore::File(s) => f.debug_tuple("File").field(&s.path).finish(),
            TokenStore::Keychain(_) => f.write_str("Keychain"),
        }
    }
}

/// Errors of the token store, without the token.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The file could not be read or written.
    #[error("the client secrets file: {0}")]
    File(#[from] io::Error),
    /// The OS credential store failed.
    #[error("the system keychain: {0}")]
    Keychain(String),
}

impl TokenStore {
    /// The platform's store: the OS keychain on macOS and Windows, unless `force_file`;
    /// otherwise `<client data>/secrets.json`.
    pub fn for_platform(client_data: &Path, force_file: bool) -> TokenStore {
        let file = || TokenStore::File(FileStore::new(client_data.join("secrets.json")));
        if force_file {
            return file();
        }
        match native() {
            Some(store) => TokenStore::Keychain(store),
            None => file(),
        }
    }

    /// The token for a profile, if one is stored.
    pub fn get(&self, profile: &str) -> Result<Option<String>, SecretError> {
        match self {
            TokenStore::File(f) => Ok(f.load()?.remove(profile)),
            TokenStore::Keychain(store) => match entry(store, profile)?.get_password() {
                Ok(t) => Ok(Some(t)),
                Err(keyring_core::Error::NoEntry) => Ok(None),
                Err(e) => Err(SecretError::Keychain(e.to_string())),
            },
        }
    }

    /// Stores a profile's token, replacing any.
    pub fn set(&self, profile: &str, token: &str) -> Result<(), SecretError> {
        match self {
            TokenStore::File(f) => f.update(|m| {
                m.insert(profile.to_owned(), token.to_owned());
            }),
            TokenStore::Keychain(store) => entry(store, profile)?
                .set_password(token)
                .map_err(|e| SecretError::Keychain(e.to_string())),
        }
    }

    /// Forgets a profile's token. Forgetting a missing one is not an error.
    pub fn delete(&self, profile: &str) -> Result<(), SecretError> {
        match self {
            TokenStore::File(f) => f.update(|m| {
                m.remove(profile);
            }),
            TokenStore::Keychain(store) => match entry(store, profile)?.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(e) => Err(SecretError::Keychain(e.to_string())),
            },
        }
    }
}

fn entry(store: &Arc<CredentialStore>, profile: &str) -> Result<keyring_core::Entry, SecretError> {
    store
        .build(SERVICE, profile, None)
        .map_err(|e| SecretError::Keychain(e.to_string()))
}

#[cfg(target_os = "macos")]
fn native() -> Option<Arc<CredentialStore>> {
    apple_native_keyring_store::keychain::Store::new()
        .ok()
        .map(|s| s as Arc<CredentialStore>)
}

#[cfg(windows)]
fn native() -> Option<Arc<CredentialStore>> {
    windows_native_keyring_store::Store::new()
        .ok()
        .map(|s| s as Arc<CredentialStore>)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn native() -> Option<Arc<CredentialStore>> {
    None
}

/// A JSON object of profile → token in a 0600 file, written atomically.
#[derive(Debug, Clone)]
pub struct FileStore {
    path: PathBuf,
    write: Arc<Mutex<()>>,
}

impl FileStore {
    /// A store kept in `path`; the file is created on the first write.
    pub fn new(path: PathBuf) -> Self {
        FileStore {
            path,
            write: Arc::new(Mutex::new(())),
        }
    }

    fn load(&self) -> io::Result<BTreeMap<String, String>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e),
        }
    }

    fn update(&self, f: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<(), SecretError> {
        let _guard = self.write.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = self.load()?;
        f(&mut map);
        let bytes = serde_json::to_vec(&map).map_err(io::Error::other)?;
        crate::profiles::write_private(&self.path, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(store: &TokenStore) {
        assert_eq!(store.get("devbox").unwrap(), None);
        store.set("devbox", "kkd_one").unwrap();
        store.set("vm-3", "kkd_two").unwrap();
        assert_eq!(store.get("devbox").unwrap().as_deref(), Some("kkd_one"));
        store.set("devbox", "kkd_three").unwrap();
        assert_eq!(store.get("devbox").unwrap().as_deref(), Some("kkd_three"));
        store.delete("devbox").unwrap();
        store.delete("devbox").unwrap();
        assert_eq!(store.get("devbox").unwrap(), None);
        assert_eq!(store.get("vm-3").unwrap().as_deref(), Some("kkd_two"));
        store.delete("vm-3").unwrap();
    }

    #[test]
    fn the_file_store_round_trips_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::for_platform(dir.path(), true);
        round_trip(&store);
        store.set("devbox", "kkd_x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secrets.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_keychain_code_works_with_a_credential_store() {
        let store = TokenStore::Keychain(keyring_core::mock::Store::new().unwrap());
        round_trip(&store);
    }

    /// The real OS keychain, where there is one (CI on macOS and Windows).
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn the_os_keychain_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::for_platform(dir.path(), false);
        assert!(matches!(store, TokenStore::Keychain(_)), "{store:?}");
        let name = format!("test-{}", std::process::id());
        store.set(&name, "kkd_test").unwrap();
        assert_eq!(store.get(&name).unwrap().as_deref(), Some("kkd_test"));
        store.delete(&name).unwrap();
        assert_eq!(store.get(&name).unwrap(), None);
        assert!(!dir.path().join("secrets.json").exists());
    }
}
