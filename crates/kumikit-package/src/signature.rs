//! Package signatures, format v1 (`docs/spec/04-plugin-host.md#signatures`,
//! [ADR 0018](../../../docs/decisions/0018-package-signing-and-registry.md)).
//!
//! `SIGNATURE` lists the SHA-256 of every other file in the package and signs that list
//! with the publisher's ed25519 key:
//!
//! ```text
//! kumikit-signature 1
//! key ed25519:<32 bytes, base64url>
//! file <sha256 hex> <path>
//! sig <64 bytes, base64url>
//! ```
//!
//! The signed message is every byte before the `sig` line. Verification requires the
//! files on disk to equal the list exactly.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

use crate::{PackageError, sha256_file};

/// The signature's file name, at the package root.
pub const SIGNATURE_FILE: &str = "SIGNATURE";

/// The first line of a v1 signature.
const HEADER: &str = "kumikit-signature 1";

/// Largest `SIGNATURE` read (a package has at most 10 000 files).
const MAX_SIGNATURE: u64 = 4 << 20;

const PEM_BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_END: &str = "-----END PRIVATE KEY-----";

/// A signature that cannot be trusted: malformed, not matching the files, or not made by
/// its key. The message says which, in words for the user.
fn untrusted(message: impl Into<String>) -> PackageError {
    PackageError::Untrusted(message.into())
}

/// A publisher's public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// Parses a key id, `ed25519:` and the key in base64url (spaces are ignored, so the
    /// grouped form reads back).
    pub fn parse(id: &str) -> Option<PublicKey> {
        let compact: String = id.chars().filter(|c| !c.is_whitespace()).collect();
        let b64 = compact.strip_prefix("ed25519:")?;
        let bytes = URL_SAFE_NO_PAD.decode(b64).ok()?;
        Some(PublicKey(bytes.try_into().ok()?))
    }

    /// The key's raw bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key id in groups of four characters, for reading aloud and comparing by eye.
    pub fn grouped(&self) -> String {
        let id = self.to_string();
        let (prefix, b64) = id.split_at("ed25519:".len());
        let groups: Vec<String> = b64
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect();
        format!("{prefix}{}", groups.join(" "))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ed25519:{}", URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

/// A publisher's private key. Its bytes are never printed: `Debug` shows only the public
/// key.
pub struct SigningKey(Ed25519KeyPair);

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SigningKey({})", self.public())
    }
}

impl SigningKey {
    /// Makes a new key; returns it and its PKCS#8 PEM text, to be written to a 0600 file.
    pub fn generate() -> Result<(SigningKey, String), PackageError> {
        let rng = ring::rand::SystemRandom::new();
        let der = Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| PackageError::Invalid("cannot generate a key".into()))?;
        let pem = pem_encode(der.as_ref());
        Ok((SigningKey::from_pem(&pem)?, pem))
    }

    /// Reads a PKCS#8 PEM private key. The error never contains key material.
    pub fn from_pem(text: &str) -> Result<SigningKey, PackageError> {
        let bad = || PackageError::Invalid("not an ed25519 private key (PKCS#8 PEM)".into());
        let body = text
            .trim()
            .strip_prefix(PEM_BEGIN)
            .and_then(|r| r.strip_suffix(PEM_END))
            .ok_or_else(bad)?;
        let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let der = STANDARD.decode(b64).map_err(|_| bad())?;
        // Accepts v1 documents too (no public key inside), as other tools write them.
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der)
            .map(SigningKey)
            .map_err(|_| bad())
    }

    /// The matching public key.
    pub fn public(&self) -> PublicKey {
        let mut out = [0u8; 32];
        out.copy_from_slice(self.0.public_key().as_ref());
        PublicKey(out)
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.0.sign(message).as_ref().to_vec()
    }
}

fn pem_encode(der: &[u8]) -> String {
    let b64 = STANDARD.encode(der);
    let mut out = String::from(PEM_BEGIN);
    out.push('\n');
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(PEM_END);
    out.push('\n');
    out
}

/// What verifying a package directory found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verified {
    /// There is no `SIGNATURE`.
    Unsigned,
    /// Every file matches and the signature verifies with this key.
    Signed(PublicKey),
}

