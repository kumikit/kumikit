//! A test harness: starts the real `kumikit` binary and talks to it like a client would.

#![allow(dead_code)] // Not every test file uses every helper.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::process::{Child, Command};

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Upper bound for a whole test, so a broken core fails the test instead of hanging CI.
/// Success never depends on it.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// Temp directories for one test: data, a workspace, and the local address.
pub struct Dirs {
    root: tempfile::TempDir,
    pub address: String,
}

impl Dirs {
    pub fn new() -> Dirs {
        // Unix socket paths are limited to ~104 bytes; keep them short. The directory must
        // be private, since the core refuses a socket directory others can read.
        #[cfg(unix)]
        let root = {
            use std::os::unix::fs::PermissionsExt;
            let d = tempfile::Builder::new()
                .prefix("kk")
                .tempdir_in("/tmp")
                .unwrap();
            std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            d
        };
        #[cfg(windows)]
        let root = tempfile::tempdir().unwrap();

        #[cfg(unix)]
        let address = root.path().join("s.sock").to_str().unwrap().to_owned();
        #[cfg(windows)]
        let address = format!(
            r"\\.\pipe\kumikit-test-{}",
            root.path().file_name().unwrap().to_string_lossy()
        );
        std::fs::create_dir(root.path().join("workspace")).unwrap();
        Dirs { root, address }
    }

    /// The test's temp root (private, short).
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    pub fn workspace(&self) -> String {
        self.root
            .path()
            .join("workspace")
            .to_str()
            .unwrap()
            .to_owned()
    }
}

/// A running `kumikit serve`.
pub struct Core {
    child: Child,
}

impl Core {
    /// Starts the core and waits for its `ready` line.
    pub async fn start(dirs: &Dirs) -> Core {
        Core::start_timed(dirs).await.0
    }

    /// The process id.
    pub fn pid(&self) -> u32 {
        self.child.id().expect("core is running")
    }

    /// Starts the core; also returns the time from spawn to the `ready` line.
    pub async fn start_timed(dirs: &Dirs) -> (Core, Duration) {
        Core::start_with_path(dirs, None).await
    }

    /// Starts the core with `dir` first on its `PATH` (where test agents live).
    pub async fn start_with_path(dirs: &Dirs, dir: Option<&Path>) -> (Core, Duration) {
        let started = std::time::Instant::now();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kumikit"));
        if let Some(dir) = dir {
            let mut paths = vec![dir.to_path_buf()];
            if let Some(p) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&p));
            }
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
        let mut child = command
            .arg("serve")
            .arg("--socket")
            .arg(&dirs.address)
            .arg("--data-dir")
            .arg(dirs.data())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kumikit");
        let stdout = child.stdout.take().unwrap();
        let line = BufReader::new(stdout)
            .lines()
            .next_line()
            .await
            .unwrap()
            .expect("kumikit exited before it was ready");
        let elapsed = started.elapsed();
        assert_eq!(line, format!("ready {}", dirs.address));
        (Core { child }, elapsed)
    }

    /// Stops the core: SIGTERM on Unix (graceful), a hard kill on Windows.
    pub async fn stop(mut self) {
        #[cfg(unix)]
        {
            let pid = self
                .child
                .id()
                .and_then(|id| rustix::process::Pid::from_raw(id as i32));
            if let Some(pid) = pid {
                rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
            }
            let status = self.child.wait().await.unwrap();
            assert!(status.success(), "kumikit exited with {status}");
        }
        #[cfg(windows)]
        {
            self.child.kill().await.unwrap();
        }
    }

    /// Kills the core without letting it shut down.
    pub async fn crash(mut self) {
        self.child.kill().await.unwrap();
    }
}

/// A JSON-RPC client over the local transport.
pub struct Client {
    write: WriteHalf<Stream>,
    lines: Lines<BufReader<ReadHalf<Stream>>>,
    next_id: i64,
    /// Notifications that arrived while waiting for a response.
    pending: VecDeque<Value>,
    /// Notifications other than `events.event`, set aside by `event()`.
    notices: VecDeque<Value>,
    /// Every message sent and received, for recording fixtures.
    pub transcript: Vec<Value>,
}

impl Client {
    pub async fn connect(address: &str) -> Client {
        let stream = kumikit_core::transport::connect(address)
            .await
            .expect("connect");
        let (read, write) = tokio::io::split(stream);
        Client {
            write,
            lines: BufReader::new(read).lines(),
            next_id: 0,
            pending: VecDeque::new(),
            notices: VecDeque::new(),
            transcript: Vec::new(),
        }
    }

    /// Connects and completes the handshake.
    pub async fn hello(address: &str) -> Client {
        let mut c = Client::connect(address).await;
        c.call(
            "core.hello",
            json!({"protocol": "0.2", "client": {"name": "kumikit-test", "version": "0.0.0"}}),
        )
        .await;
        c
    }

    async fn recv(&mut self) -> Value {
        let line = self
            .lines
            .next_line()
            .await
            .unwrap()
            .expect("core closed the connection");
        let v: Value = serde_json::from_str(&line).unwrap();
        self.transcript.push(v.clone());
        v
    }

