//! Default data directory per platform (`docs/spec/01-architecture.md#data-directories`);
//! the local address comes from `kumikit_protocol::local`.
//!
//! The functions take an environment lookup so tests can check every platform's rules.

use std::path::PathBuf;

pub use kumikit_protocol::local::{Env, Platform, default_address};

/// The process environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
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
    fn host_name_is_not_empty() {
        assert!(!host_name().is_empty());
    }
}
