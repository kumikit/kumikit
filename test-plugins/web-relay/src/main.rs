//! Test only (M2 plan, Tests): the browser stand-in for the desktop shell. It starts a
//! real `kumikit serve` in a scratch directory with the fake ACP agent on its `PATH`,
//! serves the browser-test UI bundle and the bundled UI plugins on loopback, and relays
//! one WebSocket per page to the core with the shell's own bridge code. Never shipped.
//!
//! Usage: `kumikit-web-relay --ui <dir> --ui-plugins <dir> --core-plugins <dir> --work <dir>
//! [--remote]`. Prints `ready <url>` once it serves. With `--remote` (M3) it also starts a
//! second core with remote access on loopback, behind a cuttable proxy (`net-cut`), and
//! serves the shell's profile commands with the shell's own code, so pairing, pinning and
//! token login are the shipped ones.
//!
//! Control endpoints for tests:
//! `POST /test/pick {"path"}` queues what the next file or folder picker returns, and
//! `POST /test/drop` closes every bridge connection (the cores keep running). With
//! `--remote`: `POST /test/pair-code` makes a pairing code on the remote core (`kumikit
//! pair --json`, with the proxy's address as `address`), `POST /test/cut` and
//! `/test/restore` cut and restore the remote's network, `POST /test/revoke` revokes every
//! device of the remote core, and `POST /test/rotate` restarts the remote core with a new
//! certificate on the same port. Closing the relay's stdin stops the cores and the relay.

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
use kumikit_shell::client_registry::{ClientError, ClientRef, ClientRegistry};
use kumikit_shell::connections::{
    CommandError, Connections, Keepalive, PairRequest, SshRequest, SshUpdateRequest,
};
use kumikit_shell::profiles::{LOCAL, Profiles};
use kumikit_shell::secrets::TokenStore;
use kumikit_shell::ssh_sync::{self, SshTargetRequest};
use kumikit_shell::store::Store;
use kumikit_shell::updater::{AppUpdater, UpdateApplyRequest};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc};

/// The same strictness as the shell's CSP, for an http origin.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
    font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-src 'self'; \
    object-src 'none'";

struct Relay {
    ui: PathBuf,
    store: Store,
    /// The shell's client registry code, for UI plugins and themes from registries.
    registry: ClientRegistry,
    address: String,
    picks: Mutex<VecDeque<Option<String>>>,
    drop_all: broadcast::Sender<()>,
    connections: Connections,
    updater: AppUpdater,
    client_dir: PathBuf,
    updating: std::sync::atomic::AtomicBool,
    remote: Option<RemoteSide>,
}

/// The second core of `--remote`.
struct RemoteSide {
    socket: String,
    data: PathBuf,
    /// The core's own address, behind the proxy.
    target: std::net::SocketAddr,
    proxy: net_cut::Proxy,
    core: tokio::sync::Mutex<tokio::process::Child>,
    path: std::ffi::OsString,
    core_plugins: PathBuf,
}

