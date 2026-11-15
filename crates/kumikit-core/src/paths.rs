//! Default data directory and local address per platform
//! (`docs/spec/01-architecture.md#data-directories`, `docs/spec/02-protocol.md`).
//!
//! The functions take an environment lookup so tests can check every platform's rules.

use std::path::PathBuf;

/// Looks up an environment variable; empty values count as unset.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Which platform's rules to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux and other Unix systems.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl Platform {
    /// The platform this binary runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

/// The core's data directory, or `None` if the needed variables are unset.
pub fn data_dir(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    match platform {
        Platform::Linux => env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("kumikit").join("core")),
        Platform::MacOs => env("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join("dev.kumikit")
                .join("core")
        }),
        Platform::Windows => env("APPDATA").map(|d| PathBuf::from(d).join("kumikit").join("core")),
    }
}

/// The default local address: a socket path on Unix, a pipe name on Windows. `uid` is
/// the user id on Unix (ignored on Windows).
pub fn default_address(platform: Platform, env: Env<'_>, uid: u32) -> String {
    match platform {
        Platform::Windows => {
            let user = env("USERNAME").unwrap_or_else(|| "user".into());
            let safe: String = user
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!(r"\\.\pipe\kumikit-{safe}")
        }
        Platform::Linux | Platform::MacOs => {
            let dir = match env("XDG_RUNTIME_DIR") {
                Some(run) => PathBuf::from(run).join("kumikit"),
                None => {
                    let base = match platform {
                        Platform::MacOs => env("TMPDIR").unwrap_or_else(|| "/tmp".into()),
                        _ => "/tmp".into(),
                    };
                    PathBuf::from(base).join(format!("kumikit-{uid}"))
                }
            };
            dir.join("core.sock").to_string_lossy().into_owned()
        }
    }
}

/// This machine's name for `core.hello`.
pub fn host_name() -> String {
    #[cfg(unix)]
    {
        rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(unix))]
    {
        process_env("COMPUTERNAME").unwrap_or_else(|| "localhost".into())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn data_dirs() {
        let e = env(&[("HOME", "/home/u")]);
        assert_eq!(
            data_dir(Platform::Linux, &e).unwrap(),
            PathBuf::from("/home/u/.local/share/kumikit/core")
        );
        let e = env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(
            data_dir(Platform::Linux, &e).unwrap(),
            PathBuf::from("/data/kumikit/core")
        );
        let e = env(&[("HOME", "/Users/u")]);
        assert_eq!(
            data_dir(Platform::MacOs, &e).unwrap(),
            PathBuf::from("/Users/u/Library/Application Support/dev.kumikit/core")
        );
        let e = env(&[("APPDATA", "C:/Users/u/AppData/Roaming")]);
        assert_eq!(
            data_dir(Platform::Windows, &e).unwrap(),
            PathBuf::from("C:/Users/u/AppData/Roaming/kumikit/core")
        );
        assert_eq!(data_dir(Platform::Linux, &env(&[])), None);
    }

    #[test]
    fn addresses() {
        let e = env(&[("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert_eq!(
            default_address(Platform::Linux, &e, 1000),
            "/run/user/1000/kumikit/core.sock"
        );
        assert_eq!(
            default_address(Platform::Linux, &env(&[]), 1000),
            "/tmp/kumikit-1000/core.sock"
        );
        let e = env(&[("TMPDIR", "/var/folders/x/T/")]);
        assert_eq!(
            default_address(Platform::MacOs, &e, 501),
            "/var/folders/x/T/kumikit-501/core.sock"
        );
        let e = env(&[("USERNAME", "Ana Lee")]);
        assert_eq!(
            default_address(Platform::Windows, &e, 0),
            r"\\.\pipe\kumikit-Ana_Lee"
        );
    }

    #[test]
    fn host_name_is_not_empty() {
        assert!(!host_name().is_empty());
    }
}
