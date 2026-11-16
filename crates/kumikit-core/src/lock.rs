//! One core per data directory (`docs/spec/03-kernel.md#storage`): `kumikit serve`, and
//! CLI commands that work on the data directory directly, hold an exclusive lock on
//! `core.lock`. The OS releases it when the process exits, even after a crash.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// Lock file name inside the data directory.
pub const LOCK_FILE: &str = "core.lock";

/// Holds the data directory's lock until dropped.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    /// Takes the lock, or fails at once if another process holds it.
    pub fn acquire(data_dir: &Path) -> io::Result<DataDirLock> {
        let path = data_dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(DataDirLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "another kumikit core is using the data directory {}",
                    data_dir.display()
                ),
            )),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = DataDirLock::acquire(dir.path()).unwrap();
        let err = DataDirLock::acquire(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        DataDirLock::acquire(dir.path()).unwrap();
    }
}
