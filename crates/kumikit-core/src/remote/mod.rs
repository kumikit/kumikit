//! The remote transport (`docs/spec/02-protocol.md#remote-transport`,
//! [ADR 0016](../../../../docs/decisions/0016-remote-transport-and-pairing.md)): TLS 1.3,
//! HTTP/1.1, and `/rpc` upgraded to a WebSocket. Off unless `serve --listen` is given.
//!
//! A connection reaches the JSON-RPC server only after it authenticated: the gate below
//! reads the first message itself and builds no server state before it succeeds.

pub mod pairing;
pub mod tls;
mod web;
pub mod ws;

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use kumikit_kernel::Kernel;
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_protocol::jsonrpc::{RequestId, Response as RpcResponse};
use kumikit_protocol::methods::{
    self as m, AuthPairParams, AuthPairResult, AuthTokenParams, AuthTokenResult, Method,
    PairingCode,
};
use kumikit_protocol::remote::{CLOSE_REVOKED, RPC_PATH, format_code, valid_device_name};
use kumikit_protocol::{ErrorCode, RpcError};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};
use tokio_tungstenite::tungstenite::{Message, handshake::derive_accept_key};

use crate::messages::Close;
use crate::rpc::{self, Peer, ServerInfo};
use pairing::{Pairing, Refused};

/// Time limits of the remote transport. Tests shorten them through hidden `serve` flags.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// From the TCP accept to the end of the first (authenticating) message.
    pub pre_auth: Duration,
    /// Most connections that may be unauthenticated at once.
    pub max_pre_auth: usize,
    /// Interval of the core's pings.
    pub ping: Duration,
    /// Silence after which a client is treated as gone.
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            pre_auth: Duration::from_secs(10),
            max_pre_auth: 32,
            ping: Duration::from_secs(20),
            idle: Duration::from_secs(60),
        }
    }
}

/// Remote access of a running core.
#[derive(Debug)]
pub struct Remote {
    /// The certificate's fingerprint (`SHA256:…`).
    pub fingerprint: String,
    /// Where clients connect: `wss://<host>:<port>/rpc`.
    pub url: String,
    /// The address the listener is bound to.
    pub bound: SocketAddr,
    /// Time limits.
    pub limits: Limits,
    /// The browser client's files (`--web`), when the core serves it.
    pub web: Option<std::path::PathBuf>,
    pairing: Mutex<Pairing>,
}

impl Remote {
    /// A new pairing code, replacing any live one.
    pub fn create_code(&self) -> io::Result<PairingCode> {
        let code = self
            .pairing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .create(Instant::now())?;
        let expires = time::OffsetDateTime::now_utc()
            + time::Duration::try_from(pairing::CODE_TTL).unwrap_or(time::Duration::MINUTE);
        Ok(PairingCode {
            code: format_code(&code),
            expires: kumikit_kernel::clock::format_time(expires),
            fingerprint: self.fingerprint.clone(),
            url: self.url.clone(),
        })
    }

    fn redeem(&self, code: &str) -> Result<(), Refused> {
        self.pairing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .redeem(code, Instant::now())
    }
}

/// Binds the listener for `--listen <value>`.
pub async fn bind(value: &str) -> io::Result<TcpListener> {
    let bad = |e: &dyn std::fmt::Display| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot listen on {value}: {e}"),
        )
    };
    let (host, port) = kumikit_protocol::remote::parse_host_port(value)
        .ok_or_else(|| bad(&"expected host[:port]"))?;
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| bad(&e))?
        .next()
        .ok_or_else(|| bad(&"the name has no address"))?;
    TcpListener::bind(addr)
        .await
        .map_err(|e| io::Error::new(e.kind(), format!("cannot listen on {value}: {e}")))
}

impl Remote {
    /// Describes a bound listener.
    pub fn new(bound: SocketAddr, fingerprint: String, host_name: &str, limits: Limits) -> Self {
        let url = if bound.ip().is_unspecified() {
            format!("wss://{host_name}:{}{RPC_PATH}", bound.port())
        } else {
            format!("wss://{bound}{RPC_PATH}")
        };
        Remote {
            fingerprint,
            url,
            bound,
            limits,
            web: None,
            pairing: Mutex::new(Pairing::default()),
        }
    }
}

