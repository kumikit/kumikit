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
use tauri::http::{Request, Response};
use tauri::ipc::Channel;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};

/// What the commands share.
struct Shell {
    bridge: Bridge,
    local: LocalCore,
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

fn main() {
    let form = SchemeForm::current();
    let ui_dir = env("KUMIKIT_UI_DIR").map(PathBuf::from);
    let plugins_dir = env("KUMIKIT_UI_PLUGINS_DIR").map(PathBuf::from);
    let app_ui = ui_dir.clone();
    // The open dialog only: packages and workspace folders (ADR 0013).
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(feature = "test-hooks")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bridge_connect,
        bridge_send,
        bridge_close,
        test_report
    ]);
    #[cfg(not(feature = "test-hooks"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bridge_connect,
        bridge_send,
        bridge_close
    ]);
    let result = builder
        .register_uri_scheme_protocol(APP_SCHEME, move |ctx, request: Request<Vec<u8>>| {
            let root = app_ui
                .clone()
                .or_else(|| {
                    ctx.app_handle()
                        .path()
                        .resource_dir()
                        .ok()
                        .map(|d| d.join("ui"))
                })
                .unwrap_or_default();
            respond(assets::load(&root, request.uri().path()), form, false)
        })
        .register_uri_scheme_protocol(PLUGIN_SCHEME, move |_ctx, request: Request<Vec<u8>>| {
            let uri = request.uri();
            let host = uri.host().unwrap_or("");
            let asset = match (
                plugins_dir.as_ref(),
                assets::plugin_target(form, host, uri.path()),
            ) {
                (Some(dir), Some((id, path))) => assets::load(&dir.join(id), &path),
                _ => assets::load(&PathBuf::new(), "/"),
            };
            respond(asset, form, true)
        })
        .setup(move |app| {
            let exe_dir = std::env::current_exe()?
                .parent()
                .map(PathBuf::from)
                .unwrap_or_default();
            let bundled = app
                .path()
                .resource_dir()
                .ok()
                .map(|d| d.join("core-plugins"))
                .filter(|d| d.is_dir());
            let local = LocalCore::from_env(&env, local::current_uid(), &exe_dir, bundled);
            app.manage(Shell {
                bridge: Bridge::default(),
                local,
            });
            let mut window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::CustomProtocol(
                    format!("{}/index.html", form.origin(APP_SCHEME)).parse()?,
                ),
            )
            .title("Kumikit")
            .inner_size(1100.0, 760.0);
            if let Some(dir) = local::client_data_dir(Platform::current(), &env) {
                window = window.data_directory(dir.join("webview"));
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
