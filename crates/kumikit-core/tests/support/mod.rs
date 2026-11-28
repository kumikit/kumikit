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

    /// What a core started with [`Core::start_logged`] has logged so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.root().join("core.log")).unwrap_or_default()
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
        Core::start_with_args(dirs, dir, &[]).await
    }

    /// Starts the core with extra `serve` arguments.
    pub async fn start_with_args(
        dirs: &Dirs,
        dir: Option<&Path>,
        args: &[&std::ffi::OsStr],
    ) -> (Core, Duration) {
        Core::start_with_env(dirs, dir, args, &[]).await
    }

    /// Starts the core with extra `serve` arguments and environment variables.
    pub async fn start_with_env(
        dirs: &Dirs,
        dir: Option<&Path>,
        args: &[&std::ffi::OsStr],
        env: &[(&str, &str)],
    ) -> (Core, Duration) {
        let started = std::time::Instant::now();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kumikit"));
        command.envs(env.iter().copied());
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
            .args(args)
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

    /// Starts the core with its log at debug level in `<root>/core.log` (read it with
    /// [`Dirs::log`]), with `dir` first on its `PATH`.
    pub async fn start_logged(dirs: &Dirs, dir: Option<&Path>) -> Core {
        let file = std::fs::File::create(dirs.root().join("core.log")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_kumikit"));
        if let Some(dir) = dir {
            let mut paths = vec![dir.to_path_buf()];
            if let Some(p) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&p));
            }
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
        let mut child = command
            .env("KUMIKIT_LOG", "debug")
            .arg("serve")
            .arg("--socket")
            .arg(&dirs.address)
            .arg("--data-dir")
            .arg(dirs.data())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(file))
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kumikit");
        let line = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .next_line()
            .await
            .unwrap()
            .expect("kumikit exited before it was ready");
        assert_eq!(line, format!("ready {}", dirs.address));
        Core { child }
    }

    /// Starts the core with remote access on loopback (`--listen 127.0.0.1:0`) plus `args`,
    /// and reads its `remote` and `ready` lines. With `log`, stderr goes to
    /// `<root>/core.log` at trace level instead of the test's output.
    pub async fn start_remote(dirs: &Dirs, args: &[&str], log: bool) -> (Core, RemoteInfo) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kumikit"));
        command
            .arg("serve")
            .arg("--socket")
            .arg(&dirs.address)
            .arg("--data-dir")
            .arg(dirs.data())
            .args(["--listen", "127.0.0.1:0"])
            .args(args)
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        if log {
            let file = std::fs::File::create(dirs.root().join("core.log")).unwrap();
            command
                .env("KUMIKIT_LOG", "trace")
                .stderr(Stdio::from(file));
        } else {
            command.stderr(Stdio::inherit());
        }
        let mut child = command.spawn().expect("spawn kumikit");
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let remote = lines.next_line().await.unwrap().expect("no remote line");
        let ready = lines.next_line().await.unwrap().expect("no ready line");
        assert_eq!(ready, format!("ready {}", dirs.address));
        let rest = remote.strip_prefix("remote ").expect("remote line first");
        let (url, fingerprint) = rest.split_once(' ').unwrap();
        let addr = url
            .strip_prefix("wss://")
            .and_then(|u| u.strip_suffix("/rpc"))
            .unwrap()
            .to_owned();
        (
            Core { child },
            RemoteInfo {
                url: url.to_owned(),
                addr,
                fingerprint: fingerprint.to_owned(),
            },
        )
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

    /// Waits for the core to exit on its own; returns whether it exited successfully.
    pub async fn wait(mut self) -> bool {
        self.child.wait().await.unwrap().success()
    }

    /// Kills the core without letting it shut down.
    pub async fn crash(mut self) {
        self.child.kill().await.unwrap();
    }
}

/// What a core with remote access printed.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    /// `wss://127.0.0.1:<port>/rpc`.
    pub url: String,
    /// `127.0.0.1:<port>`.
    pub addr: String,
    /// `SHA256:…`.
    pub fingerprint: String,
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
            json!({"protocol":"0.13", "client": {"name": "kumikit-test", "version": "0.0.0"}}),
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

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use kumikit_protocol::remote::parse_fingerprint;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

// ---- a TLS client pinned to the core's certificate ----

#[derive(Debug)]
struct Pinned {
    sha256: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if got == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate changed".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub async fn tls_with(
    info: &RemoteInfo,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> Result<TlsStream<TcpStream>, std::io::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(versions)
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            sha256: parse_fingerprint(&info.fingerprint).unwrap(),
            provider,
        }))
        .with_no_client_auth();
    let tcp = TcpStream::connect(&info.addr).await?;
    tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
}

pub async fn tls(info: &RemoteInfo) -> TlsStream<TcpStream> {
    tls_with(info, &[&rustls::version::TLS13]).await.unwrap()
}

pub type Ws = WebSocketStream<TlsStream<TcpStream>>;

pub async fn ws(info: &RemoteInfo) -> Ws {
    tokio_tungstenite::client_async(info.url.as_str(), tls(info).await)
        .await
        .unwrap()
        .0
}

/// Opens a remote connection, sends one request as its first message, and returns the
/// response.
pub async fn remote_call_once(info: &RemoteInfo, method: &str, params: Value) -> Value {
    let mut ws = ws(info).await;
    ws.send(Message::text(
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
    ))
    .await
    .unwrap();
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).unwrap(),
            Some(Ok(_)) => {}
            other => panic!("connection ended: {other:?}"),
        }
    }
}