impl Verified {
    /// The key, if signed.
    pub fn key(&self) -> Option<PublicKey> {
        match self {
            Verified::Unsigned => None,
            Verified::Signed(k) => Some(*k),
        }
    }
}

/// Every regular file under `dir` except the root `SIGNATURE`, by '/'-separated relative
/// path, with its SHA-256. Anything that is not a file or directory is refused.
fn hashes(dir: &Path) -> Result<BTreeMap<String, String>, PackageError> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let path = entry.path();
            if ty.is_dir() {
                stack.push(path);
                continue;
            }
            if !ty.is_file() {
                return Err(untrusted(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| PackageError::Invalid(e.to_string()))?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel == SIGNATURE_FILE {
                continue;
            }
            let hash = sha256_file(&path)?;
            out.insert(rel, hash);
        }
    }
    Ok(out)
}

/// Builds the signature text for the files under `dir` (its own `SIGNATURE`, if any, is
/// left out).
pub fn sign_dir(dir: &Path, key: &SigningKey) -> Result<String, PackageError> {
    let files = hashes(dir)?;
    let mut text = format!("{HEADER}\nkey {}\n", key.public());
    for (path, hash) in &files {
        if !safe_listed_path(path) {
            return Err(PackageError::Invalid(format!(
                "{path} cannot be signed: paths may not contain spaces, newlines or `\\`"
            )));
        }
        text.push_str(&format!("file {hash} {path}\n"));
    }
    let sig = key.sign(text.as_bytes());
    text.push_str(&format!("sig {}\n", URL_SAFE_NO_PAD.encode(sig)));
    Ok(text)
}

/// A path as a `file` line may hold it: relative, '/'-separated, no empty, `.` or `..`
/// segments, no backslash, whitespace or control characters.
fn safe_listed_path(path: &str) -> bool {
    !path.is_empty()
        && path != SIGNATURE_FILE
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && path
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && c != '\\')
}

/// Checks the package unpacked at `dir`: no `SIGNATURE` is [`Verified::Unsigned`]; a
/// signature that is malformed, lists other files or hashes than `dir` holds, or does not
/// verify is [`PackageError::Untrusted`].
pub fn verify_dir(dir: &Path) -> Result<Verified, PackageError> {
    let sig_path = dir.join(SIGNATURE_FILE);
    let meta = match std::fs::symlink_metadata(&sig_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Verified::Unsigned),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() {
        return Err(untrusted("SIGNATURE is not a regular file"));
    }
    if meta.len() > MAX_SIGNATURE {
        return Err(untrusted("SIGNATURE is too large"));
    }
    let text = String::from_utf8(std::fs::read(&sig_path)?)
        .map_err(|_| untrusted("SIGNATURE is not UTF-8 text"))?;
    let parsed = parse(&text)?;
    let files = hashes(dir)?;
    let listed: BTreeMap<String, String> = parsed.files.into_iter().collect();
    for path in files.keys() {
        if !listed.contains_key(path) {
            return Err(untrusted(format!("{path} is not covered by the signature")));
        }
    }
    for (path, hash) in &listed {
        match files.get(path) {
            None => return Err(untrusted(format!("{path} is signed but missing"))),
            Some(h) if h != hash => {
                return Err(untrusted(format!("{path} changed after it was signed")));
            }
            Some(_) => {}
        }
    }
    UnparsedPublicKey::new(&ED25519, parsed.key.bytes())
        .verify(&text.as_bytes()[..parsed.signed_len], &parsed.sig)
        .map_err(|_| untrusted("the signature does not match its key"))?;
    Ok(Verified::Signed(parsed.key))
}

struct Parsed {
    key: PublicKey,
    files: Vec<(String, String)>,
    sig: Vec<u8>,
    /// Length of the signed prefix (everything before the `sig` line).
    signed_len: usize,
}

