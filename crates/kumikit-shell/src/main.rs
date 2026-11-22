//! The desktop app: one window around the UI kernel. All logic is in the library; this
//! file wires it into Tauri.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::path::PathBuf;

use kumikit_protocol::local::Platform;
use kumikit_shell::assets::{self, APP_SCHEME, Asset, PLUGIN_SCHEME, SchemeForm};
use kumikit_shell::bridge::{Bridge, Incoming};
use kumikit_shell::client_registry::{
    ClientError, ClientRef, ClientRegistry, ClientResult, ClientUpdate,
};
use kumikit_shell::connections::{
    CommandError, Connections, Keepalive, PairRequest, Removed, SshRequest,
};
use kumikit_shell::local::{self, LocalCore};
use kumikit_shell::profiles::{LOCAL, ProfileInfo, Profiles};
use kumikit_shell::secrets::TokenStore;
use kumikit_shell::store::{Entry, Store};
use tauri::http::{Request, Response};
use tauri::ipc::Channel;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};

/// What the commands share.
struct Shell {
    bridge: Bridge,
    local: LocalCore,
    store: Store,
    /// UI plugins and themes from registries.
    registry: ClientRegistry,
    /// Remote profiles.
    connections: Connections,
    /// The UI kernel's files.
    ui: PathBuf,
}

/// Who a response is for.
enum Served {
    /// The UI kernel's own files.
    App,
    /// An inline UI plugin or a theme: imported from the app origin.
    Plugin,
    /// A sandboxed panel's files, with that panel's CSP. The panel's document has an
    /// opaque origin, so its module scripts are fetched with `Origin: null`.
    Panel(String),
}

fn respond(asset: Asset, form: SchemeForm, served: Served) -> Response<Cow<'static, [u8]>> {
    let csp = match &served {
        Served::Panel(csp) => csp.clone(),
        Served::App | Served::Plugin => assets::csp(form),
    };
    let body = if matches!(served, Served::Panel(_))
        && form == SchemeForm::Native
        && asset.status == 200
        && asset.content_type.starts_with("text/html")
    {
        assets::panel_html_crossorigin(&asset.body)
    } else {
        asset.body
    };
    let mut builder = Response::builder()
        .status(asset.status)
        .header("Content-Type", asset.content_type)
        .header("Content-Security-Policy", csp)
        .header("X-Content-Type-Options", "nosniff");
    match served {
        Served::App => {}
        // Plugin modules are imported from the app origin.
        Served::Plugin => {
            builder = builder.header("Access-Control-Allow-Origin", form.origin(APP_SCHEME));
        }
        Served::Panel(_) => builder = builder.header("Access-Control-Allow-Origin", "null"),
    }
    builder
        .body(Cow::Owned(body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
}

/// Opens a connection to a profile's core.
#[tauri::command]
async fn bridge_connect(
    shell: State<'_, Shell>,
    profile: String,
    channel: Channel<Incoming>,
) -> Result<String, CommandError> {
    // A closed channel means the page went away; the connection closes with it.
    let sink = move |m| {
        let _ = channel.send(m);
    };
    if profile != LOCAL {
        return shell
            .connections
            .connect(&shell.bridge, &profile, sink)
            .await;
    }
    let (stream, found) =
        local::connect_or_start(&shell.local)
            .await
            .map_err(|e| CommandError {
                code: "unreachable",
                message: e.to_string(),
            })?;
    #[cfg(feature = "test-hooks")]
    {
        #[allow(clippy::print_stdout)]
        {
            println!("ui-connected {found:?}");
        }
    }
    #[cfg(not(feature = "test-hooks"))]
    let _ = found;
    Ok(shell.bridge.attach(stream, sink))
}

/// The profiles the webview may connect to: names, labels and kinds only.
#[tauri::command]
fn profiles_list(shell: State<'_, Shell>) -> Result<Vec<ProfileInfo>, CommandError> {
    shell.connections.list()
}

/// The certificate fingerprint of the core at an address; sends nothing to it.
#[tauri::command]
async fn profile_probe(shell: State<'_, Shell>, address: String) -> Result<String, CommandError> {
    shell.connections.probe(&address).await
}

/// Pairs with a core, pinned to the fingerprint the user confirmed, and saves it.
#[tauri::command]
async fn profile_pair(
    shell: State<'_, Shell>,
    request: PairRequest,
) -> Result<ProfileInfo, CommandError> {
    shell.connections.pair(request).await
}

/// Adds a core reached through `ssh`.
#[tauri::command]
async fn profile_add_ssh(
    shell: State<'_, Shell>,
    request: SshRequest,
) -> Result<ProfileInfo, CommandError> {
    shell.connections.add_ssh(request).await
}

/// Forgets a core (revoking this device there when it answers).
#[tauri::command]
async fn profile_remove(shell: State<'_, Shell>, name: String) -> Result<Removed, CommandError> {
    shell.connections.remove(&name).await
}

/// Sends one JSON-RPC message.
#[tauri::command]
async fn bridge_send(
    shell: State<'_, Shell>,
    connection: String,
    line: String,
) -> Result<(), String> {
    shell
        .bridge
        .send(&connection, line)
        .await
        .map_err(|e| e.to_string())
}

/// Closes a connection.
#[tauri::command]
fn bridge_close(shell: State<'_, Shell>, connection: String) -> bool {
    shell.bridge.close(&connection)
}

/// The UI plugins and themes on this client.
#[tauri::command]
async fn client_plugins_list(shell: State<'_, Shell>) -> Result<Vec<Entry>, String> {
    let store = shell.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.list())
        .await
        .map_err(|e| e.to_string())
}