/// Starts `kumikit serve`; returns it after its `ready` line, with the `remote` line if any.
async fn start_core(
    socket: &str,
    data: &Path,
    core_plugins: &Path,
    path: &std::ffi::OsStr,
    listen: Option<&str>,
) -> Result<(tokio::process::Child, Option<String>)> {
    let mut command = tokio::process::Command::new(core_bin()?);
    command
        .args(["serve", "--socket", socket])
        .arg("--data-dir")
        .arg(data)
        .arg("--bundled-plugins")
        .arg(core_plugins)
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(listen) = listen {
        command.args([
            "--listen",
            listen,
            "--keepalive-ping-ms",
            "200",
            "--keepalive-idle-ms",
            "1500",
        ]);
    }
    let mut core = command.spawn().context("start kumikit serve")?;
    let stdout = core.stdout.take().context("core stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut remote = None;
    loop {
        match lines.next_line().await? {
            Some(l) if l.starts_with("remote ") => remote = Some(l),
            Some(l) if l.starts_with("ready ") => break,
            other => bail!("the core did not start: {other:?}"),
        }
    }
    Ok((core, remote))
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
    let (mut core, _) =
        start_core(&address, &work.join("core"), &core_plugins, &path, None).await?;

    let remote = if args.iter().any(|a| a == "--remote") {
        #[cfg(unix)]
        let socket = work.join("r.sock").to_string_lossy().into_owned();
        #[cfg(windows)]
        let socket = format!(r"\\.\pipe\kumikit-relay-remote-{}", std::process::id());
        let data = work.join("remote-core");
        let (child, line) =
            start_core(&socket, &data, &core_plugins, &path, Some("127.0.0.1:0")).await?;
        let target: std::net::SocketAddr = line
            .as_deref()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|u| u.strip_prefix("wss://"))
            .and_then(|u| u.strip_suffix("/rpc"))
            .and_then(|a| a.parse().ok())
            .context("the remote core printed no address")?;
        Some(RemoteSide {
            socket,
            data,
            target,
            proxy: net_cut::Proxy::start(target).await?,
            core: tokio::sync::Mutex::new(child),
            path: path.clone(),
            core_plugins: core_plugins.clone(),
        })
    } else {
        None
    };
    let client = work.join("client");
    let connections = Connections {
        profiles: Profiles::new(&client),
        secrets: TokenStore::for_platform(&client, true),
        ssh: PathBuf::from(kumikit_test_plugins::FAKE_SSH),
        // Short, so a cut network is noticed in a second or two.
        keepalive: Keepalive {
            ping: std::time::Duration::from_millis(200),
            idle: std::time::Duration::from_millis(1000),
        },
        device_name: "Playwright".into(),
    };

    // The default registry comes from KUMIKIT_DEFAULT_REGISTRY, as in the app.
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let registry = ClientRegistry::new(&client, store.clone(), &env);
    let updater = AppUpdater::from_env(&client, address.clone(), &env, true);
    let relay = Arc::new(Relay {
        ui,
        registry,
        store,
        address,
        picks: Mutex::new(VecDeque::new()),
        drop_all: broadcast::Sender::new(4),
        connections,
        updater,
        client_dir: client,
        updating: std::sync::atomic::AtomicBool::new(false),
        remote,
    });
    let relay_watch = Arc::clone(&relay);
    let app = Router::new()
        .route("/bridge", get(bridge))
        .route("/test/pick", get(take_pick).post(queue_pick))
        .route("/test/drop", post(drop_connections))
        .route("/test/pair-code", post(pair_code))
        .route("/test/cut", post(cut))
        .route("/test/restore", post(restore))
        .route("/test/revoke", post(revoke))
        .route("/test/rotate", post(rotate))
        .route("/profiles", get(profiles_list))
        .route("/profiles/probe", post(profile_probe))
        .route("/profiles/pair", post(profile_pair))
        .route("/profiles/ssh", post(profile_add_ssh))
        .route("/profiles/ssh/update", post(profile_update_ssh))
        .route("/profiles/{name}", delete(profile_remove))
        .route("/ssh/probe", post(ssh_probe))
        .route("/ssh/sync", post(ssh_sync))
        .route("/updates/info", get(app_update_info))
        .route("/updates/check", post(app_update_check))
        .route("/updates/download", post(app_update_download))
        .route("/updates/apply", post(app_update_apply))
        .route("/client-plugins", get(list_plugins).post(install_plugin))
        .route("/client-plugins/{id}", delete(remove_plugin))
        .route("/client-registry/search", post(registry_search))
        .route("/client-registry/install", post(registry_install))
        .route("/client-registry/updates", post(registry_updates))
        .route("/client-registry/update", post(registry_update))
        .route(
            "/client-registry/registries",
            get(registries_get).post(registries_set),
        )
        .route("/client-registry/lock", get(registry_lock))
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
    let core_crashed = async {
        let _ = core.wait().await;
        if relay_watch
            .updating
            .load(std::sync::atomic::Ordering::Acquire)
        {
            std::future::pending::<()>().await;
        }
    };
    let result = tokio::select! {
        r = axum::serve(listener, app) => r.map_err(anyhow::Error::from),
        () = core_crashed => Err(anyhow::anyhow!("the core exited")),
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
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(dir) = r.store.dir(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = asset(assets::load(&dir, &format!("/{path}")));
    // A sandboxed panel's files get its own CSP, as the shell and the core send them.
    if let Some(network) = r.store.panel_network(&id) {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .filter(|h| {
                h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
            })
            .unwrap_or("localhost");
        let own = format!("http://{host}");
        let csp = format!(
            "{}; frame-ancestors {own}",
            assets::panel_csp(&format!("{own}/plugins/{id}/"), &network)
        );
        let h = response.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&csp) {
            h.insert(header::CONTENT_SECURITY_POLICY, v);
        }
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("null"),
        );
    }
    response
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

#[derive(Deserialize)]
struct BridgeQuery {
    profile: String,
}

async fn bridge(
    State(r): State<Arc<Relay>>,
    axum::extract::Query(q): axum::extract::Query<BridgeQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| relay_socket(r, q.profile, socket))
}

