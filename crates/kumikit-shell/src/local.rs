//! The local core: where it listens, and starting it when no core answers
//! (`docs/spec/06-client.md#shell-rust-kumikit-shell`).
//!
//! The shell starts only the core binary bundled with it, by absolute path, detached so
//! it keeps running when the window closes (ADR 0013).

use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kumikit_protocol::local::{Env, Platform, default_address};

use crate::transport::{self, Stream};

/// The core binary's file name.
pub const CORE_BIN: &str = if cfg!(windows) {
    "kumikit.exe"
} else {
    "kumikit"
};

/// Where the local core is and how to start it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCore {
    /// Socket path (Unix) or pipe name (Windows).
    pub address: String,
    /// The core binary to start.
    pub core_bin: PathBuf,
    /// `--data-dir` for a started core; `None` lets the core use its default.
    pub data_dir: Option<PathBuf>,
    /// `--bundled-plugins` for a started core.
    pub bundled_plugins: Option<PathBuf>,
}

impl LocalCore {
    /// The defaults for this machine, with the overrides in 06-client:
    /// `KUMIKIT_SOCKET`, `KUMIKIT_CORE_DATA_DIR` and `KUMIKIT_CORE_BIN`.
    /// `exe_dir` is the directory of the shell's executable, where the core is bundled.
    pub fn from_env(env: Env<'_>, uid: u32, exe_dir: &Path, bundled: Option<PathBuf>) -> Self {
        LocalCore {
            address: env("KUMIKIT_SOCKET")
                .unwrap_or_else(|| default_address(Platform::current(), env, uid)),
            core_bin: env("KUMIKIT_CORE_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| exe_dir.join(CORE_BIN)),
            data_dir: env("KUMIKIT_CORE_DATA_DIR").map(PathBuf::from),
            bundled_plugins: bundled,
        }
    }
}

/// This computer's name, the default name of a paired device.
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
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Kumikit desktop".into())
    }
}

/// How a connection to the local core came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// A core was already running.
    Running,
    /// This call started the core.
    Started,
}

/// Errors finding or starting the local core.
#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    /// The bundled core binary is missing.
    #[error("the core program is missing: {0}")]
    MissingBinary(PathBuf),
    /// The core could not be started or exited before it was ready.
    #[error("the core did not start: {0}")]
    Start(String),
    /// Connecting failed after the core reported it was ready.
    #[error("cannot connect to the core at {address}: {source}")]
    Connect {
        /// The address tried.
        address: String,
        /// Why.
        source: io::Error,
    },
}

/// Connects to the local core, starting it first if nothing answers.
pub async fn connect_or_start(core: &LocalCore) -> Result<(Stream, Found), LocalError> {
    if let Ok(stream) = transport::connect(&core.address).await {
        return Ok((stream, Found::Running));
    }
    let started = {
        let core = core.clone();
        tokio::task::spawn_blocking(move || start(&core))
            .await
            .map_err(|e| LocalError::Start(e.to_string()))?
    };
    match started {
        Ok(()) => {}
        // Another window may have started a core at the same moment; ours then fails on
        // the data-directory lock or the socket. Use theirs if it answers.
        // The other core holds the lock before it binds, so give it a moment to bind.
        Err(e) => {
            for _ in 0..40 {
                if let Ok(stream) = transport::connect(&core.address).await {
                    return Ok((stream, Found::Running));
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            return Err(e);
        }
    }
    transport::connect(&core.address)
        .await
        .map(|s| (s, Found::Started))
        .map_err(|source| LocalError::Connect {
            address: core.address.clone(),
            source,
        })
}

/// Starts the core detached and waits for its `ready <address>` line. Blocking.
fn start(core: &LocalCore) -> Result<(), LocalError> {
    if !core.core_bin.is_file() {
        return Err(LocalError::MissingBinary(core.core_bin.clone()));
    }
    let mut command = Command::new(&core.core_bin);
    command.arg("serve").arg("--socket").arg(&core.address);
    if let Some(dir) = &core.data_dir {
        command.arg("--data-dir").arg(dir);
    }
    if let Some(dir) = &core.bundled_plugins {
        command.arg("--bundled-plugins").arg(dir);
    }
    // stdout only until `ready`; logs are not kept (a closed stderr would make the core's
    // log writes fail once the shell is gone).
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = spawn_detached(&mut command).map_err(|e| LocalError::Start(e.to_string()))?;
    let Some(stdout) = child.stdout.take() else {
        return Err(LocalError::Start("no stdout".into()));
    };
    let mut line = String::new();
    let read = BufReader::new(stdout).read_line(&mut line);
    // Reap the core whenever it exits, so it never lingers as a zombie of the shell.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    match read {
        Ok(n) if n > 0 && line.trim_end() == format!("ready {}", core.address) => Ok(()),
        Ok(_) => Err(LocalError::Start(
            "the core exited before it was ready".into(),
        )),
        Err(e) => Err(LocalError::Start(e.to_string())),
    }
}

#[cfg(unix)]
fn spawn_detached(command: &mut Command) -> io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    // Its own process group: signals to the shell's group (a terminal's Ctrl+C) do not
    // reach the core.
    command.process_group(0).spawn()
}

