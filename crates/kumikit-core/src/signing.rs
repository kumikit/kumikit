//! Publisher keys and package signatures from the CLI: `kumikit plugin keygen`, `pack
//! --key`, `sign` and `verify` (`docs/spec/10-distribution.md#publisher-keys`).
//!
//! Private keys are never printed or logged; errors name only the file. Key files are
//! written 0600 and kept out of git work trees.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use kumikit_plugin_host::package::{self, PublicKey, SigningKey, Verified};

use crate::cli::Out;
use crate::paths::{Env, Platform};

/// The environment variable that may hold a private key (PEM), for a publisher's CI.
pub const KEY_ENV: &str = "KUMIKIT_SIGNING_KEY";

/// Where `keygen` puts keys by default: `<user config dir>/kumikit/keys`.
pub fn keys_dir(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    match platform {
        Platform::Linux => env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("kumikit").join("keys")),
        Platform::MacOs => env("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join("dev.kumikit")
                .join("keys")
        }),
        Platform::Windows => env("APPDATA").map(|d| PathBuf::from(d).join("kumikit").join("keys")),
    }
}

/// The git work tree `path` would be inside, if any (a `.git` in it or a parent).
fn git_work_tree(path: &Path) -> Option<PathBuf> {
    let start = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    start
        .ancestors()
        .skip(1)
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Writes `text` to a new file only the user can read. An existing file is never
/// overwritten.
fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow!(
                "{} already exists; Kumikit never overwrites a key",
                path.display()
            )
        } else {
            anyhow!("cannot write {}: {e}", path.display())
        }
    })?;
    std::io::Write::write_all(&mut file, text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// `kumikit plugin keygen [--name <name>] [--out <file>] [--force]`.
pub fn keygen(
    name: &str,
    out_path: Option<&Path>,
    force: bool,
    platform: Platform,
    env: Env<'_>,
    out: &mut dyn Out,
) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("the key name must be lowercase letters, digits and `-`");
    }
    let path = match out_path {
        Some(p) => p.to_path_buf(),
        None => keys_dir(platform, env)
            .ok_or_else(|| anyhow!("cannot find a config directory; pass --out <file>"))?
            .join(format!("{name}.key")),
    };
    if let Some(tree) = git_work_tree(&path)
        && !force
    {
        bail!(
            "{} is inside the git work tree {}; a private key must never be committed. \
             Choose another place, or pass --force if you are sure",
            path.display(),
            tree.display()
        );
    }
    let (key, pem) = SigningKey::generate()?;
    write_private(&path, &pem)?;
    out.line(&format!(
        "Wrote the private key to {} (only you can read it).",
        path.display()
    ));
    out.line("Keep it secret and backed up: anyone with it can publish as you, and a lost key cannot be replaced.");
    out.line(&format!("Public key: {}", key.public()));
    Ok(())
}

/// Reads the private key from `--key <file>` or from `KUMIKIT_SIGNING_KEY`.
pub fn read_key(file: Option<&Path>, env: Env<'_>) -> Result<SigningKey> {
    let text = match file {
        Some(path) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path)
                    .with_context(|| format!("cannot read {}", path.display()))?
                    .permissions()
                    .mode();
                if mode & 0o077 != 0 {
                    bail!(
                        "{} can be read by other users; run `chmod 600 {}` first",
                        path.display(),
                        path.display()
                    );
                }
            }
            std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?
        }
        None => env(KEY_ENV).ok_or_else(|| anyhow!("pass --key <file> or set {KEY_ENV}"))?,
    };
    SigningKey::from_pem(&text).map_err(|_| match file {
        Some(p) => anyhow!("{} is not an ed25519 private key", p.display()),
        None => anyhow!("{KEY_ENV} does not hold an ed25519 private key"),
    })
}

/// `kumikit plugin sign <file.kkp> --key <file>`.
pub fn sign(package: &Path, key: &SigningKey, out: &mut dyn Out) -> Result<()> {
    package::sign_package(package, package, key)
        .with_context(|| format!("cannot sign {}", package.display()))?;
    out.line(&format!(
        "Signed {} with {}",
        package.display(),
        key.public()
    ));
    Ok(())
}

