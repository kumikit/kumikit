//! Test only (M2 plan, Tests): the browser stand-in for the desktop shell. It starts a
//! real `kumikit serve` in a scratch directory with the fake ACP agent on its `PATH`,
//! serves the browser-test UI bundle and the bundled UI plugins on loopback, and relays
//! one WebSocket per page to the core with the shell's own bridge code. Never shipped.
//!
//! Usage: `kumikit-web-relay --ui <dir> --ui-plugins <dir> --core-plugins <dir> --work <dir>`
//! Prints `ready <url>` once it serves. Control endpoints for tests:
//! `POST /test/pick {"path"}` queues what the next file or folder picker returns, and
//! `POST /test/drop` closes every bridge connection (the core keeps running). Closing the
//! relay's stdin stops the core and the relay.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use kumikit_shell::assets::{self, Asset};
use kumikit_shell::bridge::{Bridge, Incoming};
use kumikit_shell::store::Store;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc};

/// The same strictness as the shell's CSP, for an http origin.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
    font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-src 'none'; \
    object-src 'none'";

struct Relay {
    ui: PathBuf,
    store: Store,
    address: String,
    picks: Mutex<VecDeque<Option<String>>>,
    drop_all: broadcast::Sender<()>,
}

fn arg(args: &[String], name: &str) -> Result<PathBuf> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .with_context(|| format!("{name} <dir> is required"))
}

/// The `kumikit` binary built next to this one.
fn core_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) {
        "kumikit.exe"
    } else {
        "kumikit"
    };
    exe.ancestors()
        .skip(1)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .context("build the core first: cargo build -p kumikit-core")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let ui = arg(&args, "--ui")?;
    let work = arg(&args, "--work")?;
    let store = Store {
        bundled: Some(arg(&args, "--ui-plugins")?),
        installed: work.join("client/ui-plugins"),
    };
    let core_plugins = arg(&args, "--core-plugins")?;
    std::fs::create_dir_all(&work)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(unix)]
    let address = work.join("s.sock").to_string_lossy().into_owned();
    #[cfg(windows)]
    let address = format!(r"\\.\pipe\kumikit-relay-{}", std::process::id());

    // The core, with the fake agent first on its PATH.
    let agent_dir = Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .context("fake agent path")?
        .to_owned();
    let path = std::env::join_paths(std::iter::once(agent_dir).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))?;
    let mut core = tokio::process::Command::new(core_bin()?)
        .args(["serve", "--socket", &address])
        .arg("--data-dir")
        .arg(work.join("core"))
        .arg("--bundled-plugins")
        .arg(&core_plugins)
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start kumikit serve")?;
    let stdout = core.stdout.take().context("core stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    match lines.next_line().await? {
        Some(l) if l.starts_with("ready ") => {}
        other => bail!("the core did not start: {other:?}"),
    }

    let relay = Arc::new(Relay {
        ui,
        store,
        address,
        picks: Mutex::new(VecDeque::new()),
        drop_all: broadcast::Sender::new(4),
    });
    let app = Router::new()
        .route("/bridge", get(bridge))
        .route("/test/pick", get(take_pick).post(queue_pick))
        .route("/test/drop", post(drop_connections))
        .route("/client-plugins", get(list_plugins).post(install_plugin))
        .route("/client-plugins/{id}", delete(remove_plugin))
        .route("/plugins/{id}/{*path}", get(plugin_file))
        .fallback(get(ui_file))
        .with_state(relay);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    {
        use std::io::Write;
        let mut out = std::io::stdout();
        writeln!(out, "ready {url}")?;
        out.flush()?;
    }
    // The test runner holds our stdin; when it closes (the test ended or crashed), the
    // core is stopped with us, so no core outlives a test.
    let stdin_closed = async {
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::stdin(), &mut buf).await;
    };
    let result = tokio::select! {
        r = axum::serve(listener, app) => r.map_err(anyhow::Error::from),
        _ = core.wait() => Err(anyhow::anyhow!("the core exited")),
        () = stdin_closed => Ok(()),
    };
    let _ = core.kill().await;
    result
}

fn asset(a: Asset) -> Response {
    let mut r = (
        StatusCode::from_u16(a.status).unwrap_or(StatusCode::NOT_FOUND),
        a.body,
    )
        .into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(a.content_type),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn ui_file(State(r): State<Arc<Relay>>, uri: Uri) -> Response {
    asset(assets::load(&r.ui, uri.path()))
}

async fn plugin_file(
    State(r): State<Arc<Relay>>,
    UrlPath((id, path)): UrlPath<(String, String)>,
) -> Response {
    match r.store.dir(&id) {
        Some(dir) => asset(assets::load(&dir, &format!("/{path}"))),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct Pick {
    path: Option<String>,
}

async fn queue_pick(State(r): State<Arc<Relay>>, axum::Json(p): axum::Json<Pick>) -> StatusCode {
    r.picks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(p.path);
    StatusCode::NO_CONTENT
}

async fn take_pick(State(r): State<Arc<Relay>>) -> axum::Json<serde_json::Value> {
    let next = r
        .picks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
        .flatten();
    axum::Json(serde_json::json!({ "path": next }))
}

async fn drop_connections(State(r): State<Arc<Relay>>) -> StatusCode {
    let _ = r.drop_all.send(());
    StatusCode::NO_CONTENT
}

fn store_err(e: impl std::fmt::Display) -> Response {
    (StatusCode::BAD_REQUEST, e.to_string()).into_response()
}

async fn list_plugins(State(r): State<Arc<Relay>>) -> Response {
    axum::Json(r.store.list()).into_response()
}

async fn install_plugin(State(r): State<Arc<Relay>>, axum::Json(p): axum::Json<Pick>) -> Response {
    let Some(path) = p.path else {
        return store_err("path is required");
    };
    match r.store.install(Path::new(&path)) {
        Ok(entry) => axum::Json(entry).into_response(),
        Err(e) => store_err(e),
    }
}

async fn remove_plugin(State(r): State<Arc<Relay>>, UrlPath(id): UrlPath<String>) -> Response {
    match r.store.remove(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e),
    }
}

async fn bridge(State(r): State<Arc<Relay>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| relay_socket(r, socket))
}

/// One page connection: the shell's bridge on one side, the WebSocket on the other.
async fn relay_socket(r: Arc<Relay>, mut socket: WebSocket) {
    let stream = match kumikit_shell::transport::connect(&r.address).await {
        Ok(s) => s,
        Err(e) => {
            let msg = serde_json::json!({ "type": "closed", "reason": e.to_string() });
            let _ = socket.send(Message::Text(msg.to_string().into())).await;
            return;
        }
    };
    let bridge = Bridge::default();
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let id = bridge.attach(stream, move |m| drop(tx.send(m)));
    let mut drops = r.drop_all.subscribe();
    loop {
        tokio::select! {
            incoming = rx.recv() => {
                let Some(m) = incoming else { break };
                let closed = matches!(m, Incoming::Closed { .. });
                let text = serde_json::to_string(&m).unwrap_or_default();
                if socket.send(Message::Text(text.into())).await.is_err() || closed {
                    break;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(line))) => {
                    if bridge.send(&id, line.to_string()).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            },
            _ = drops.recv() => break,
        }
    }
    bridge.close(&id);
    let _ = socket.send(Message::Close(None)).await;
}
