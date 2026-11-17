//! Remote cores over WebSocket and TLS (`docs/spec/02-protocol.md#remote-transport`,
//! [ADR 0017](../../../docs/decisions/0017-shell-remote-profiles-and-ssh.md)).
//!
//! The certificate is pinned: its SHA-256 must equal the profile's fingerprint; names and
//! dates are not checked, and the TLS 1.3 handshake signature is. The shell sends the
//! authenticating first message itself, so tokens never reach the webview.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kumikit_protocol::remote::{RPC_PATH, format_fingerprint, parse_fingerprint, parse_host_port};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::bridge::MAX_LINE;

/// A WebSocket to a remote core.
pub type Ws = WebSocketStream<TlsStream<TcpStream>>;

/// How long connecting, the TLS handshake, the upgrade and the authentication may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a remote connection failed, in a form the UI acts on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum RemoteError {
    /// The address is not `host[:port]`.
    #[error("{0}")]
    BadAddress(String),
    /// Nothing answered, or the connection broke before it was set up.
    #[error("{0}")]
    Unreachable(String),
    /// The core's certificate is not the pinned one.
    #[error(
        "the core's certificate changed (now {0}); remove and re-add the core if you expected this"
    )]
    CertificateChanged(String),
    /// The core refused the code or token.
    #[error("{0}")]
    Unauthorized(String),
    /// The core answered something unexpected.
    #[error("{0}")]
    Protocol(String),
}

impl RemoteError {
    /// The code the webview gets: `unreachable`, `certificate_changed`, `unauthorized`,
    /// `bad_address` or `protocol`.
    pub fn code(&self) -> &'static str {
        match self {
            RemoteError::BadAddress(_) => "bad_address",
            RemoteError::Unreachable(_) => "unreachable",
            RemoteError::CertificateChanged(_) => "certificate_changed",
            RemoteError::Unauthorized(_) => "unauthorized",
            RemoteError::Protocol(_) => "protocol",
        }
    }
}

/// Checks the certificate against a pin, or (when probing) accepts any and records it.
#[derive(Debug)]
struct Verifier {
    pin: Option<[u8; 32]>,
    seen: Arc<Mutex<Option<[u8; 32]>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        *self.seen.lock().unwrap_or_else(|p| p.into_inner()) = Some(got);
        match self.pin {
            Some(pin) if !constant_time_eq(&pin, &got) => {
                Err(rustls::Error::General("certificate changed".into()))
            }
            _ => Ok(ServerCertVerified::assertion()),
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

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// TCP and TLS to `address`. With a pin, a different certificate fails with
/// [`RemoteError::CertificateChanged`]; without one, any certificate is accepted (a
/// probe). Returns the stream and the certificate's SHA-256.
async fn tls(
    address: &str,
    pin: Option<[u8; 32]>,
) -> Result<(TlsStream<TcpStream>, [u8; 32]), RemoteError> {
    let (host, port) = parse_host_port(address)
        .ok_or_else(|| RemoteError::BadAddress(format!("{address} is not host[:port]")))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let seen = Arc::new(Mutex::new(None));
    let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| RemoteError::Protocol(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Verifier {
            pin,
            seen: Arc::clone(&seen),
            provider,
        }))
        .with_no_client_auth();
    let mut config = config;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = ServerName::try_from(host.clone())
        .map_err(|_| RemoteError::BadAddress(format!("{host} is not a host name or address")))?;
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| RemoteError::Unreachable(format!("cannot reach {address}: {e}")))?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await;
    let seen = *seen.lock().unwrap_or_else(|p| p.into_inner());
    match (stream, seen) {
        (Ok(s), Some(sha)) => Ok((s, sha)),
        (Err(_), Some(sha)) if pin.is_some_and(|p| p != sha) => {
            Err(RemoteError::CertificateChanged(format_fingerprint(&sha)))
        }
        (Err(e), _) => Err(RemoteError::Unreachable(format!(
            "TLS with {address} failed: {e}"
        ))),
        (Ok(_), None) => Err(RemoteError::Protocol("the core sent no certificate".into())),
    }
}

/// The certificate fingerprint of the core at `address`. Sends nothing over the
/// connection.
pub async fn probe(address: &str) -> Result<String, RemoteError> {
    let (mut stream, sha) = within(tls(address, None)).await?;
    let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
    Ok(format_fingerprint(&sha))
}

fn pin(fingerprint: &str) -> Result<[u8; 32], RemoteError> {
    parse_fingerprint(fingerprint)
        .ok_or_else(|| RemoteError::Protocol(format!("{fingerprint} is not a SHA-256 fingerprint")))
}