/// `kumikit plugin verify <file.kkp> [--key <id>]`.
pub fn verify(package: &Path, expected: Option<&str>, out: &mut dyn Out) -> Result<()> {
    let expected = expected
        .map(|id| PublicKey::parse(id).ok_or_else(|| anyhow!("{id} is not a key id (ed25519:…)")))
        .transpose()?;
    let verified = package::verify_package(package).map_err(|e| anyhow!("{e}"))?;
    match (verified, expected) {
        (Verified::Signed(k), Some(want)) if k != want => {
            bail!("signed by {k}, not by {want}")
        }
        (Verified::Signed(k), _) => out.line(&format!("Signed by {k}; every file matches.")),
        (Verified::Unsigned, Some(want)) => bail!("not signed (expected {want})"),
        (Verified::Unsigned, None) => out.line("Not signed."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct Lines(Vec<String>);
    impl Out for Lines {
        fn line(&mut self, text: &str) {
            self.0.push(text.to_owned());
        }
    }

    fn env(map: &HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> {
        let owned: HashMap<String, String> = map
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| owned.get(k).cloned()
    }

    #[test]
    fn keys_dir_per_platform() {
        let e = HashMap::from([("HOME", "/h"), ("APPDATA", "C:\\A")]);
        let f = env(&e);
        assert_eq!(
            keys_dir(Platform::Linux, &f),
            Some(PathBuf::from("/h/.config/kumikit/keys"))
        );
        assert_eq!(
            keys_dir(Platform::MacOs, &f),
            Some(PathBuf::from(
                "/h/Library/Application Support/dev.kumikit/keys"
            ))
        );
        assert_eq!(
            keys_dir(Platform::Windows, &f),
            Some(PathBuf::from("C:\\A").join("kumikit").join("keys"))
        );
        let x = HashMap::from([("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(
            keys_dir(Platform::Linux, &env(&x)),
            Some(PathBuf::from("/x/kumikit/keys"))
        );
    }

    #[test]
    fn keygen_writes_0600_outside_repo_and_never_prints_the_key() {
        let home = tempfile::tempdir().unwrap();
        let e = HashMap::from([("XDG_CONFIG_HOME", home.path().to_str().unwrap())]);
        let mut out = Lines(Vec::new());
        keygen("acme", None, false, Platform::Linux, &env(&e), &mut out).unwrap();
        let path = home.path().join("kumikit/keys/acme.key");
        let pem = std::fs::read_to_string(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let printed = out.0.join("\n");
        for line in pem.lines().filter(|l| !l.starts_with("-----")) {
            assert!(!printed.contains(line), "the private key was printed");
        }
        let key = read_key(Some(&path), &env(&HashMap::new())).unwrap();
        assert!(printed.contains(&key.public().to_string()));
        // Never overwritten.
        let err = keygen("acme", None, false, Platform::Linux, &env(&e), &mut out).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), pem);
    }

    #[test]
    fn keygen_refuses_git_work_tree() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let target = repo.path().join("keys").join("acme.key");
        let mut out = Lines(Vec::new());
        let err = keygen(
            "acme",
            Some(&target),
            false,
            Platform::Linux,
            &env(&HashMap::new()),
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("git work tree"), "{err}");
        assert!(!target.exists());
        keygen(
            "acme",
            Some(&target),
            true,
            Platform::Linux,
            &env(&HashMap::new()),
            &mut out,
        )
        .unwrap();
        assert!(target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_key_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.key");
        let (_, pem) = SigningKey::generate().unwrap();
        std::fs::write(&path, pem).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let err = read_key(Some(&path), &env(&HashMap::new())).unwrap_err();
        assert!(err.to_string().contains("chmod 600"), "{err}");
    }

    #[test]
    fn the_key_can_come_from_the_environment_and_errors_hide_it() {
        let (key, pem) = SigningKey::generate().unwrap();
        let e = HashMap::from([(KEY_ENV, pem.as_str())]);
        assert_eq!(read_key(None, &env(&e)).unwrap().public(), key.public());
        let bad = HashMap::from([(
            KEY_ENV,
            "-----BEGIN PRIVATE KEY-----\nc2VjcmV0\n-----END PRIVATE KEY-----",
        )]);
        let err = read_key(None, &env(&bad)).unwrap_err().to_string();
        assert!(!err.contains("c2VjcmV0"), "{err}");
        assert!(read_key(None, &env(&HashMap::new())).is_err());
    }

    #[test]
    fn sign_and_verify_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("plugin.toml"), "id = \"a.b\"").unwrap();
        let kkp = dir.path().join("a.kkp");
        package::pack(&src, &kkp).unwrap();
        let mut out = Lines(Vec::new());
        verify(&kkp, None, &mut out).unwrap();
        assert_eq!(out.0.last().unwrap(), "Not signed.");
        let (key, _) = SigningKey::generate().unwrap();
        sign(&kkp, &key, &mut out).unwrap();
        verify(&kkp, Some(&key.public().to_string()), &mut out).unwrap();
        let (other, _) = SigningKey::generate().unwrap();
        let err = verify(&kkp, Some(&other.public().to_string()), &mut out).unwrap_err();
        assert!(err.to_string().contains("not by"), "{err}");
    }
}
