//! The desktop app: one window around the UI kernel.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::path::PathBuf;

use kumikit_shell::assets::{self, APP_SCHEME, Asset, PLUGIN_SCHEME, SchemeForm};
use tauri::http::{Request, Response};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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

/// Test hook: the page reports a line on the shell's stdout.
#[cfg(feature = "test-hooks")]
#[tauri::command]
fn test_report(text: String) {
    #[allow(clippy::print_stdout)]
    {
        println!("report {text}");
    }
}

fn main() {
    let form = SchemeForm::current();
    let ui_dir = std::env::var_os("KUMIKIT_UI_DIR").map(PathBuf::from);
    let plugins_dir = std::env::var_os("KUMIKIT_UI_PLUGINS_DIR").map(PathBuf::from);
    let app_ui = ui_dir.clone();
    let builder = tauri::Builder::default();
    #[cfg(feature = "test-hooks")]
    let builder = builder.invoke_handler(tauri::generate_handler![test_report]);
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
            let url = format!("{}/index.html", form.origin(APP_SCHEME)).parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(url))
                .title("Kumikit")
                .inner_size(1100.0, 760.0)
                .build()?;
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