/// What every connection of the listener shares.
struct Shared {
    host: PluginHost,
    info: Arc<ServerInfo>,
    remote: Arc<Remote>,
}

/// Accepts remote connections until the task is aborted.
pub async fn run(
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    host: PluginHost,
    info: Arc<ServerInfo>,
    remote: Arc<Remote>,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    let pre_auth = Arc::new(Semaphore::new(remote.limits.max_pre_auth));
    let shared = Arc::new(Shared { host, info, remote });
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "remote accept failed");
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&pre_auth).try_acquire_owned() else {
            tracing::debug!(%peer, "too many unauthenticated connections; closing");
            drop(tcp);
            continue;
        };
        let (acceptor, shared) = (acceptor.clone(), Arc::clone(&shared));
        tokio::spawn(async move {
            let deadline = Instant::now() + shared.remote.limits.pre_auth;
            let tls = match tokio::time::timeout_at(deadline, acceptor.accept(tcp)).await {
                Ok(Ok(tls)) => tls,
                Ok(Err(e)) => {
                    tracing::debug!(%peer, error = %e, "TLS handshake failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(%peer, "TLS handshake timed out");
                    return;
                }
            };
            let permit = Arc::new(Mutex::new(Some(permit)));
            let service = hyper::service::service_fn(move |req| {
                let (shared, permit) = (Arc::clone(&shared), Arc::clone(&permit));
                async move {
                    Ok::<_, std::convert::Infallible>(route(req, shared, permit, deadline).await)
                }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
                .serve_connection(TokioIo::new(tls), service)
                .with_upgrades();
            if let Err(e) = conn.await {
                tracing::debug!(%peer, error = %e, "remote HTTP connection ended");
            }
        });
    }
}

type Body = Full<Bytes>;

fn status(code: StatusCode, text: &'static str) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from_static(text.as_bytes())));
    *r.status_mut() = code;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r
}

async fn route(
    req: Request<Incoming>,
    shared: Arc<Shared>,
    permit: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    deadline: Instant,
) -> Response<Body> {
    if req.uri().path() == RPC_PATH {
        // Only browsers send `Origin`: they authenticate with the cookie set at pairing,
        // and only when this core serves the web UI.
        if req.headers().contains_key(header::ORIGIN) {
            return web::browser_upgrade(req, shared, permit, deadline).await;
        }
        return upgrade(req, shared, permit, deadline, None);
    }
    // Not a WebSocket: this connection no longer takes an unauthenticated place.
    drop(permit.lock().unwrap_or_else(|p| p.into_inner()).take());
    match shared.remote.web.clone() {
        Some(dir) => web::serve(req, &shared, &dir).await,
        None => status(StatusCode::NOT_FOUND, "not found\n"),
    }
}

fn header_has(req: &Request<Incoming>, name: header::HeaderName, token: &str) -> bool {
    req.headers()
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case(token))
}

/// Answers a WebSocket upgrade on `/rpc` and starts the connection once upgraded.
/// A device already known when the upgrade is answered (a browser's cookie), with the
/// revocations it was subscribed to before the cookie was checked.
type Preauthenticated = (String, broadcast::Receiver<String>);

fn upgrade(
    mut req: Request<Incoming>,
    shared: Arc<Shared>,
    permit: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    deadline: Instant,
    pre: Option<Preauthenticated>,
) -> Response<Body> {
    let key = req.headers().get(header::SEC_WEBSOCKET_KEY).cloned();
    let ok = req.method() == hyper::Method::GET
        && header_has(&req, header::UPGRADE, "websocket")
        && header_has(&req, header::CONNECTION, "upgrade")
        && req
            .headers()
            .get(header::SEC_WEBSOCKET_VERSION)
            .is_some_and(|v| v == "13");
    let (Some(key), true) = (key, ok) else {
        return status(StatusCode::BAD_REQUEST, "expected a WebSocket upgrade\n");
    };
    let Some(permit) = permit.lock().unwrap_or_else(|p| p.into_inner()).take() else {
        return status(StatusCode::BAD_REQUEST, "one WebSocket per connection\n");
    };
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(u) => u,
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket upgrade failed");
                return;
            }
        };
        let ws = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(ws::config()),
        )
        .await;
        connection(ws, shared, permit, deadline, pre).await;
    });
    let mut r = Response::new(Full::new(Bytes::new()));
    *r.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let h = r.headers_mut();
    h.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    h.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    if let Ok(accept) = HeaderValue::from_str(&derive_accept_key(key.as_bytes())) {
        h.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
    }
    r
}