    /// Sends a request and returns the whole response object.
    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.transcript.push(msg.clone());
        let mut line = msg.to_string();
        line.push('\n');
        self.write.write_all(line.as_bytes()).await.unwrap();
        loop {
            let v = self.recv().await;
            if v.get("method").is_some() {
                self.pending.push_back(v);
            } else if v["id"] == id {
                return v;
            }
        }
    }

    /// Sends a request and returns its result; panics on an error response.
    pub async fn call(&mut self, method: &str, params: Value) -> Value {
        let r = self.request(method, params).await;
        assert!(r.get("error").is_none(), "{method} failed: {r}");
        r["result"].clone()
    }

    /// The next `events.event` params. Other notifications are set aside for
    /// [`Client::notification`].
    pub async fn event(&mut self) -> Value {
        loop {
            let v = match self.pending.pop_front() {
                Some(v) => v,
                None => self.recv().await,
            };
            if v["method"] == "events.event" {
                return v["params"].clone();
            }
            assert!(v.get("method").is_some(), "unexpected message {v}");
            self.notices.push_back(v);
        }
    }

    /// The params of the next notification named `method` (other than `events.event`).
    pub async fn notification(&mut self, method: &str) -> Value {
        loop {
            if let Some(i) = self.notices.iter().position(|n| n["method"] == method) {
                return self.notices.remove(i).unwrap()["params"].clone();
            }
            match self.pending.pop_front() {
                Some(v) if v["method"] == method => return v["params"].clone(),
                Some(v) => self.notices.push_back(v),
                None => {
                    let v = self.recv().await;
                    if v["method"] == method {
                        return v["params"].clone();
                    }
                    self.pending.push_back(v);
                }
            }
        }
    }

    /// Events until (and including) the first one of `kind`.
    pub async fn events_until(&mut self, kind: &str) -> Vec<Value> {
        let mut out = Vec::new();
        loop {
            let e = self.event().await;
            let done = e["kind"] == kind;
            out.push(e);
            if done {
                return out;
            }
        }
    }

    /// True if no notification is buffered or readable right now. Used after a
    /// request/response round trip, which proves the core sent nothing in between.
    pub fn no_buffered_events(&self) -> bool {
        self.pending.is_empty()
    }
}

pub fn seqs(events: &[Value]) -> Vec<u64> {
    events.iter().map(|e| e["seq"].as_u64().unwrap()).collect()
}

pub fn kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// Writes a client transcript as pretty JSON, for turning into protocol fixtures.
pub fn record(name: &str, transcript: &[Value]) {
    if let Some(dir) = std::env::var_os("KUMIKIT_RECORD") {
        let path = Path::new(&dir).join(format!("{name}.json"));
        std::fs::write(path, serde_json::to_string_pretty(transcript).unwrap()).unwrap();
    }
}

/// The probe test plugin's manifest (`test-plugins/probe`).
pub const PROBE_MANIFEST: &str = include_str!("../../../../test-plugins/probe/plugin.toml");

/// The probe's qualified provider id.
pub const PROBE: &str = "test.probe/probe";

/// Packs a plugin directory (a manifest plus files) into `<root>/<name>.kkp`.
pub fn kkp(dirs: &Dirs, name: &str, manifest: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let src = dirs.root().join(format!("{name}-src"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("plugin.toml"), manifest).unwrap();
    for (path, data) in files {
        std::fs::write(src.join(path), data).unwrap();
    }
    let out = dirs.root().join(format!("{name}.kkp"));
    kumikit_plugin_host::package::pack(&src, &out).unwrap();
    out
}

/// The probe plugin as a `.kkp`.
pub fn probe_kkp(dirs: &Dirs) -> PathBuf {
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    kkp(dirs, "probe", PROBE_MANIFEST, &[("plugin.wasm", &wasm)])
}

/// Installs and enables a package through the protocol, granting what it requests.
pub async fn install(c: &mut Client, package: &Path) -> Value {
    let installed = c
        .call(
            "plugin.install",
            json!({ "path": package.to_str().unwrap() }),
        )
        .await;
    let id = installed["plugin"]["id"].clone();
    let grant = installed["plugin"]["requested"].clone();
    c.call("plugin.enable", json!({ "plugin": id, "grant": grant }))
        .await["plugin"]
        .clone()
}

/// Runs `kumikit plugin <args>` against this test's core (stdin is not a terminal).
/// Returns success, stdout and stderr.
pub async fn plugin_cli(dirs: &Dirs, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_kumikit"))
        .arg("plugin")
        .args(args)
        .arg("--socket")
        .arg(&dirs.address)
        .arg("--data-dir")
        .arg(dirs.data())
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The provider-acp plugin (`plugins/provider-acp`) as a `.kkp`.
pub fn provider_acp_kkp(dirs: &Dirs) -> PathBuf {
    let dir = Path::new(kumikit_test_plugins::PROVIDER_ACP_DIR);
    let manifest = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let schema = std::fs::read(dir.join("acp.schema.json")).unwrap();
    let wasm = std::fs::read(kumikit_test_plugins::PROVIDER_ACP_WASM).unwrap();
    kkp(
        dirs,
        "provider-acp",
        &manifest,
        &[("plugin.wasm", &wasm), ("acp.schema.json", &schema)],
    )
}
