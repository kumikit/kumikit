//! M3 step 2 spike: rustls (ring, TLS 1.3) with an rcgen certificate, and a WebSocket over
//! it, on loopback, with the client pinning the certificate's SHA-256. Replaced by the real
//! remote transport tests in step 6.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
struct Pinned {
    sha256: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if got == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate changed".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is not used".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn certificate() -> (CertificateDer<'static>, rcgen::KeyPair) {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert.der().clone(), key)
}

async fn round_trip(
    der: CertificateDer<'static>,
    key: rcgen::KeyPair,
    pin: [u8; 32],
) -> Result<String, String> {
    let server_cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![der.clone()],
            PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let Ok(tls) = acceptor.accept(tcp).await else {
            return;
        };
        let mut ws = tokio_tungstenite::accept_async(tls).await.unwrap();
        use futures_util::{SinkExt, StreamExt};
        if let Some(Ok(Message::Text(t))) = ws.next().await {
            ws.send(Message::text(format!("echo {t}"))).await.unwrap();
        }
    });

    let client_cfg = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            sha256: pin,
            provider: provider(),
        }))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_cfg));
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let tls = connector
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .map_err(|e| e.to_string())?;
    let (mut ws, _) = tokio_tungstenite::client_async(format!("wss://{addr}/rpc"), tls)
        .await
        .map_err(|e| e.to_string())?;
    use futures_util::{SinkExt, StreamExt};
    ws.send(Message::text("hi")).await.unwrap();
    match ws.next().await {
        Some(Ok(Message::Text(t))) => Ok(t.to_string()),
        other => Err(format!("{other:?}")),
    }
}

#[tokio::test]
async fn pinned_tls13_websocket_round_trip() {
    let (der, key) = certificate();
    let pin: [u8; 32] = Sha256::digest(der.as_ref()).into();
    assert_eq!(round_trip(der, key, pin).await.unwrap(), "echo hi");
    let (der, key) = certificate();
    let err = round_trip(der, key, pin).await.unwrap_err();
    assert!(err.contains("certificate changed"), "{err}");
}
