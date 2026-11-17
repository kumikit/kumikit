//! Serving files to the webview: the UI kernel from `kumikit-app://` and UI plugins from
//! `kumikit-plugin://<id>/` (`docs/spec/06-client.md#shell-rust-kumikit-shell`).
//!
//! Everything here is plain functions over paths and strings, so it is tested without a
//! window. `main.rs` turns an [`Asset`] into a Tauri response.

use std::path::{Component, Path, PathBuf};

/// The scheme the UI kernel is served from.
pub const APP_SCHEME: &str = "kumikit-app";
/// The scheme UI plugins are served from.
pub const PLUGIN_SCHEME: &str = "kumikit-plugin";

/// How the webview exposes custom schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeForm {
    /// `<scheme>://<host>/` (WebKitGTK, WKWebView).
    Native,
    /// `http://<scheme>.localhost/` (WebView2).
    Localhost,
}

impl SchemeForm {
    /// The form the webview on this platform uses.
    pub fn current() -> Self {
        if cfg!(windows) {
            SchemeForm::Localhost
        } else {
            SchemeForm::Native
        }
    }

    /// The origin of a scheme, without a trailing slash.
    pub fn origin(self, scheme: &str) -> String {
        match self {
            SchemeForm::Native => format!("{scheme}://localhost"),
            SchemeForm::Localhost => format!("http://{scheme}.localhost"),
        }
    }

    /// The base URL of a plugin's files, with a trailing slash. On native schemes the
    /// plugin id is the host (`kumikit-plugin://<id>/`); with the localhost form it is
    /// the first path segment.
    pub fn plugin_base(self, id: &str) -> String {
        match self {
            SchemeForm::Native => format!("{PLUGIN_SCHEME}://{id}/"),
            SchemeForm::Localhost => format!("http://{PLUGIN_SCHEME}.localhost/{id}/"),
        }
    }

    /// The scheme-relative sources a CSP lists for a scheme.
    fn csp_source(self, scheme: &str) -> String {
        match self {
            SchemeForm::Native => format!("{scheme}:"),
            SchemeForm::Localhost => format!("http://{scheme}.localhost"),
        }
    }
}

/// The Content-Security-Policy sent with every response.
pub fn csp(form: SchemeForm) -> String {
    let app = form.csp_source(APP_SCHEME);
    let plugin = form.csp_source(PLUGIN_SCHEME);
    // Tauri's IPC: `ipc:` on native schemes, `http://ipc.localhost` on WebView2.
    let ipc = "ipc: http://ipc.localhost";
    format!(
        "default-src 'none'; script-src {app} {plugin}; style-src {app} {plugin}; \
         img-src {app} {plugin} data:; font-src {app} {plugin}; connect-src {app} {plugin} {ipc}; \
         base-uri 'none'; form-action 'none'; frame-src 'none'; object-src 'none'"
    )
}

/// A file to answer a request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// HTTP status.
    pub status: u16,
    /// `Content-Type`.
    pub content_type: &'static str,
    /// The body.
    pub body: Vec<u8>,
}

impl Asset {
    fn not_found() -> Self {
        Asset {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
        }
    }
}

/// Splits a plugin URL into the plugin id and the path inside it.
pub fn plugin_target(form: SchemeForm, host: &str, path: &str) -> Option<(String, String)> {
    match form {
        SchemeForm::Native => Some((host.to_owned(), path.to_owned())),
        SchemeForm::Localhost => {
            let rest = path.trim_start_matches('/');
            let (id, inner) = rest.split_once('/').unwrap_or((rest, ""));
            (!id.is_empty()).then(|| (id.to_owned(), format!("/{inner}")))
        }
    }
}

/// Reads the file `url_path` names under `root`. `/` and directories serve
/// `index.html`. Paths that leave `root` (including through symlinks) are not found.
pub fn load(root: &Path, url_path: &str) -> Asset {
    let Some(relative) = relative_path(url_path) else {
        return Asset::not_found();
    };
    let Ok(root) = root.canonicalize() else {
        return Asset::not_found();
    };
    let mut file = root.join(&relative);
    if file.is_dir() {
        file.push("index.html");
    }
    let Ok(file) = file.canonicalize() else {
        return Asset::not_found();
    };
    if !file.starts_with(&root) {
        return Asset::not_found();
    }
    match std::fs::read(&file) {
        Ok(body) => Asset {
            status: 200,
            content_type: content_type(&file),
            body,
        },
        Err(_) => Asset::not_found(),
    }
}

/// Decodes a URL path into a relative path of normal components, or `None` if it has
/// `..`, a root, a drive prefix, a backslash or bad percent-encoding.
fn relative_path(url_path: &str) -> Option<PathBuf> {
    let path = url_path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode(path)?;
    if decoded.contains('\\') || decoded.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in decoded.split('/').filter(|p| !p.is_empty() && *p != ".") {
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(c)), None) => out.push(c),
            _ => return None,
        }
    }
    Some(out)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ui/sub")).unwrap();
        std::fs::write(dir.path().join("ui/index.html"), "<p>hi</p>").unwrap();
        std::fs::write(dir.path().join("ui/sub/a b.js"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "no").unwrap();
        dir
    }

    #[test]
    fn serves_files_and_index() {
        let dir = tree();
        let root = dir.path().join("ui");
        let index = load(&root, "/");
        assert_eq!(index.status, 200);
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert_eq!(index.body, b"<p>hi</p>");
        let js = load(&root, "/sub/a%20b.js?v=1");
        assert_eq!(js.status, 200);
        assert_eq!(js.content_type, "text/javascript; charset=utf-8");
    }

    #[test]
    fn refuses_paths_outside_the_root() {
        let dir = tree();
        let root = dir.path().join("ui");
        for path in [
            "/../secret.txt",
            "/sub/../../secret.txt",
            "/%2e%2e/secret.txt",
            "/..%2fsecret.txt",
            "/sub\\..\\..\\secret.txt",
            "/%zz",
            "/missing.js",
        ] {
            assert_eq!(load(&root, path).status, 404, "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_out_of_the_root() {
        let dir = tree();
        let root = dir.path().join("ui");
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("link.txt")).unwrap();
        assert_eq!(load(&root, "/link.txt").status, 404);
    }

    #[test]
    fn plugin_urls_per_form() {
        assert_eq!(
            SchemeForm::Native.plugin_base("community.example"),
            "kumikit-plugin://community.example/"
        );
        assert_eq!(
            SchemeForm::Localhost.plugin_base("a.b"),
            "http://kumikit-plugin.localhost/a.b/"
        );
        assert_eq!(
            plugin_target(SchemeForm::Native, "a.b", "/dist/x.js"),
            Some(("a.b".into(), "/dist/x.js".into()))
        );
        assert_eq!(
            plugin_target(
                SchemeForm::Localhost,
                "kumikit-plugin.localhost",
                "/a.b/dist/x.js"
            ),
            Some(("a.b".into(), "/dist/x.js".into()))
        );
        assert_eq!(plugin_target(SchemeForm::Localhost, "h", "/"), None);
    }

    #[test]
    fn csp_allows_no_inline_code() {
        for form in [SchemeForm::Native, SchemeForm::Localhost] {
            let csp = csp(form);
            assert!(csp.starts_with("default-src 'none'"));
            assert!(!csp.contains("unsafe"));
            assert!(csp.contains("frame-src 'none'"));
            // The UI kernel reads bundled.json and theme tokens with fetch.
            assert!(csp.contains(&format!(
                "connect-src {} {}",
                form.csp_source(APP_SCHEME),
                form.csp_source(PLUGIN_SCHEME)
            )));
        }
    }
}
