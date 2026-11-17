//! Cores reached through the system `ssh` (`docs/spec/02-protocol.md#stdio-over-ssh`,
//! [ADR 0017](../../../docs/decisions/0017-shell-remote-profiles-and-ssh.md)).
//!
//! The shell runs `ssh -T -o BatchMode=yes … -- <destination> <command>` with no shell in
//! between; on the far side `kumikit bridge` copies lines to that machine's local core.
//! `BatchMode` means ssh never prompts: a window has no terminal to answer in.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::process::{Child, Command};

/// The remote command when a profile names none.
pub const DEFAULT_COMMAND: &str = "kumikit bridge";

/// The `ssh` program: `KUMIKIT_SSH`, or `ssh` found on `PATH` when started.
pub fn program(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    env("KUMIKIT_SSH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "ssh.exe" } else { "ssh" }))
}

/// Why a destination or command is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SshError {
    /// The destination could be read as an option, or holds whitespace or control
    /// characters.
    #[error(
        "an SSH destination is a host name, user@host, or a Host from ~/.ssh/config, without spaces and not starting with -"
    )]
    BadDestination,
    /// The remote command holds control characters or is empty.
    #[error("the remote command must be one line")]
    BadCommand,
}

/// Checks a destination: not empty, no leading `-`, no whitespace or control characters.
pub fn check_destination(destination: &str) -> Result<(), SshError> {
    let ok = !destination.is_empty()
        && destination.len() <= 255
        && !destination.starts_with('-')
        && !destination
            .chars()
            .any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(())
    } else {
        Err(SshError::BadDestination)
    }
}

/// Checks a remote command: one non-empty line.
pub fn check_command(command: &str) -> Result<(), SshError> {
    if command.trim().is_empty() || command.chars().any(char::is_control) {
        Err(SshError::BadCommand)
    } else {
        Ok(())
    }
}

/// The arguments given to `ssh`, exactly.
pub fn args(destination: &str, command: &str) -> Result<Vec<String>, SshError> {
    check_destination(destination)?;
    check_command(command)?;
    Ok([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "--",
        destination,
        command,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}

/// A running `ssh`: its stdio as one stream, and what it wrote on stderr.
#[derive(Debug)]
pub struct SshStream {
    /// ssh's stdout: what the core sends.
    pub stdout: tokio::process::ChildStdout,
    /// ssh's stdin: what the client sends. Dropping it ends the connection.
    pub stdin: tokio::process::ChildStdin,
    /// ssh's stderr, for explaining a failure.
    pub stderr: Stderr,
}

/// What ssh wrote on stderr, and whether it has finished writing.
#[derive(Debug, Clone)]
pub struct Stderr {
    text: Arc<Mutex<String>>,
    done: tokio::sync::watch::Receiver<bool>,
}

impl Stderr {
    /// The reason to show when the connection ended: ssh's own message if it wrote one.
    /// Waits up to 2 s for ssh to finish writing it, since its stdout can close first.
    pub async fn explain(&self, reason: String) -> String {
        let mut done = self.done.clone();
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(2), done.wait_for(|d| *d)).await;
        let s = self.text.lock().unwrap_or_else(|p| p.into_inner());
        if s.trim().is_empty() {
            reason
        } else {
            format!("ssh: {}", s.trim())
        }
    }
}

/// Starts `ssh`. The child is killed when its stream is dropped and it is reaped in the
/// background.
pub fn spawn(program: &Path, destination: &str, command: &str) -> std::io::Result<SshStream> {
    let args = args(destination, command)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut child: Child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            std::io::Error::new(e.kind(), format!("cannot start {}: {e}", program.display()))
        })?;
    let missing = || std::io::Error::other("ssh's pipes are missing");
    let stdin = child.stdin.take().ok_or_else(missing)?;
    let stdout = child.stdout.take().ok_or_else(missing)?;
    let errors = child.stderr.take().ok_or_else(missing)?;
    let text = Arc::new(Mutex::new(String::new()));
    let collected = Arc::clone(&text);
    let (done_tx, done) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let mut lines = BufReader::new(errors).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut s = collected.lock().unwrap_or_else(|p| p.into_inner());
            if s.len() < 4096 {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(line.trim());
            }
        }
        // stderr closed: ssh is exiting. Reap it.
        let _ = done_tx.send(true);
        let _ = child.wait().await;
    });
    Ok(SshStream {
        stdout,
        stdin,
        stderr: Stderr { text, done },
    })
}

/// Checks that an SSH destination reaches a core: sends `core.hello` and waits for any
/// answer (a version mismatch still proves a core is there).
pub async fn check<R, W>(read: R, mut write: W, stderr: &Stderr) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let hello = serde_json::json!({"jsonrpc": "2.0", "id": 0, "method": "core.hello",
        "params": {"protocol": kumikit_protocol::PROTOCOL_VERSION.to_string(),
                   "client": {"name": "kumikit-desktop", "version": env!("CARGO_PKG_VERSION")}}});
    let attempt = async {
        write
            .write_all(format!("{hello}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        write.flush().await.map_err(|e| e.to_string())?;
        let mut lines = BufReader::new(read).lines();
        match lines.next_line().await {
            Ok(Some(line)) if line.contains("\"id\":0") => Ok(()),
            Ok(Some(_)) => Err("the far side did not answer like a Kumikit core".to_owned()),
            _ => Err("the connection closed".to_owned()),
        }
    };
    match tokio::time::timeout(crate::remote::CONNECT_TIMEOUT, attempt).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(reason)) => Err(stderr.explain(reason).await),
        Err(_) => Err(stderr.explain("no answer in time".into()).await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations() {
        for ok in ["devbox", "me@10.0.0.5", "me@[::1]", "host.example:22"] {
            assert_eq!(check_destination(ok), Ok(()), "{ok}");
        }
        for bad in ["", "-oProxyCommand=evil", "a b", "a\nb", "a\tb"] {
            assert_eq!(
                check_destination(bad),
                Err(SshError::BadDestination),
                "{bad:?}"
            );
        }
        assert_eq!(check_command("kumikit bridge"), Ok(()));
        assert_eq!(check_command("  "), Err(SshError::BadCommand));
        assert_eq!(check_command("a\nb"), Err(SshError::BadCommand));
    }

    #[test]
    fn the_arguments_are_exact() {
        assert_eq!(
            args("devbox", DEFAULT_COMMAND).unwrap(),
            [
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "--",
                "devbox",
                "kumikit bridge"
            ]
        );
        assert!(args("-x", DEFAULT_COMMAND).is_err());
    }

    #[test]
    fn the_program_can_be_overridden() {
        assert_eq!(
            program(|_| Some("/x/fake-ssh".into())),
            PathBuf::from("/x/fake-ssh")
        );
        assert!(program(|_| None).to_string_lossy().starts_with("ssh"));
    }
}