/// One page connection: the shell's bridge on one side, the WebSocket on the other. The
/// first message says whether connecting worked: `{"type":"open"}`, or
/// `{"type":"error","code","message"}` as `bridge_connect` would fail.
async fn relay_socket(r: Arc<Relay>, profile: String, mut socket: WebSocket) {
    let bridge = Bridge::default();
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let sink = move |m| drop(tx.send(m));
    let attached = if profile == LOCAL {
        match kumikit_shell::transport::connect(&r.address).await {
            Ok(stream) => Ok(bridge.attach(stream, sink)),
            Err(e) => Err(CommandError {
                code: "unreachable",
                message: e.to_string(),
            }),
        }
    } else {
        r.connections.connect(&bridge, &profile, sink).await
    };
    let id = match attached {
        Ok(id) => id,
        Err(e) => {
            let msg = serde_json::json!({ "type": "error", "code": e.code, "message": e.message });
            let _ = socket.send(Message::Text(msg.to_string().into())).await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    if socket
        .send(Message::Text(r#"{"type":"open"}"#.into()))
        .await
        .is_err()
    {
        bridge.close(&id);
        return;
    }
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

// ---- the client registry (the shell's commands) ----

fn client_err(e: ClientError) -> Response {
    (StatusCode::BAD_REQUEST, axum::Json(e)).into_response()
}

fn reply<T: serde::Serialize>(r: Result<T, ClientError>) -> Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => client_err(e),
    }
}

#[derive(serde::Deserialize)]
struct Search {
    query: String,
    refresh: bool,
}

async fn registry_search(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<Search>,
) -> Response {
    reply(r.registry.search(&p.query, p.refresh).await)
}

async fn registry_install(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<ClientRef>,
) -> Response {
    reply(r.registry.install(&p).await)
}

#[derive(serde::Deserialize)]
struct Refresh {
    refresh: bool,
}

async fn registry_updates(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<Refresh>,
) -> Response {
    reply(r.registry.updates(p.refresh).await)
}

#[derive(serde::Deserialize)]
struct Update {
    id: String,
    version: String,
}

async fn registry_update(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<Update>,
) -> Response {
    reply(r.registry.update(&p.id, &p.version).await)
}

async fn registries_get(State(r): State<Arc<Relay>>) -> Response {
    reply(r.registry.registries().await)
}

#[derive(serde::Deserialize)]
struct Urls {
    urls: Vec<String>,
}

async fn registries_set(State(r): State<Arc<Relay>>, axum::Json(p): axum::Json<Urls>) -> Response {
    reply(
        r.registry
            .set_registries(&p.urls)
            .await
            .map(|()| serde_json::json!({})),
    )
}

async fn registry_lock(State(r): State<Arc<Relay>>) -> Response {
    axum::Json(serde_json::json!({ "lock": r.store.lock_text() })).into_response()
}

// ---- profiles (the shell's commands) ----

fn command_err(e: CommandError) -> Response {
    (StatusCode::BAD_REQUEST, axum::Json(e)).into_response()
}

async fn profiles_list(State(r): State<Arc<Relay>>) -> Response {
    match r.connections.list() {
        Ok(l) => axum::Json(l).into_response(),
        Err(e) => command_err(e),
    }
}

#[derive(Deserialize)]
struct Probe {
    address: String,
}

async fn profile_probe(State(r): State<Arc<Relay>>, axum::Json(p): axum::Json<Probe>) -> Response {
    match r.connections.probe(&p.address).await {
        Ok(f) => axum::Json(f).into_response(),
        Err(e) => command_err(e),
    }
}

async fn profile_pair(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<PairRequest>,
) -> Response {
    match r.connections.pair(p).await {
        Ok(i) => axum::Json(i).into_response(),
        Err(e) => command_err(e),
    }
}

async fn profile_add_ssh(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<SshRequest>,
) -> Response {
    match r.connections.add_ssh(p).await {
        Ok(i) => axum::Json(i).into_response(),
        Err(e) => command_err(e),
    }
}

async fn profile_update_ssh(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<SshUpdateRequest>,
) -> Response {
    match r.connections.update_ssh(p) {
        Ok(i) => axum::Json(i).into_response(),
        Err(e) => command_err(e),
    }
}

async fn ssh_probe(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<SshTargetRequest>,
) -> Response {
    match ssh_sync::probe(
        &r.connections.ssh,
        &r.connections.profiles,
        r.updater.current_version(),
        &p,
    )
    .await
    {
        Ok(res) => axum::Json(res).into_response(),
        Err(e) => command_err(e),
    }
}

async fn ssh_sync(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<SshTargetRequest>,
) -> Response {
    let release = match r.updater.release_client() {
        Ok(c) => c,
        Err(e) => return command_err(e),
    };
    match ssh_sync::sync(
        &r.connections.ssh,
        &r.connections.profiles,
        release,
        &r.client_dir,
        r.updater.current_version(),
        &p,
    )
    .await
    {
        Ok(res) => axum::Json(res).into_response(),
        Err(e) => command_err(e),
    }
}

async fn app_update_info(State(r): State<Arc<Relay>>) -> Response {
    axum::Json(r.updater.info()).into_response()
}

async fn app_update_check(State(r): State<Arc<Relay>>) -> Response {
    match r.updater.check().await {
        Ok(res) => axum::Json(res).into_response(),
        Err(e) => command_err(e),
    }
}

async fn app_update_download(State(r): State<Arc<Relay>>) -> Response {
    match r.updater.download().await {
        Ok(res) => axum::Json(res).into_response(),
        Err(e) => command_err(e),
    }
}

async fn app_update_apply(
    State(r): State<Arc<Relay>>,
    axum::Json(p): axum::Json<UpdateApplyRequest>,
) -> Response {
    r.updating.store(true, std::sync::atomic::Ordering::Release);
    match r.updater.apply(p).await {
        Ok(res) => axum::Json(res).into_response(),
        Err(e) => {
            r.updating
                .store(false, std::sync::atomic::Ordering::Release);
            command_err(e)
        }
    }
}

async fn profile_remove(State(r): State<Arc<Relay>>, UrlPath(name): UrlPath<String>) -> Response {
    match r.connections.remove(&name).await {
        Ok(removed) => axum::Json(removed).into_response(),
        Err(e) => command_err(e),
    }
}

// ---- the remote core's test controls ----

fn no_remote() -> Response {
    (StatusCode::NOT_FOUND, "start the relay with --remote").into_response()
}

fn server_err(e: impl std::fmt::Display) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
}

async fn kumikit_cli(args: &[&str]) -> Result<std::process::Output, String> {
    let bin = core_bin().map_err(|e| e.to_string())?;
    tokio::process::Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())
}

async fn pair_code(State(r): State<Arc<Relay>>) -> Response {
    let Some(remote) = &r.remote else {
        return no_remote();
    };
    let out = match kumikit_cli(&["pair", "--json", "--socket", &remote.socket]).await {
        Ok(o) => o,
        Err(e) => return server_err(e),
    };
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
            .into_response();
    };
    // Clients reach the remote core through the proxy.
    v["address"] = serde_json::json!(remote.proxy.addr().to_string());
    axum::Json(v).into_response()
}

