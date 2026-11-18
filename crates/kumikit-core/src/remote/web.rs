//! The browser client's side of the remote transport (`docs/spec/06-client.md#browser-client`,
//! `docs/spec/02-protocol.md#remote-transport`), on only with `serve --web <dir>`: the UI
//! files, pairing from the page into an HttpOnly cookie, and the WebSocket upgrade
//! authenticated by that cookie.
//!
//! A browser is recognised by its `Origin` header. It must be the core's own origin, so a
//! page on another site can neither open the socket nor post to the pairing routes.

use std::path::Path;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use kumikit_protocol::remote::DEVICE_COOKIE;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

use super::{Body, Shared, pair_device, pairing, status, upgrade};

/// Largest accepted request body (pairing and sign-out are tiny).
const MAX_BODY: usize = 16 * 1024;

/// How long the device cookie lasts: a year.
const COOKIE_AGE: u32 = 365 * 24 * 60 * 60;

/// `https://` and the request's `Host`: the origin the page was loaded from.
fn own_origin(req: &Request<Incoming>) -> Option<String> {
    let host = req.headers().get(header::HOST)?.to_str().ok()?;
    Some(format!("https://{host}"))
}

/// Whether the request comes from a page of this core.
fn same_origin(req: &Request<Incoming>) -> bool {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    matches!((origin, own_origin(req)), (Some(o), Some(own)) if o == own)
}

/// The device token in the request's cookie, if any.
fn cookie_token(req: &Request<Incoming>) -> Option<String> {
    req.headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| {
            c.trim()
                .strip_prefix(DEVICE_COOKIE)?
                .strip_prefix('=')
                .map(str::to_owned)
        })
}

fn json(code: StatusCode, body: serde_json::Value) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from(body.to_string())));
    *r.status_mut() = code;
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn cookie(token: &str, age: u32) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{DEVICE_COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={age}"
    ))
    .ok()
}

