//! The Unix domain socket transport: a private directory, a 0600 socket and a
//! peer-credential check on every connection.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

/// A bound socket that accepts only connections from this user.
#[derive(Debug)]
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
    uid: u32,
}

/// The current user id.
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Whether a peer with `peer` uid may connect to a core running as `own`.
pub fn peer_allowed(peer: u32, own: u32) -> bool {
    peer == own
}

/// Makes sure `dir` exists, is a directory owned by `uid`, and grants nothing to group
/// or others. Creates it with mode 0700 if missing.
pub fn ensure_private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::other(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    if meta.uid() != uid {
        return Err(io::Error::other(format!(
            "{} is owned by another user",
            dir.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} is accessible by other users (mode {:o}); it must be 0700",
            dir.display(),
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

impl Listener {
    /// Binds the socket at `path`. Fails if another core is listening there, if the path
    /// is something other than a socket, or if its directory is not private.
    pub async fn bind(path: &Path) -> io::Result<Listener> {
        let uid = current_uid();
        let dir = path
            .parent()
            .ok_or_else(|| io::Error::other("socket path has no parent directory"))?;
        ensure_private_dir(dir, uid)?;

        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_socket() => {
                if UnixStream::connect(path).await.is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("another core is listening on {}", path.display()),
                    ));
                }
                // Nobody answers: a stale socket from a core that did not shut down.
                std::fs::remove_file(path)?;
            }
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "{} exists and is not a socket",
                    path.display()
                )));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        let listener = UnixListener::bind(path)?;
        // The directory is 0700, so nobody else can reach the socket in the moment
        // before this chmod.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener {
            listener,
            path: path.to_path_buf(),
            uid,
        })
    }

    /// Waits for the next connection from this user; others are closed unread.
    pub async fn accept(&self) -> io::Result<UnixStream> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            match stream.peer_cred() {
                Ok(cred) if peer_allowed(cred.uid(), self.uid) => return Ok(stream),
                Ok(cred) => {
                    tracing::warn!(
                        peer_uid = cred.uid(),
                        "rejected a connection from another user"
                    );
                }
                Err(e) => tracing::warn!(error = %e, "rejected a connection without credentials"),
            }
        }
    }

    /// Removes the socket file.
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Connects to a core's socket.
pub async fn connect(path: &str) -> io::Result<UnixStream> {
    UnixStream::connect(path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Socket paths are limited to about 104 bytes, so tests use a short directory. It
    /// must be private: tempfile creates directories with the umask's mode (often 0755).
    fn short_dir() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("kk")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn peer_check() {
        assert!(peer_allowed(1000, 1000));
        assert!(!peer_allowed(1001, 1000));
        assert!(!peer_allowed(0, 1000), "not even root is let in by uid");
    }

    #[tokio::test]
    async fn socket_is_0600_in_a_private_dir() {
        let dir = short_dir();
        let path = dir.path().join("run/s.sock");
        let l = Listener::bind(&path).await.unwrap();
        let mode = std::fs::metadata(&path).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        l.remove();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn refuses_a_shared_directory() {
        let dir = short_dir();
        let shared = dir.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = Listener::bind(&shared.join("s.sock")).await.unwrap_err();
        assert!(err.to_string().contains("0700"), "{err}");
    }

    #[tokio::test]
    async fn refuses_a_live_socket_and_replaces_a_stale_one() {
        let dir = short_dir();
        let path = dir.path().join("s.sock");
        let live = Listener::bind(&path).await.unwrap();
        let err = Listener::bind(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        drop(live); // closes the listener but leaves the file: now stale
        assert!(path.exists());
        let again = Listener::bind(&path).await.unwrap();
        again.remove();
    }

    #[tokio::test]
    async fn refuses_a_non_socket_file() {
        let dir = short_dir();
        let path = dir.path().join("s.sock");
        std::fs::write(&path, "x").unwrap();
        assert!(Listener::bind(&path).await.is_err());
    }

    #[tokio::test]
    async fn accepts_same_user() {
        let dir = short_dir();
        let path = dir.path().join("s.sock");
        let l = Listener::bind(&path).await.unwrap();
        let p = path.to_str().unwrap().to_owned();
        let client = tokio::spawn(async move { connect(&p).await });
        l.accept().await.unwrap();
        client.await.unwrap().unwrap();
    }
}