async fn cut(State(r): State<Arc<Relay>>) -> Response {
    let Some(remote) = &r.remote else {
        return no_remote();
    };
    remote.proxy.cut();
    StatusCode::NO_CONTENT.into_response()
}

async fn restore(State(r): State<Arc<Relay>>) -> Response {
    let Some(remote) = &r.remote else {
        return no_remote();
    };
    remote.proxy.restore();
    StatusCode::NO_CONTENT.into_response()
}

async fn revoke(State(r): State<Arc<Relay>>) -> Response {
    let Some(remote) = &r.remote else {
        return no_remote();
    };
    let out = match kumikit_cli(&["devices", "list", "--socket", &remote.socket]).await {
        Ok(o) => o,
        Err(e) => return server_err(e),
    };
    let ids: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next().filter(|w| w.len() == 26))
        .map(str::to_owned)
        .collect();
    for id in ids {
        if let Err(e) = kumikit_cli(&["devices", "revoke", &id, "--socket", &remote.socket]).await {
            return server_err(e);
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Restarts the remote core with a new certificate on the same port.
async fn rotate(State(r): State<Arc<Relay>>) -> Response {
    let Some(remote) = &r.remote else {
        return no_remote();
    };
    let mut core = remote.core.lock().await;
    let _ = core.kill().await;
    let _ = std::fs::remove_dir_all(remote.data.join("tls"));
    let listen = remote.target.to_string();
    match start_core(
        &remote.socket,
        &remote.data,
        &remote.core_plugins,
        &remote.path,
        Some(&listen),
    )
    .await
    {
        Ok((child, _)) => {
            *core = child;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => server_err(e),
    }
}