fn parse(text: &str) -> Result<Parsed, PackageError> {
    let malformed = |why: &str| untrusted(format!("SIGNATURE is malformed: {why}"));
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(malformed("lines must end with a single LF"));
    }
    let mut lines = text[..text.len() - 1].split('\n');
    if lines.next() != Some(HEADER) {
        return Err(malformed("unknown format (expected `kumikit-signature 1`)"));
    }
    let key = lines
        .next()
        .and_then(|l| l.strip_prefix("key "))
        .and_then(PublicKey::parse)
        .ok_or_else(|| malformed("no valid `key ed25519:…` line"))?;
    let mut files: Vec<(String, String)> = Vec::new();
    let mut offset = HEADER.len() + 1 + "key ".len() + key.to_string().len() + 1;
    // The key line must be in its canonical form, or the offset (and the message) differ.
    if !text[HEADER.len() + 1..].starts_with(&format!("key {key}\n")) {
        return Err(malformed("the key is not in canonical form"));
    }
    for line in lines {
        if let Some(sig) = line.strip_prefix("sig ") {
            let sig = URL_SAFE_NO_PAD
                .decode(sig)
                .ok()
                .filter(|s| s.len() == 64)
                .ok_or_else(|| malformed("bad `sig` line"))?;
            if offset + line.len() + 1 != text.len() {
                return Err(malformed("text after the `sig` line"));
            }
            return Ok(Parsed {
                key,
                files,
                sig,
                signed_len: offset,
            });
        }
        let rest = line
            .strip_prefix("file ")
            .ok_or_else(|| malformed("unknown line"))?;
        let (hash, path) = rest
            .split_once(' ')
            .ok_or_else(|| malformed("a `file` line needs a hash and a path"))?;
        if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(malformed("a hash is not 64 lowercase hex digits"));
        }
        if !safe_listed_path(path) {
            return Err(malformed("a path is not allowed in a package"));
        }
        if files.last().is_some_and(|(last, _)| last.as_str() >= path) {
            return Err(malformed("paths must be sorted and unique"));
        }
        files.push((path.to_owned(), hash.to_owned()));
        offset += line.len() + 1;
    }
    Err(malformed("no `sig` line"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, text) in files {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    fn signed(files: &[(&str, &str)]) -> (tempfile::TempDir, SigningKey) {
        let d = dir_with(files);
        let (key, _) = SigningKey::generate().unwrap();
        let text = sign_dir(d.path(), &key).unwrap();
        std::fs::write(d.path().join(SIGNATURE_FILE), text).unwrap();
        (d, key)
    }

    const FILES: &[(&str, &str)] = &[
        ("plugin.toml", "id = \"a.b\""),
        ("plugin.wasm", "\0asm"),
        ("assets/icon.svg", "<svg/>"),
    ];

    fn untrusted_msg(r: Result<Verified, PackageError>) -> String {
        match r {
            Err(PackageError::Untrusted(m)) => m,
            other => panic!("expected untrusted, got {other:?}"),
        }
    }

    fn rewrite(d: &Path, f: impl FnOnce(String) -> String) {
        let p = d.join(SIGNATURE_FILE);
        let text = std::fs::read_to_string(&p).unwrap();
        std::fs::write(p, f(text)).unwrap();
    }

    #[test]
    fn signature_round_trip() {
        let (d, key) = signed(FILES);
        assert_eq!(
            verify_dir(d.path()).unwrap(),
            Verified::Signed(key.public())
        );
        let text = std::fs::read_to_string(d.path().join(SIGNATURE_FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "kumikit-signature 1");
        assert!(lines[2].ends_with(" assets/icon.svg"), "sorted: {text}");
        assert!(lines[4].ends_with(" plugin.wasm"));
        assert!(lines[5].starts_with("sig "));
    }

    #[test]
    fn unsigned_is_not_an_error() {
        let d = dir_with(FILES);
        assert_eq!(verify_dir(d.path()).unwrap(), Verified::Unsigned);
    }

    #[test]
    fn extra_file_refused() {
        let (d, _) = signed(FILES);
        std::fs::write(d.path().join("assets/extra.js"), "evil()").unwrap();
        assert!(untrusted_msg(verify_dir(d.path())).contains("not covered"));
    }

    #[test]
    fn missing_file_refused() {
        let (d, _) = signed(FILES);
        std::fs::remove_file(d.path().join("plugin.wasm")).unwrap();
        assert!(untrusted_msg(verify_dir(d.path())).contains("missing"));
    }

    #[test]
    fn changed_file_refused() {
        let (d, _) = signed(FILES);
        std::fs::write(d.path().join("plugin.toml"), "id = \"x.y\"").unwrap();
        assert!(untrusted_msg(verify_dir(d.path())).contains("changed"));
    }

    #[test]
    fn wrong_key_refused() {
        let (d, _) = signed(FILES);
        let (other, _) = SigningKey::generate().unwrap();
        rewrite(d.path(), |t| {
            let old = t.lines().nth(1).unwrap().to_owned();
            t.replace(&old, &format!("key {}", other.public()))
        });
        assert!(untrusted_msg(verify_dir(d.path())).contains("does not match"));
    }

    #[test]
    fn a_changed_list_breaks_the_signature() {
        // Rewriting a hash to match a changed file is caught by the signature.
        let (d, _) = signed(FILES);
        std::fs::write(d.path().join("plugin.toml"), "id = \"x.y\"").unwrap();
        let new = sha256_file(&d.path().join("plugin.toml")).unwrap();
        rewrite(d.path(), |t| {
            t.lines()
                .map(|l| {
                    if l.ends_with(" plugin.toml") {
                        format!("file {new} plugin.toml")
                    } else {
                        l.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        });
        assert!(untrusted_msg(verify_dir(d.path())).contains("does not match"));
    }

    #[test]
    fn malformed_signature_refused() {
        type Edit = fn(String) -> String;
        let cases: &[(&str, Edit)] = &[
            ("header", |t| {
                t.replacen("kumikit-signature 1", "kumikit-signature 2", 1)
            }),
            ("crlf", |t| t.replace('\n', "\r\n")),
            ("no sig", |t| {
                let i = t.find("sig ").unwrap();
                t[..i].to_owned()
            }),
            ("after sig", |t| t + "file 00 x\n"),
            ("unknown line", |t| t.replacen("file ", "fyle ", 1)),
            ("short sig", |t| {
                let i = t.find("sig ").unwrap();
                format!("{}sig AAAA\n", &t[..i])
            }),
            ("uppercase hash", |t| {
                let line = t.lines().nth(2).unwrap().to_owned();
                t.replace(&line, &line.to_uppercase().replacen("FILE", "file", 1))
            }),
        ];
        for (name, f) in cases {
            let (d, _) = signed(FILES);
            rewrite(d.path(), f);
            match verify_dir(d.path()) {
                Err(PackageError::Untrusted(_)) => {}
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn duplicate_or_unsafe_path_in_list_refused() {
        for bad in [
            "../plugin.toml",
            "/etc/passwd",
            "a//b",
            "./x",
            "a\\b",
            "SIGNATURE",
        ] {
            let (d, _) = signed(FILES);
            rewrite(d.path(), |t| {
                let line = t.lines().nth(2).unwrap().to_owned();
                let hash = line.split(' ').nth(1).unwrap().to_owned();
                t.replace(&line, &format!("file {hash} {bad}"))
            });
            assert!(
                untrusted_msg(verify_dir(d.path())).contains("malformed"),
                "{bad}"
            );
        }
        let (d, _) = signed(FILES);
        rewrite(d.path(), |t| {
            let line = t.lines().nth(2).unwrap().to_owned();
            t.replace(&line, &format!("{line}\n{line}"))
        });
        assert!(untrusted_msg(verify_dir(d.path())).contains("sorted and unique"));
    }

    #[test]
    fn keys_round_trip_through_pem_and_ids() {
        let (key, pem) = SigningKey::generate().unwrap();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        let again = SigningKey::from_pem(&pem).unwrap();
        assert_eq!(again.public(), key.public());
        let id = key.public().to_string();
        assert_eq!(id.len(), "ed25519:".len() + 43);
        assert_eq!(PublicKey::parse(&id), Some(key.public()));
        assert_eq!(
            PublicKey::parse(&key.public().grouped()),
            Some(key.public())
        );
        assert_eq!(PublicKey::parse("ed25519:short"), None);
        assert_eq!(PublicKey::parse("rsa:AAAA"), None);
        // Debug output never holds the private key.
        let debug = format!("{key:?}");
        assert!(debug.contains(&id) && !debug.contains(pem.lines().nth(1).unwrap()));
        assert!(SigningKey::from_pem("not a key").is_err());
    }
}
