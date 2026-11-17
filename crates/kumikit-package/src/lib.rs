//! `.kkp` packages: a tar archive compressed with zstd
//! (`docs/spec/04-plugin-host.md#package-contents`).
//!
//! Unpacking trusts nothing in the archive: only regular files and directories, only
//! relative paths without `..`, and caps on the total size and number of entries, so a
//! package cannot write outside its directory or fill the disk.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// Largest unpacked package.
pub const MAX_UNPACKED: u64 = 256 << 20;
/// Most entries in a package.
pub const MAX_ENTRIES: usize = 10_000;

/// A package that could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// The archive breaks a rule.
    #[error("invalid package: {0}")]
    Invalid(String),
    /// Reading or writing failed.
    #[error("package I/O: {0}")]
    Io(#[from] io::Error),
}

/// SHA-256 of a file, hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Packs the files under `dir` into a `.kkp` at `out`. Paths are stored relative to
/// `dir`, sorted, with fixed metadata, so the same input gives the same bytes.
pub fn pack(dir: &Path, out: &Path) -> Result<(), PackageError> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let file = File::create(out)?;
    let encoder = zstd::Encoder::new(file, 19)?;
    let mut tar = tar::Builder::new(encoder);
    for rel in files {
        let data = std::fs::read(dir.join(&rel))?;
        let mut header = tar::Header::new_ustar();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        // Archive paths always use '/'.
        let name = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        tar.append_data(&mut header, name, data.as_slice())?;
    }
    let encoder = tar.into_inner()?;
    encoder.finish()?.flush()?;
    Ok(())
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), PackageError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = entry.path();
        if ty.is_dir() {
            collect(root, &path, out)?;
        } else if ty.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| PackageError::Invalid(e.to_string()))?;
            out.push(rel.to_path_buf());
        } else {
            return Err(PackageError::Invalid(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Unpacks the `.kkp` at `package` into the empty directory `dest`.
pub fn unpack(package: &Path, dest: &Path) -> Result<(), PackageError> {
    let file = File::open(package)?;
    let decoder = zstd::Decoder::new(file)
        .map_err(|e| PackageError::Invalid(format!("not a zstd archive: {e}")))?;
    let mut archive = tar::Archive::new(decoder);
    let mut total: u64 = 0;
    let entries = archive
        .entries()
        .map_err(|e| PackageError::Invalid(format!("not a tar archive: {e}")))?;
    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return Err(PackageError::Invalid(format!(
                "more than {MAX_ENTRIES} entries"
            )));
        }
        let mut entry = entry.map_err(|e| PackageError::Invalid(format!("bad entry: {e}")))?;
        let raw = entry
            .path()
            .map_err(|e| PackageError::Invalid(format!("bad path: {e}")))?
            .into_owned();
        let rel = safe_path(&raw)?;
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(dest.join(&rel))?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                let size = entry
                    .header()
                    .size()
                    .map_err(|e| PackageError::Invalid(e.to_string()))?;
                total = total.saturating_add(size);
                if total > MAX_UNPACKED {
                    return Err(PackageError::Invalid(format!(
                        "unpacked size over {} MiB",
                        MAX_UNPACKED >> 20
                    )));
                }
                let target = dest.join(&rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                // Copy at most the declared size, so a lying header cannot overrun the cap.
                let copied = io::copy(&mut (&mut entry).take(size), &mut out)?;
                if copied != size {
                    return Err(PackageError::Invalid(format!(
                        "{} is truncated",
                        raw.display()
                    )));
                }
            }
            other => {
                return Err(PackageError::Invalid(format!(
                    "{} is a {other:?} entry; only files and directories are allowed",
                    raw.display()
                )));
            }
        }
    }
    Ok(())
}

/// A relative path made only of normal components.
fn safe_path(raw: &Path) -> Result<PathBuf, PackageError> {
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(PackageError::Invalid(format!(
                    "path {} leaves the package",
                    raw.display()
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(PackageError::Invalid("empty path".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_zst(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> tempfile::NamedTempFile {
        let mut b = tar::Builder::new(Vec::new());
        build(&mut b);
        let tar = b.into_inner().unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut enc = zstd::Encoder::new(file.reopen().unwrap(), 3).unwrap();
        enc.write_all(&tar).unwrap();
        enc.finish().unwrap();
        file
    }

    /// A header with a raw name, bypassing the builder's own path checks.
    fn raw_entry(b: &mut tar::Builder<Vec<u8>>, name: &[u8], ty: tar::EntryType, data: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.as_old_mut().name[..name.len()].copy_from_slice(name);
        h.set_entry_type(ty);
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append(&h, data).unwrap();
    }

    #[test]
    fn pack_and_unpack_round_trip() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("plugin.toml"), "x").unwrap();
        std::fs::create_dir(src.path().join("assets")).unwrap();
        std::fs::write(src.path().join("assets/README"), "hi").unwrap();
        let out = tempfile::tempdir().unwrap();
        let kkp = out.path().join("p.kkp");
        pack(src.path(), &kkp).unwrap();
        let again = out.path().join("q.kkp");
        pack(src.path(), &again).unwrap();
        assert_eq!(
            sha256_file(&kkp).unwrap(),
            sha256_file(&again).unwrap(),
            "reproducible"
        );

        let dest = tempfile::tempdir().unwrap();
        unpack(&kkp, dest.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.path().join("assets/README")).unwrap(),
            "hi"
        );
    }

    fn refused(file: &tempfile::NamedTempFile) -> String {
        let dest = tempfile::tempdir().unwrap();
        match unpack(file.path(), dest.path()) {
            Err(PackageError::Invalid(m)) => m,
            other => panic!("expected an invalid package, got {other:?}"),
        }
    }

    #[test]
    fn traversal_and_absolute_paths_are_refused() {
        for name in [&b"../evil"[..], b"/etc/evil", b"a/../../evil"] {
            let f = tar_zst(|b| raw_entry(b, name, tar::EntryType::Regular, b"x"));
            assert!(refused(&f).contains("leaves the package"));
        }
    }

    #[test]
    fn links_and_devices_are_refused() {
        for ty in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Char,
        ] {
            let f = tar_zst(|b| raw_entry(b, b"x", ty, b""));
            assert!(refused(&f).contains("only files and directories"));
        }
    }

    #[test]
    fn oversize_and_garbage_are_refused() {
        let f = tar_zst(|b| {
            let mut h = tar::Header::new_gnu();
            h.set_path("big").unwrap();
            h.set_size(MAX_UNPACKED + 1);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            // The header claims more than it carries; the cap is checked first.
            b.append(&h, &b""[..]).ok();
        });
        assert!(refused(&f).contains("unpacked size"));

        let garbage = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(garbage.path(), b"not a package").unwrap();
        refused(&garbage);
    }

    #[test]
    fn a_file_cannot_be_written_twice() {
        let f = tar_zst(|b| {
            raw_entry(b, b"a", tar::EntryType::Regular, b"1");
            raw_entry(b, b"a", tar::EntryType::Regular, b"2");
        });
        let dest = tempfile::tempdir().unwrap();
        assert!(unpack(f.path(), dest.path()).is_err());
    }
}