/// One WebSocket: the gate, then the JSON-RPC server as the authenticated device.
async fn connection<S>(
    mut ws: WebSocketStream<S>,
    shared: Arc<Shared>,
    permit: OwnedSemaphorePermit,
    deadline: Instant,
    pre: Option<Preauthenticated>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let kernel = shared.host.manager().kernel().clone();
    if let Some((device, revocations)) = pre {
        drop(permit);
        return serve(ws, &shared, kernel, device, revocations).await;
    }
    // Subscribed before the token is checked, so a revocation in between is not missed.
    let revocations = kernel.device_revocations();
    let device =
        match tokio::time::timeout_at(deadline, gate(&mut ws, &kernel, &shared.remote)).await {
            Ok(Ok(device)) => device,
            Ok(Err(reason)) => {
                tracing::debug!(reason, "remote authentication failed");
                let _ = ws
                    .send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Policy,
                        reason: reason.into(),
                    })))
                    .await;
                return;
            }
            Err(_) => {
                tracing::debug!("remote authentication timed out");
                let _ = ws.close(None).await;
                return;
            }
        };
    drop(permit);
    serve(ws, &shared, kernel, device, revocations).await;
}

/// The JSON-RPC server for an authenticated device, until it leaves or is revoked.
async fn serve<S>(
    ws: WebSocketStream<S>,
    shared: &Shared,
    kernel: Kernel,
    device: String,
    revocations: broadcast::Receiver<String>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let limits = shared.remote.limits;
    let (reader, writer) = ws::split(ws, limits.ping, limits.idle);
    let stop = revoked(kernel, device.clone(), revocations);
    rpc::serve_messages(
        shared.host.clone(),
        Arc::clone(&shared.info),
        reader,
        writer,
        Peer::Device(device),
        stop,
    )
    .await;
}

/// Resolves when `device` is revoked.
async fn revoked(kernel: Kernel, device: String, mut rx: broadcast::Receiver<String>) -> Close {
    loop {
        match rx.recv().await {
            Ok(id) if id == device => break,
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // Missed some: ask the database instead.
                if !kernel.device_exists(&device).await.unwrap_or(false) {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => std::future::pending::<()>().await,
        }
    }
    Close {
        code: CLOSE_REVOKED,
        reason: "this device was revoked".into(),
    }
}

/// Reads and answers the first message, which must authenticate. Returns the device id,
/// or why the connection is refused (after the error response is sent). Params of
/// `auth.*` are never logged.
async fn gate<S>(
    ws: &mut WebSocketStream<S>,
    kernel: &Kernel,
    remote: &Remote,
) -> Result<String, &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let text = loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => break t,
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
            Some(Ok(Message::Binary(_))) => {
                return refuse(ws, None, "binary frames are not supported").await;
            }
            _ => return Err("closed before authenticating"),
        }
    };
    let value: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
    let id: Option<RequestId> = value
        .get("id")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let method = value.get("method").and_then(Value::as_str);
    let is_request = value.get("jsonrpc").and_then(Value::as_str) == Some("2.0") && id.is_some();
    let params = value
        .get("params")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let result = match (is_request, method) {
        (true, Some(m::AuthToken::NAME)) => token(kernel, params).await,
        (true, Some(m::AuthPair::NAME)) => pair(kernel, remote, params).await,
        _ => Err(RpcError::new(
            ErrorCode::Unauthenticated,
            "authenticate first: auth.token or auth.pair",
        )),
    };
    match result {
        Ok((device, result)) => {
            let Some(id) = id else {
                return Err("no request id");
            };
            let line = serde_json::to_string(&RpcResponse::success(id, result)).unwrap_or_default();
            if ws.send(Message::text(line)).await.is_err() {
                return Err("closed while authenticating");
            }
            Ok(device)
        }
        Err(err) => {
            let reason = match err.data.code {
                ErrorCode::Unauthenticated => "not authenticated",
                _ => "invalid authentication request",
            };
            send_error(ws, id, err).await;
            Err(reason)
        }
    }
}