/// A browser's WebSocket on `/rpc`: refused unless it comes from this core's page and
/// carries a paired device's cookie.
pub(super) async fn browser_upgrade(
    req: Request<Incoming>,
    shared: Arc<Shared>,
    permit: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    deadline: Instant,
) -> Response<Body> {
    if shared.remote.web.is_none() {
        return status(StatusCode::FORBIDDEN, "browsers are not served here\n");
    }
    if !same_origin(&req) {
        return status(StatusCode::FORBIDDEN, "not this core's page\n");
    }
    let kernel = shared.host.manager().kernel().clone();
    // Subscribed before the cookie is checked, so a revocation in between is not missed.
    let revocations = kernel.device_revocations();
    let device = match cookie_token(&req) {
        Some(t) => kernel
            .device_authenticate(&pairing::token_hash(&t))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let Some(device) = device else {
        return status(StatusCode::UNAUTHORIZED, "pair this browser first\n");
    };
    upgrade(
        req,
        shared,
        permit,
        deadline,
        Some((device.id, revocations)),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairBody {
    code: String,
    #[serde(default)]
    device_name: Option<String>,
}

/// The web routes: the UI files, `/auth/status`, `/auth/pair` and `/auth/logout`.
pub(super) async fn serve(req: Request<Incoming>, shared: &Shared, dir: &Path) -> Response<Body> {
    let kernel = shared.host.manager().kernel().clone();
    let path = req.uri().path().to_owned();
    match (req.method().clone(), path.as_str()) {
        (Method::GET, "/auth/status") => {
            let paired = match cookie_token(&req) {
                Some(t) => kernel
                    .device_authenticate(&pairing::token_hash(&t))
                    .await
                    .ok()
                    .flatten()
                    .is_some(),
                None => false,
            };
            json(
                StatusCode::OK,
                json!({ "paired": paired, "core": { "name": shared.info.name } }),
            )
        }
        (Method::POST, "/auth/pair" | "/auth/logout") => {
            // A form or a fetch from another site cannot pass both checks.
            let is_json = req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/json"));
            if !same_origin(&req) || !is_json {
                return status(StatusCode::FORBIDDEN, "not this core's page\n");
            }
            if path == "/auth/logout" {
                if let Some(t) = cookie_token(&req)
                    && let Ok(Some(d)) = kernel.device_authenticate(&pairing::token_hash(&t)).await
                {
                    let _ = kernel.device_revoke(&d.id).await;
                }
                let mut r = json(StatusCode::OK, json!({}));
                if let Some(c) = cookie("", 0) {
                    r.headers_mut().insert(header::SET_COOKIE, c);
                }
                return r;
            }
            let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return status(StatusCode::PAYLOAD_TOO_LARGE, "too large\n"),
            };
            let Ok(p) = serde_json::from_slice::<PairBody>(&body) else {
                return json(
                    StatusCode::BAD_REQUEST,
                    json!({ "code": "invalid_params", "message": "expected { code, deviceName }" }),
                );
            };
            let name = p
                .device_name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "Browser".into());
            match pair_device(&kernel, &shared.remote, &p.code, &name).await {
                Ok((token, record)) => {
                    // The token goes only into the cookie, which page scripts cannot read.
                    let mut r = json(
                        StatusCode::OK,
                        json!({ "device": { "id": record.id, "name": record.name, "created": record.created } }),
                    );
                    if let Some(c) = cookie(&token, COOKIE_AGE) {
                        r.headers_mut().insert(header::SET_COOKIE, c);
                    }
                    r
                }
                Err(e) => json(
                    if e.data.code == kumikit_protocol::ErrorCode::Unauthenticated {
                        StatusCode::UNAUTHORIZED
                    } else {
                        StatusCode::BAD_REQUEST
                    },
                    json!({ "code": e.data.code.as_str(), "message": e.message }),
                ),
            }
        }
        (Method::GET, "/ui-plugins/index.json") => match ui_plugins(dir).await {
            Ok(list) => json(StatusCode::OK, list),
            Err(e) => {
                tracing::warn!(error = %e, "cannot list the web UI's plugins");
                json(StatusCode::OK, json!([]))
            }
        },
        (Method::GET | Method::HEAD, _) => file(dir, &path),
        _ => status(StatusCode::METHOD_NOT_ALLOWED, "not allowed\n"),
    }
}

/// The UI plugins served with the web UI (`<dir>/ui-plugins/<id>/plugin.toml`), listed as
/// the shell's client plugin store lists bundled ones: `{ id, version, source, manifest }`.
/// They are read-only; the UI kernel checks their contents when it loads them.
async fn ui_plugins(dir: &Path) -> std::io::Result<serde_json::Value> {
    let root = dir.join("ui-plugins");
    tokio::task::spawn_blocking(move || {
        let mut list = Vec::new();
        let entries = match std::fs::read_dir(&root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!([])),
            Err(e) => return Err(e),
        };
        let mut dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let Ok(text) = std::fs::read_to_string(d.join("plugin.toml")) else {
                continue;
            };
            let manifest: serde_json::Value = match toml::from_str::<toml::Value>(&text) {
                Ok(v) => serde_json::to_value(v).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(dir = %d.display(), error = %e, "skipping a UI plugin with a bad manifest");
                    continue;
                }
            };
            let id = manifest["id"].as_str().unwrap_or_default().to_owned();
            // The directory name is the id; a manifest that says otherwise is skipped,
            // since its files are served from the directory.
            if d.file_name().and_then(|n| n.to_str()) != Some(id.as_str()) {
                continue;
            }
            list.push(json!({ "id": id, "version": manifest["version"], "source": "bundled", "manifest": manifest }));
        }
        Ok(serde_json::Value::Array(list))
    })
    .await
    .map_err(std::io::Error::other)?
}

/// A file of the web UI, with the browser client's CSP.
fn file(dir: &Path, path: &str) -> Response<Body> {
    let a = kumikit_assets::load(dir, path);
    let mut r = Response::new(Full::new(Bytes::from(a.body)));
    *r.status_mut() = StatusCode::from_u16(a.status).unwrap_or(StatusCode::NOT_FOUND);
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(a.content_type),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(kumikit_assets::WEB_CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_cookie_is_http_only_and_strict() {
        let c = super::cookie("kkd_x", 10).unwrap();
        let c = c.to_str().unwrap();
        for part in [
            "kk_device=kkd_x",
            "HttpOnly",
            "Secure",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=10",
        ] {
            assert!(c.contains(part), "{c}");
        }
    }
}
