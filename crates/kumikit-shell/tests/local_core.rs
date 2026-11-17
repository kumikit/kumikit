//! Finding and starting the local core with the real `kumikit` binary.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Duration;

use kumikit_shell::local::{CORE_BIN, Found, LocalCore, LocalError, connect_or_start};
use kumikit_shell::transport::Stream;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Generous; only reached if something hangs.
const DEADLINE: Duration = Duration::from_secs(60);

/// The core binary built next to this test (`cargo test --workspace` builds it).
fn core_bin() -> PathBuf {
    // Tests run from somewhere under the profile directory (`deps/` or `build/…`); the
    // binaries sit in the profile directory itself.
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .skip(1)
        .map(|d| d.join(CORE_BIN))
        .find(|p| p.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no {CORE_BIN} above {}; run `cargo build -p kumikit-core` (or `cargo test --workspace`)",
                exe.display()
            )
        })
}

struct Dirs {
    _root: tempfile::TempDir,
    core: LocalCore,
}

fn dirs() -> Dirs {
    // Unix socket paths are limited to about 104 bytes, and the core wants a private
    // directory.
    let root = tempfile::Builder::new()
        .prefix("kks")
        .tempdir_in(short_tmp())
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(unix)]
    let address = root.path().join("s.sock").to_str().unwrap().to_owned();
    #[cfg(windows)]
    let address = format!(
        r"\\.\pipe\kumikit-test-{}",
        root.path().file_name().unwrap().to_str().unwrap()
    );
    let core = LocalCore {
        address,
        core_bin: core_bin(),
        data_dir: Some(root.path().join("data")),
        bundled_plugins: None,
    };
    Dirs { _root: root, core }
}

fn short_tmp() -> PathBuf {
    if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    }
}

async fn call(stream: &mut BufReader<Stream>, id: u64, method: &str, params: Value) -> Value {
    let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let w = stream.get_mut();
    w.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
    w.flush().await.unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn hello(stream: Stream) -> BufReader<Stream> {
    let mut s = BufReader::new(stream);
    let r = call(
        &mut s,
        1,
        "core.hello",
        json!({"protocol": "0.4", "client": {"name": "shell-test", "version": "0"}}),
    )
    .await;
    assert_eq!(r["result"]["protocol"], "0.4", "{r}");
    s
}

/// Stops the core through the protocol and waits until it has closed the connection.
async fn shutdown(mut s: BufReader<Stream>) {
    let r = call(&mut s, 99, "core.shutdown", json!({})).await;
    assert_eq!(r["result"], json!({}), "{r}");
    let mut rest = String::new();
    assert_eq!(
        s.read_line(&mut rest).await.unwrap(),
        0,
        "connection closed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn starts_the_core_when_none_answers_then_reuses_it() {
    tokio::time::timeout(DEADLINE, async {
        let d = dirs();
        let (stream, found) = connect_or_start(&d.core).await.unwrap();
        assert_eq!(found, Found::Started);
        let first = hello(stream).await;
        drop(first);
        let (stream, found) = connect_or_start(&d.core).await.unwrap();
        assert_eq!(found, Found::Running);
        shutdown(hello(stream).await).await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn two_windows_starting_at_once_share_one_core() {
    tokio::time::timeout(DEADLINE, async {
        let d = dirs();
        let (a, b) = tokio::join!(connect_or_start(&d.core), connect_or_start(&d.core));
        let (a, fa) = a.unwrap();
        let (b, fb) = b.unwrap();
        assert!(
            fa == Found::Started || fb == Found::Started,
            "one of them started it"
        );
        let a = hello(a).await;
        let mut b = hello(b).await;
        // Both reach the same core: a workspace opened on one is listed on the other.
        let ws = d
            .core
            .data_dir
            .clone()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        call(&mut b, 2, "workspace.open", json!({"path": ws})).await;
        let mut a = a;
        let list = call(&mut a, 2, "workspace.list", json!({})).await;
        assert_eq!(list["result"]["workspaces"].as_array().unwrap().len(), 1);
        drop(b);
        shutdown(a).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_missing_core_binary_is_reported() {
    let mut d = dirs();
    d.core.core_bin = d.core.core_bin.with_file_name("no-such-kumikit");
    match connect_or_start(&d.core).await {
        Err(LocalError::MissingBinary(p)) => assert!(p.ends_with("no-such-kumikit")),
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("connected without a core"),
    }
}
