//! The core's TLS certificate (`docs/spec/02-protocol.md#remote-transport`): the user's,
//! or a self-signed one generated once into `<core data>/tls/`. Clients pin its SHA-256.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kumikit_protocol::remote::format_fingerprint;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

/// Where the certificate comes from.
#[derive(Debug, Clone)]
pub enum CertSource {
    /// `<data dir>/tls/cert.pem` and `key.pem`, generated if missing.
    Generated(PathBuf),
    /// Files the user named (`--tls-cert`, `--tls-key`).
    Given {
        /// PEM certificate chain, leaf first.
        cert: PathBuf,
        /// PEM private key.
        key: PathBuf,
    },
}

/// A loaded certificate and the server configuration built from it.
#[derive(Debug)]
pub struct Tls {
    /// The rustls server configuration (TLS 1.3 only).
    pub config: Arc<rustls::ServerConfig>,
    /// The leaf certificate's fingerprint, as clients show it.
    pub fingerprint: String,
}

/// The crypto provider every TLS use in the core shares.
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Loads (or first generates) the certificate and builds the server configuration.
pub fn load(source: &CertSource, host_name: &str) -> io::Result<Tls> {
    let (cert_path, key_path) = match source {
        CertSource::Generated(data_dir) => {
            let dir = data_dir.join("tls");
            let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
            if !cert.exists() || !key.exists() {
                generate(&dir, &cert, &key, host_name)?;
            }
            (cert, key)
        }
        CertSource::Given { cert, key } => (cert.clone(), key.clone()),
    };
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_path)
        .and_then(|it| it.collect::<Result<_, _>>())
        .map_err(|e| bad(&cert_path, e))?;
    let leaf = chain
        .first()
        .ok_or_else(|| bad(&cert_path, "no certificate in the file"))?;
    let fingerprint = format_fingerprint(&Sha256::digest(leaf.as_ref()).into());
    let key = PrivateKeyDer::from_pem_file(&key_path).map_err(|e| bad(&key_path, e))?;
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| bad(&cert_path, e))?;
    let mut config = config;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Tls {
        config: Arc::new(config),
        fingerprint,
    })
}

fn bad(path: &Path, e: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot use {}: {e}", path.display()),
    )
}

/// Generates a self-signed ECDSA P-256 certificate valid for ten years. Clients pin it,
/// so its names and dates are not what they check.
fn generate(dir: &Path, cert: &Path, key: &Path, host_name: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let pair =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(io::Error::other)?;
    let mut names = vec!["localhost".to_owned()];
    if !host_name.is_empty() && host_name != "localhost" {
        names.insert(0, host_name.to_owned());
    }
    let mut params = rcgen::CertificateParams::new(names).map_err(io::Error::other)?;
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("Kumikit core on {host_name}"),
    );
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(3650);
    let certificate = params.self_signed(&pair).map_err(io::Error::other)?;
    write_private(key, pair.serialize_pem().as_bytes())?;
    write_private(cert, certificate.pem().as_bytes())
}

/// Writes a file only this user can read (0600 on Unix), replacing it atomically.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_once_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let source = CertSource::Generated(dir.path().to_owned());
        let a = load(&source, "devbox").unwrap();
        let b = load(&source, "devbox").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert!(a.fingerprint.starts_with("SHA256:"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.path().join("tls")), 0o700);
            assert_eq!(mode(&dir.path().join("tls/key.pem")), 0o600);
        }
        // Deleting the files makes a new certificate.
        std::fs::remove_dir_all(dir.path().join("tls")).unwrap();
        let c = load(&source, "devbox").unwrap();
        assert_ne!(a.fingerprint, c.fingerprint);
    }

    #[test]
    fn given_files_are_used_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let generated = CertSource::Generated(dir.path().to_owned());
        let fp = load(&generated, "x").unwrap().fingerprint;
        let given = CertSource::Given {
            cert: dir.path().join("tls/cert.pem"),
            key: dir.path().join("tls/key.pem"),
        };
        assert_eq!(load(&given, "x").unwrap().fingerprint, fp);
        let broken = CertSource::Given {
            cert: dir.path().join("tls/key.pem"),
            key: dir.path().join("tls/key.pem"),
        };
        let err = load(&broken, "x").unwrap_err();
        assert!(err.to_string().contains("key.pem"), "{err}");
    }
}
