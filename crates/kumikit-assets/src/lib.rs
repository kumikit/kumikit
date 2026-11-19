//! Serving UI files: the UI kernel and UI plugins, to the desktop shell's webview
//! (`kumikit-app://`, `kumikit-plugin://<id>/`; `docs/spec/06-client.md#shell-rust-kumikit-shell`)
//! and, from the core, to a browser (`docs/spec/06-client.md#browser-client`).
//!
//! Everything here is plain functions over paths and strings, with no workspace
//! dependencies, so the shell and the core share the path checks, MIME types and CSP.

use std::path::{Component, Path, PathBuf};

pub mod ui_manifest;

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
         base-uri 'none'; form-action 'none'; frame-src {plugin}; object-src 'none'"
    )
}

/// The CSP source that covers one plugin's files and nothing else: the plugin's host on
/// native schemes, its path on the localhost form (`docs/spec/07-ui-plugins.md#level-2-sandboxed-panels`).
pub fn panel_source(form: SchemeForm, id: &str) -> String {
    match form {
        SchemeForm::Native => format!("{PLUGIN_SCHEME}://{id}"),
        SchemeForm::Localhost => format!("http://{PLUGIN_SCHEME}.localhost/{id}/"),
    }
}

/// Whether `host` may appear in a panel's `[ui] network`: a lowercase DNS name (no
/// scheme, port, path, wildcard or CSP syntax).
pub fn valid_network_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').count() >= 2
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// The CSP of a sandboxed panel's files: code, styles, fonts and images only from `own`
/// (see [`panel_source`]), network only to the `https` hosts the plugin declares. Hosts
/// that are not plain DNS names are left out.
pub fn panel_csp(own: &str, network: &[String]) -> String {
    let hosts: Vec<String> = network
        .iter()
        .filter(|h| valid_network_host(h))
        .map(|h| format!("https://{h}"))
        .collect();
    let connect = if hosts.is_empty() {
        "'none'".to_owned()
    } else {
        hosts.join(" ")
    };
    format!(
        "default-src 'none'; script-src {own}; style-src {own}; img-src {own} data:; \
         font-src {own}; connect-src {connect}; base-uri 'none'; form-action 'none'; \
         frame-src 'none'; object-src 'none'"
    )
}

/// The Content-Security-Policy of the browser client, served by a core over HTTPS
/// (`docs/spec/06-client.md#browser-client`): everything from the core's own origin, which
/// covers `wss:` to it, nothing inline, and no framing by other pages.
pub const WEB_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
    font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-src 'self'; \
    frame-ancestors 'none'; object-src 'none'";

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
            assert!(csp.contains(&format!("frame-src {}", form.csp_source(PLUGIN_SCHEME))));
            // The UI kernel reads bundled.json and theme tokens with fetch.
            assert!(csp.contains(&format!(
                "connect-src {} {}",
                form.csp_source(APP_SCHEME),
                form.csp_source(PLUGIN_SCHEME)
            )));
        }
    }

    #[test]
    fn panel_csp_is_confined_to_its_plugin() {
        let own = panel_source(SchemeForm::Native, "acme.graph");
        assert_eq!(own, "kumikit-plugin://acme.graph");
        let csp = panel_csp(&own, &[]);
        assert!(csp.starts_with("default-src 'none'"));
        assert!(csp.contains("script-src kumikit-plugin://acme.graph;"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(csp.contains("frame-src 'none'"));
        assert!(!csp.contains("unsafe"));
        assert!(!csp.contains("ipc"));
        assert_eq!(
            panel_source(SchemeForm::Localhost, "acme.graph"),
            "http://kumikit-plugin.localhost/acme.graph/"
        );
    }

    #[test]
    fn panel_network_lists_only_plain_hosts() {
        let hosts = [
            "api.github.com",
            "https://evil.example",
            "*.example.com",
            "a.example; script-src *",
            "localhost",
            "x.example:8080",
            "UPPER.example",
        ]
        .map(String::from);
        let csp = panel_csp("kumikit-plugin://a.b", &hosts);
        assert!(csp.contains("connect-src https://api.github.com;"), "{csp}");
        for bad in ["evil", "*", "script-src *", "localhost", "8080", "UPPER"] {
            assert!(!csp.contains(bad), "{bad} in {csp}");
        }
    }
}