async fn refuse<S>(
    ws: &mut WebSocketStream<S>,
    id: Option<RequestId>,
    why: &'static str,
) -> Result<String, &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send_error(ws, id, RpcError::new(ErrorCode::InvalidRequest, why)).await;
    Err(why)
}

async fn send_error<S>(ws: &mut WebSocketStream<S>, id: Option<RequestId>, err: RpcError)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Ok(line) = serde_json::to_string(&RpcResponse::error(id, err)) {
        let _ = ws.send(Message::text(line)).await;
    }
}

fn invalid(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::InvalidParams, e.to_string())
}

fn device(d: kumikit_kernel::devices::DeviceRecord) -> m::Device {
    m::Device {
        id: d.id,
        name: d.name,
        created: d.created,
        last_seen: d.last_seen,
        current: Some(true),
    }
}

async fn token(kernel: &Kernel, params: Value) -> Result<(String, Value), RpcError> {
    let p: AuthTokenParams = serde_json::from_value(params).map_err(invalid)?;
    let record = kernel
        .device_authenticate(&pairing::token_hash(&p.token))
        .await
        .map_err(|e| RpcError::new(e.code(), e.to_string()))?
        .ok_or_else(|| {
            RpcError::new(
                ErrorCode::Unauthenticated,
                "unknown or revoked device token",
            )
        })?;
    let id = record.id.clone();
    let result = serde_json::to_value(AuthTokenResult {
        device: device(record),
    })
    .map_err(invalid)?;
    Ok((id, result))
}

/// Redeems a pairing code and records the new device; returns its token (shown once)
/// and record.
async fn pair_device(
    kernel: &Kernel,
    remote: &Remote,
    code: &str,
    device_name: &str,
) -> Result<(String, kumikit_kernel::devices::DeviceRecord), RpcError> {
    if !valid_device_name(device_name) {
        return Err(invalid(
            "deviceName must be 1–64 characters without control characters",
        ));
    }
    if remote.redeem(code).is_err() {
        return Err(RpcError::new(
            ErrorCode::Unauthenticated,
            "wrong or expired pairing code",
        ));
    }
    let token =
        pairing::new_token().map_err(|e| RpcError::new(ErrorCode::Internal, e.to_string()))?;
    let record = kernel
        .device_insert(device_name.trim(), &pairing::token_hash(&token))
        .await
        .map_err(|e| RpcError::new(e.code(), e.to_string()))?;
    tracing::info!(device = %record.id, "paired a new device");
    Ok((token, record))
}

async fn pair(
    kernel: &Kernel,
    remote: &Remote,
    params: Value,
) -> Result<(String, Value), RpcError> {
    let p: AuthPairParams = serde_json::from_value(params).map_err(invalid)?;
    let (token, record) = pair_device(kernel, remote, &p.code, &p.device_name).await?;
    let id = record.id.clone();
    let result = serde_json::to_value(AuthPairResult {
        token,
        device: device(record),
    })
    .map_err(invalid)?;
    Ok((id, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_name_the_host_when_bound_to_every_interface() {
        let any = Remote::new(
            "0.0.0.0:7463".parse().unwrap(),
            "fp".into(),
            "devbox",
            Limits::default(),
        );
        assert_eq!(any.url, "wss://devbox:7463/rpc");
        let one = Remote::new(
            "[::1]:9000".parse().unwrap(),
            "fp".into(),
            "devbox",
            Limits::default(),
        );
        assert_eq!(one.url, "wss://[::1]:9000/rpc");
    }
}