/// Installs a UI plugin or theme package.
#[tauri::command]
async fn client_plugins_install(shell: State<'_, Shell>, path: String) -> Result<Entry, String> {
    let store = shell.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.install(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Installs a UI plugin or theme from the client's registries.
#[tauri::command]
async fn client_plugins_install_registry(
    shell: State<'_, Shell>,
    plugin: ClientRef,
) -> Result<Entry, ClientError> {
    shell.registry.install(&plugin).await
}

/// UI plugins and themes the registries offer.
#[tauri::command]
async fn client_registry_search(
    shell: State<'_, Shell>,
    query: String,
    refresh: bool,
) -> Result<Vec<ClientResult>, ClientError> {
    shell.registry.search(&query, refresh).await
}

/// The client's registries, in order.
#[tauri::command]
async fn client_registries_get(shell: State<'_, Shell>) -> Result<Vec<String>, ClientError> {
    shell.registry.registries().await
}

/// Replaces the client's registries.
#[tauri::command]
async fn client_registries_set(
    shell: State<'_, Shell>,
    urls: Vec<String>,
) -> Result<(), ClientError> {
    shell.registry.set_registries(&urls).await
}

/// Updates the registries offer for installed UI plugins and themes.
#[tauri::command]
async fn client_plugins_updates(
    shell: State<'_, Shell>,
    refresh: bool,
) -> Result<Vec<ClientUpdate>, ClientError> {
    shell.registry.updates(refresh).await
}

/// Applies one update.
#[tauri::command]
async fn client_plugins_update(
    shell: State<'_, Shell>,
    id: String,
    version: String,
) -> Result<Entry, ClientError> {
    shell.registry.update(&id, &version).await
}

/// The client lockfile's text.
#[tauri::command]
async fn client_lock(shell: State<'_, Shell>) -> Result<String, String> {
    let store = shell.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.lock_text())
        .await
        .map_err(|e| e.to_string())
}

/// Removes an installed UI plugin or theme.
#[tauri::command]
async fn client_plugins_remove(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    let store = shell.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.remove(&id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

// Test builds keep the real plugin as a dependency: its permission set (`dialog:allow-open`)
// is what the capability grants, and the stand-in answers under the same name.
#[cfg(feature = "test-hooks")]
use tauri_plugin_dialog as _;

/// The `dialog` plugin of test builds: `open` returns the next queued path.
#[cfg(feature = "test-hooks")]
fn test_dialog() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    #[tauri::command]
    fn open() -> Option<String> {
        let queue = env("KUMIKIT_TEST_PICKS")?;
        kumikit_shell::test_hooks::next_pick(std::path::Path::new(&queue))
    }
    tauri::plugin::Builder::new("dialog")
        .invoke_handler(tauri::generate_handler![open])
        .build()
}

/// Test hook: the page reports a line on the shell's stdout.
#[cfg(feature = "test-hooks")]
#[tauri::command]
fn test_report(text: String) {
    #[allow(clippy::print_stdout)]
    {
        println!("report {text}");
    }
}

/// Keepalive of remote connections; test builds take `KUMIKIT_TEST_KEEPALIVE=<ping ms>,<idle ms>`
/// so a lost network is noticed in seconds.
fn keepalive() -> Keepalive {
    #[cfg(feature = "test-hooks")]
    if let Some((Ok(ping), Ok(idle))) = env("KUMIKIT_TEST_KEEPALIVE")
        .as_deref()
        .and_then(|v| v.split_once(','))
        .map(|(p, i)| (p.parse(), i.parse()))
    {
        return Keepalive {
            ping: std::time::Duration::from_millis(ping),
            idle: std::time::Duration::from_millis(idle),
        };
    }
    Keepalive::default()
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A resource directory, or the environment variable that overrides it (06-client).
fn resource(app: &tauri::App, var: &str, name: &str) -> Option<PathBuf> {
    env(var)
        .map(PathBuf::from)
        .or_else(|| app.path().resource_dir().ok().map(|d| d.join(name)))
}

fn main() {
    let form = SchemeForm::current();
    let safe = std::env::args().any(|a| a == "--safe");
    // The open dialog only: packages and workspace folders (ADR 0013). Test builds answer
    // it from a queue file instead, since WebDriver cannot drive native dialogs.
    #[cfg(not(feature = "test-hooks"))]
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(feature = "test-hooks")]
    let builder = tauri::Builder::default().plugin(test_dialog());
    #[cfg(feature = "test-hooks")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bridge_connect,
        bridge_send,
        bridge_close,
        client_plugins_list,
        client_plugins_install,
        client_plugins_install_registry,
        client_plugins_remove,
        client_plugins_updates,
        client_plugins_update,
        client_registry_search,
        client_registries_get,
        client_registries_set,
        client_lock,
        profiles_list,
        profile_probe,
        profile_pair,
        profile_add_ssh,
        profile_remove,
        test_report
    ]);
    #[cfg(not(feature = "test-hooks"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bridge_connect,
        bridge_send,
        bridge_close,
        client_plugins_list,
        client_plugins_install,
        client_plugins_install_registry,
        client_plugins_remove,
        client_plugins_updates,
        client_plugins_update,
        client_registry_search,
        client_registries_get,
        client_registries_set,
        client_lock,
        profiles_list,
        profile_probe,
        profile_pair,
        profile_add_ssh,
        profile_remove
    ]);
    let result = builder
        .register_uri_scheme_protocol(APP_SCHEME, move |ctx, request: Request<Vec<u8>>| {
            let shell = ctx.app_handle().state::<Shell>();
            respond(assets::load(&shell.ui, request.uri().path()), form, Served::App)
        })
        .register_uri_scheme_protocol(PLUGIN_SCHEME, move |ctx, request: Request<Vec<u8>>| {
            let shell = ctx.app_handle().state::<Shell>();
            let uri = request.uri();
            let host = uri.host().unwrap_or("");
            let Some((id, path)) = assets::plugin_target(form, host, uri.path()) else {
                return respond(assets::load(&PathBuf::new(), "/missing"), form, Served::Plugin);
            };
            let asset = match shell.store.dir(&id) {
                Some(dir) => assets::load(&dir, &path),
                None => assets::load(&PathBuf::new(), "/missing"),
            };
            let served = match shell.store.panel_network(&id) {
                Some(network) => {
                    Served::Panel(assets::panel_csp(&assets::panel_source(form, &id), &network))
                }
                None => Served::Plugin,
            };
            respond(asset, form, served)
        })
        .setup(move |app| {
            let exe_dir = std::env::current_exe()?
                .parent()
                .map(PathBuf::from)
                .unwrap_or_default();
            let core_plugins = resource(app, "KUMIKIT_BUNDLED_CORE_PLUGINS", "core-plugins")
                .filter(|d| d.is_dir());
            let local = LocalCore::from_env(&env, local::current_uid(), &exe_dir, core_plugins);
            let client = local::client_data_dir(Platform::current(), &env)
                .ok_or("cannot find a client data directory; set KUMIKIT_CLIENT_DATA_DIR")?;
            let store = Store {
                bundled: resource(app, "KUMIKIT_BUNDLED_UI_PLUGINS", "ui-plugins"),
                installed: client.join("ui-plugins"),
            };
            let registry = ClientRegistry::new(&client, store.clone(), &env);
            let ui = resource(app, "KUMIKIT_UI_DIR", "ui").unwrap_or_default();
            let connections = Connections {
                profiles: Profiles::new(&client),
                secrets: TokenStore::for_platform(
                    &client,
                    env("KUMIKIT_CLIENT_SECRETS").as_deref() == Some("file"),
                ),
                ssh: kumikit_shell::ssh::program(env),
                keepalive: keepalive(),
                device_name: local::host_name(),
            };
            app.manage(Shell {
                bridge: Bridge::default(),
                local,
                store,
                registry,
                connections,
                ui,
            });
            let query = if safe { "?safe" } else { "" };
            let mut window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::CustomProtocol(
                    format!("{}/index.html{query}", form.origin(APP_SCHEME)).parse()?,
                ),
            )
            .title("Kumikit")
            .inner_size(1100.0, 760.0);
            // The webview's storage lives in the client data directory, unless WebView2 was
            // told where to keep it: msedgedriver does that (through these variables) and
            // then waits for a file in that folder.
            let driven = env("WEBVIEW2_USER_DATA_FOLDER").is_some()
                || env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
                    .is_some_and(|a| a.contains("--user-data-dir") || a.contains("--remote-debugging"));
            #[cfg(feature = "test-hooks")]
            {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!(
                        "kumikit test-hooks: WEBVIEW2_USER_DATA_FOLDER={:?} WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS={:?}",
                        env("WEBVIEW2_USER_DATA_FOLDER"),
                        env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
                    );
                }
            }
            // Wry sets its own WebView2 arguments, which replace the ones in the
            // environment; pass the driver's on (they include its debugging port).
            if let Some(args) = env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
                window = window.additional_browser_args(&args);
            }
            if !driven {
                window = window.data_directory(client.join("webview"));
            }
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("kumikit: {e}");
        }
        std::process::exit(1);
    }
}
