//! The desktop app: one window around the UI kernel. All logic is in the library; this
//! file wires it into Tauri.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::path::PathBuf;

use kumikit_protocol::local::Platform;
use kumikit_shell::assets::{self, APP_SCHEME, Asset, PLUGIN_SCHEME, SchemeForm};
use kumikit_shell::bridge::{Bridge, Incoming};
use kumikit_shell::local::{self, LocalCore};
use kumikit_shell::store::{Entry, Store};
use tauri::http::{Request, Response};
use tauri::ipc::Channel;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};

/// What the commands share.
struct Shell {
    bridge: Bridge,
    local: LocalCore,
    store: Store,
    /// The UI kernel's files.
    ui: PathBuf,
}

fn respond(asset: Asset, form: SchemeForm, cors: bool) -> Response<Cow<'static, [u8]>> {
    let mut builder = Response::builder()
        .status(asset.status)
        .header("Content-Type", asset.content_type)
        .header("Content-Security-Policy", assets::csp(form))
        .header("X-Content-Type-Options", "nosniff");
    if cors {
        // Plugin modules are imported from the app origin.
        builder = builder.header("Access-Control-Allow-Origin", form.origin(APP_SCHEME));
    }
    builder
        .body(Cow::Owned(asset.body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
}

/// Opens a connection to a profile's core. M2 knows only `local`.
#[tauri::command]
async fn bridge_connect(
    shell: State<'_, Shell>,
    profile: String,
    channel: Channel<Incoming>,
) -> Result<String, String> {
    if profile != "local" {
        return Err(format!("unknown connection profile {profile}"));
    }
    let (stream, found) = local::connect_or_start(&shell.local)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(feature = "test-hooks")]
    {
        #[allow(clippy::print_stdout)]
        {
            println!("ui-connected {found:?}");
        }
    }
    #[cfg(not(feature = "test-hooks"))]
    let _ = found;
    Ok(shell.bridge.attach(stream, move |m| {
        // A closed channel means the page went away; the connection closes with it.
        let _ = channel.send(m);
    }))
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
        client_plugins_remove,
        test_report
    ]);
    #[cfg(not(feature = "test-hooks"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bridge_connect,
        bridge_send,
        bridge_close,
        client_plugins_list,
        client_plugins_install,
        client_plugins_remove
    ]);
    let result = builder
        .register_uri_scheme_protocol(APP_SCHEME, move |ctx, request: Request<Vec<u8>>| {
            let shell = ctx.app_handle().state::<Shell>();
            respond(assets::load(&shell.ui, request.uri().path()), form, false)
        })
        .register_uri_scheme_protocol(PLUGIN_SCHEME, move |ctx, request: Request<Vec<u8>>| {
            let shell = ctx.app_handle().state::<Shell>();
            let uri = request.uri();
            let host = uri.host().unwrap_or("");
            let asset = match assets::plugin_target(form, host, uri.path())
                .and_then(|(id, path)| shell.store.dir(&id).map(|d| (d, path)))
            {
                Some((dir, path)) => assets::load(&dir, &path),
                None => assets::load(&PathBuf::new(), "/missing"),
            };
            respond(asset, form, true)
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
            let ui = resource(app, "KUMIKIT_UI_DIR", "ui").unwrap_or_default();
            app.manage(Shell {
                bridge: Bridge::default(),
                local,
                store,
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