#[cfg(windows)]
fn spawn_detached(command: &mut Command) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    // Leave the shell's job object if there is one (a terminal or a test runner), so
    // closing the shell does not kill the core. A job that forbids breaking away refuses
    // the flag; start inside it then.
    let base = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    match command
        .creation_flags(base | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => Ok(child),
        Err(_) => command.creation_flags(base).spawn(),
    }
}

/// The client data directory (`docs/spec/01-architecture.md#data-directories`), or
/// `KUMIKIT_CLIENT_DATA_DIR`.
pub fn client_data_dir(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    if let Some(dir) = env("KUMIKIT_CLIENT_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    match platform {
        Platform::Linux => env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("kumikit").join("client")),
        Platform::MacOs => env("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join("dev.kumikit")
                .join("client")
        }),
        Platform::Windows => {
            env("APPDATA").map(|d| PathBuf::from(d).join("kumikit").join("client"))
        }
    }
}

/// The current user id (Unix); 0 elsewhere, where the address does not use it.
pub fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        rustix::process::getuid().as_raw()
    }
    #[cfg(not(unix))]
    {
        0
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
    fn client_data_dirs() {
        let e = env(&[("HOME", "/home/u")]);
        assert_eq!(
            client_data_dir(Platform::Linux, &e).unwrap(),
            PathBuf::from("/home/u/.local/share/kumikit/client")
        );
        let e = env(&[("HOME", "/Users/u")]);
        assert_eq!(
            client_data_dir(Platform::MacOs, &e).unwrap(),
            PathBuf::from("/Users/u/Library/Application Support/dev.kumikit/client")
        );
        let e = env(&[("APPDATA", "C:/A")]);
        assert_eq!(
            client_data_dir(Platform::Windows, &e).unwrap(),
            PathBuf::from("C:/A/kumikit/client")
        );
        let e = env(&[("HOME", "/h"), ("KUMIKIT_CLIENT_DATA_DIR", "/x")]);
        assert_eq!(
            client_data_dir(Platform::Linux, &e).unwrap(),
            PathBuf::from("/x")
        );
    }

    #[test]
    fn overrides_win_over_defaults() {
        let e = env(&[("XDG_RUNTIME_DIR", "/run/user/7"), ("USERNAME", "u")]);
        let core = LocalCore::from_env(&e, 7, Path::new("/opt/kk"), None);
        assert_eq!(core.address, default_address(Platform::current(), &e, 7));
        assert_eq!(core.core_bin, Path::new("/opt/kk").join(CORE_BIN));
        assert_eq!(core.data_dir, None);
        let e = env(&[
            ("KUMIKIT_SOCKET", "/tmp/s.sock"),
            ("KUMIKIT_CORE_BIN", "/b/kumikit"),
            ("KUMIKIT_CORE_DATA_DIR", "/d"),
        ]);
        let core = LocalCore::from_env(&e, 7, Path::new("/opt/kk"), Some("/r".into()));
        assert_eq!(
            core,
            LocalCore {
                address: "/tmp/s.sock".into(),
                core_bin: "/b/kumikit".into(),
                data_dir: Some("/d".into()),
                bundled_plugins: Some("/r".into()),
            }
        );
    }
}