async fn within<T>(f: impl Future<Output = Result<T, RemoteError>>) -> Result<T, RemoteError> {
    tokio::time::timeout(CONNECT_TIMEOUT, f)
        .await
        .map_err(|_| RemoteError::Unreachable("the core did not answer in time".into()))?
}

/// The WebSocket limits: 8 MiB per message and frame, like the core.
pub fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_LINE))
        .max_frame_size(Some(MAX_LINE))
}

/// A pinned WebSocket to `/rpc`, not yet authenticated.
async fn open(address: &str, fingerprint: &str) -> Result<Ws, RemoteError> {
    let (stream, _) = tls(address, Some(pin(fingerprint)?)).await?;
    let (host, port) = parse_host_port(address)
        .ok_or_else(|| RemoteError::BadAddress(format!("{address} is not host[:port]")))?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let url = format!("wss://{host}:{port}{RPC_PATH}");
    let (ws, _) = tokio_tungstenite::client_async_with_config(url, stream, Some(ws_config()))
        .await
        .map_err(|e| RemoteError::Unreachable(format!("the core refused the connection: {e}")))?;
    Ok(ws)
}

/// Sends the first (authenticating) request and reads its answer. Its params are never
/// logged.
async fn first_call(ws: &mut Ws, method: &str, params: Value) -> Result<Value, RemoteError> {
    let request = json!({"jsonrpc": "2.0", "id": 0, "method": method, "params": params});
    ws.send(Message::text(request.to_string()))
        .await
        .map_err(|e| RemoteError::Unreachable(e.to_string()))?;
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(t.as_str())
                    .map_err(|e| RemoteError::Protocol(e.to_string()))?;
                if let Some(e) = v.get("error") {
                    let message = e["message"].as_str().unwrap_or("refused").to_owned();
                    return Err(match e["data"]["code"].as_str() {
                        Some("unauthenticated") => RemoteError::Unauthorized(message),
                        _ => RemoteError::Protocol(message),
                    });
                }
                return Ok(v["result"].clone());
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
            other => {
                return Err(RemoteError::Unreachable(format!(
                    "the core closed the connection ({other:?})"
                )));
            }
        }
    }
}

/// What pairing gives: the token (for the client secrets only) and the device id.
#[derive(Debug)]
pub struct Paired {
    /// The device token.
    pub token: String,
    /// The device id.
    pub device_id: String,
}

/// Pairs with the core at `address`, pinned to `fingerprint`.
pub async fn pair(
    address: &str,
    fingerprint: &str,
    code: &str,
    device_name: &str,
) -> Result<Paired, RemoteError> {
    within(async {
        let mut ws = open(address, fingerprint).await?;
        let result = first_call(
            &mut ws,
            "auth.pair",
            json!({"code": code, "deviceName": device_name}),
        )
        .await?;
        let _ = ws.close(None).await;
        let token = result["token"].as_str();
        let device = result["device"]["id"].as_str();
        match (token, device) {
            (Some(t), Some(d)) => Ok(Paired {
                token: t.to_owned(),
                device_id: d.to_owned(),
            }),
            _ => Err(RemoteError::Protocol(
                "the core's pairing answer is incomplete".into(),
            )),
        }
    })
    .await
}

/// An authenticated WebSocket, ready for the webview's `core.hello`.
pub async fn connect(address: &str, fingerprint: &str, token: &str) -> Result<Ws, RemoteError> {
    within(async {
        let mut ws = open(address, fingerprint).await?;
        first_call(&mut ws, "auth.token", json!({"token": token})).await?;
        Ok(ws)
    })
    .await
}

/// Revokes this client's own device on the core, best effort, within `limit`.
pub async fn revoke_self(
    address: &str,
    fingerprint: &str,
    token: &str,
    device_id: &str,
    limit: Duration,
) -> bool {
    let attempt = async {
        let mut ws = connect(address, fingerprint, token).await.ok()?;
        let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "core.hello",
            "params": {"protocol": kumikit_protocol::PROTOCOL_VERSION.to_string(),
                       "client": {"name": "kumikit-desktop", "version": env!("CARGO_PKG_VERSION")}}});
        ws.send(Message::text(hello.to_string())).await.ok()?;
        let revoke = json!({"jsonrpc": "2.0", "id": 2, "method": "auth.devices.revoke",
            "params": {"device": device_id}});
        ws.send(Message::text(revoke.to_string())).await.ok()?;
        while let Some(Ok(m)) = ws.next().await {
            if let Message::Text(t) = m {
                let v: Value = serde_json::from_str(t.as_str()).ok()?;
                if v["id"] == 2 {
                    return Some(v.get("result").is_some());
                }
            }
        }
        None
    };
    matches!(tokio::time::timeout(limit, attempt).await, Ok(Some(true)))
}
