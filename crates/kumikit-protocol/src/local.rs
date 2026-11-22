//! The local transport's address per platform (`docs/spec/02-protocol.md#local-transport-details`).
//!
//! Pure functions over an injected environment lookup, shared by the core (which listens
//! there) and the desktop shell (which connects there), so both always agree.

/// Looks up an environment variable; empty values count as unset.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

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
            // Unix paths are built as strings with `/`, not `PathBuf::join`, so the result
            // does not depend on the separator of the machine computing it.
            let dir = match env("XDG_RUNTIME_DIR") {
                Some(run) => format!("{}/kumikit", run.trim_end_matches('/')),
                None => {
                    let base = match platform {
                        Platform::MacOs => env("TMPDIR").unwrap_or_else(|| "/tmp".into()),
                        _ => "/tmp".into(),
                    };
                    format!("{}/kumikit-{uid}", base.trim_end_matches('/'))
                }
            };
            format!("{dir}/core.sock")
        }
    }
}

/// The default Kumikit home directory where Kumikit creates user-visible directories such
/// as git worktrees (`<home>/worktrees`).
///
/// Honors `KUMIKIT_HOME` when set; otherwise defaults to `~/.kumikit` on Linux and macOS
/// and `%USERPROFILE%\.kumikit` (falling back to `%HOMEDRIVE%%HOMEPATH%\.kumikit`) on
/// Windows. Returns `None` if none of those variables are set.
pub fn default_home_dir(platform: Platform, env: Env<'_>) -> Option<String> {
    if let Some(custom) = env("KUMIKIT_HOME") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    match platform {
        Platform::Linux | Platform::MacOs => env("HOME").map(|h| {
            let base = h.trim_end_matches('/');
            if base.is_empty() {
                "/.kumikit".to_owned()
            } else {
                format!("{base}/.kumikit")
            }
        }),
        Platform::Windows => env("USERPROFILE")
            .or_else(|| {
                let drive = env("HOMEDRIVE")?;
                let path = env("HOMEPATH")?;
                Some(format!("{drive}{path}"))
            })
            .map(|h| format!(r"{}\.kumikit", h.trim_end_matches(['\\', '/']))),
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
    fn home_dirs() {
        let e = env(&[("HOME", "/home/u")]);
        assert_eq!(
            default_home_dir(Platform::Linux, &e).as_deref(),
            Some("/home/u/.kumikit")
        );
        let e = env(&[("HOME", "/Users/u/")]);
        assert_eq!(
            default_home_dir(Platform::MacOs, &e).as_deref(),
            Some("/Users/u/.kumikit")
        );
        let e = env(&[("USERPROFILE", r"C:\Users\u\")]);
        assert_eq!(
            default_home_dir(Platform::Windows, &e).as_deref(),
            Some(r"C:\Users\u\.kumikit")
        );
        let e = env(&[("HOMEDRIVE", "C:"), ("HOMEPATH", r"\Users\u")]);
        assert_eq!(
            default_home_dir(Platform::Windows, &e).as_deref(),
            Some(r"C:\Users\u\.kumikit")
        );
        let e = env(&[("HOME", "/home/u"), ("KUMIKIT_HOME", "/srv/kk")]);
        assert_eq!(
            default_home_dir(Platform::Linux, &e).as_deref(),
            Some("/srv/kk")
        );
        assert_eq!(default_home_dir(Platform::Linux, &env(&[])), None);
    }
}
