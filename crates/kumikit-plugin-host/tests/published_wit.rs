#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
//! Published WIT versions never change (`wit/PUBLISHED`), and every package directory is
//! named after the package it declares.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn wit_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit")
}

/// SHA-256 over every file in `dir`, in path order, with line endings normalized to `\n`
/// (a Windows checkout may turn them into `\r\n`).
/// Every file under `dir`, `deps/` included, as (path relative to `dir` with `/`, path).
fn files_under(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    for e in fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        let name = format!("{prefix}{}", path.file_name().unwrap().to_string_lossy());
        if path.is_dir() {
            files_under(&path, &format!("{name}/"), out);
        } else {
            out.push((name, path));
        }
    }
}

fn dir_hash(dir: &Path) -> String {
    let mut files = Vec::new();
    files_under(dir, "", &mut files);
    files.sort();
    let mut h = Sha256::new();
    for (name, f) in files {
        let text = fs::read_to_string(&f).unwrap().replace("\r\n", "\n");
        h.update(name.as_bytes());
        h.update([0]);
        h.update(text.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn package_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(wit_root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn published_packages_are_unchanged() {
    let listed = fs::read_to_string(wit_root().join("PUBLISHED")).unwrap();
    let mut current = String::new();
    for dir in package_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        current.push_str(&format!("{name} {}\n", dir_hash(&dir)));
    }
    eprintln!("current WIT hashes:\n{current}");
    for line in listed.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (dir, hash) = line.split_once(' ').expect("`<directory> <sha256>`");
        let path = wit_root().join(dir);
        assert!(path.is_dir(), "published package {dir} is missing");
        assert_eq!(
            dir_hash(&path),
            hash.trim(),
            "published package {dir} changed; add a new version instead"
        );
    }
}

#[test]
fn directories_match_their_package() {
    for dir in package_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let (pkg, version) = name.split_once('@').expect("<publisher>-<name>@<version>");
        let (ns, pkg) = pkg.split_once('-').expect("<namespace>-<name>");
        let expected = format!("package {ns}:{pkg}@{version};");
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            // `deps/` holds other packages this one uses (wasi:io).
            if path.is_dir() {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                text.lines().any(|l| l.trim() == expected),
                "{} does not declare `{expected}`",
                path.display()
            );
        }
    }
}

#[test]
fn hash_sees_a_change() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.wit"), "package a:b@0.1.0;\n").unwrap();
    let before = dir_hash(dir.path());
    fs::write(dir.path().join("a.wit"), "package a:b@0.1.0;\r\n").unwrap();
    assert_eq!(dir_hash(dir.path()), before, "line endings are normalized");
    fs::write(dir.path().join("a.wit"), "package a:b@0.1.1;\n").unwrap();
    assert_ne!(dir_hash(dir.path()), before);
}
