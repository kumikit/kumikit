//! Remote profiles against real cores on loopback (`docs/spec/06-client.md`, ADR 0017):
//! pinning, pairing, token login, revocation, keepalive and SSH through a stand-in `ssh`.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use kumikit_shell::bridge::{Bridge, Incoming};
use kumikit_shell::connections::{Connections, Keepalive, PairRequest, SshRequest};
use kumikit_shell::local::CORE_BIN;
use kumikit_shell::profiles::Profiles;
use kumikit_shell::secrets::TokenStore;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

const DEADLINE: Duration = Duration::from_secs(60);

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

fn core_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .skip(1)
        .map(|d| d.join(CORE_BIN))
        .find(|p| p.is_file())
        .expect("build the core first: cargo build -p kumikit-core")
}

/// A scratch directory: short (socket paths) and private (the core checks).
fn scratch() -> tempfile::TempDir {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let root = tempfile::Builder::new()
        .prefix("kkr")
        .tempdir_in(base)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    root
}

struct Core {
    child: Child,
    socket: String,
    address: String,
    fingerprint: String,
}

impl Core {
    async fn start(root: &Path, name: &str) -> Core {
        #[cfg(unix)]
        let socket = root
            .join(format!("{name}.sock"))
            .to_str()
            .unwrap()
            .to_owned();
        #[cfg(windows)]
        let socket = format!(
            r"\\.\pipe\kumikit-shell-test-{}-{name}",
            root.file_name().unwrap().to_str().unwrap()
        );
        let mut child = Command::new(core_bin())
            .args(["serve", "--socket", &socket, "--listen", "127.0.0.1:0"])
            .arg("--data-dir")
            .arg(root.join(format!("{name}-data")))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let remote = lines.next_line().await.unwrap().unwrap();
        let ready = lines.next_line().await.unwrap().unwrap();
        assert!(ready.starts_with("ready "), "{ready}");
        let (url, fingerprint) = remote
            .strip_prefix("remote ")
            .unwrap()
            .split_once(' ')
            .unwrap();
        let address = url
            .strip_prefix("wss://")
            .and_then(|u| u.strip_suffix("/rpc"))
            .unwrap()
            .to_owned();
        Core {
            child,
            socket,
            address,
            fingerprint: fingerprint.to_owned(),
        }
    }

    async fn code(&self) -> String {
        let out = Command::new(core_bin())
            .args(["pair", "--json", "--socket", &self.socket])
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["code"].as_str().unwrap().to_owned()
    }

    async fn revoke_all(&self) {
        let out = Command::new(core_bin())
            .args(["devices", "list", "--socket", &self.socket])
            .output()
            .await
            .unwrap();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(id) = line.split_whitespace().next().filter(|w| w.len() == 26) {
                let ok = Command::new(core_bin())
                    .args(["devices", "revoke", id, "--socket", &self.socket])
                    .status()
                    .await
                    .unwrap();
                assert!(ok.success());
            }
        }
    }

    async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

fn connections(root: &Path, keepalive: Keepalive) -> Connections {
    let client = root.join("client");
    Connections {
        profiles: Profiles::new(&client),
        secrets: TokenStore::for_platform(&client, true),
        ssh: PathBuf::from(kumikit_test_plugins::FAKE_SSH),
        keepalive,
        device_name: "shell-test".into(),
    }
}

fn sink() -> (
    impl Fn(Incoming) + Send + Sync + 'static,
    mpsc::UnboundedReceiver<Incoming>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    (move |m| drop(tx.send(m)), rx)
}

const HELLO: &str = r#"{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.11","client":{"name":"t","version":"0"}}}"#;

/// Sends `core.hello` over the bridge and returns its result.
async fn hello(bridge: &Bridge, id: &str, rx: &mut mpsc::UnboundedReceiver<Incoming>) -> Value {
    bridge.send(id, HELLO.into()).await.unwrap();
    match rx.recv().await.unwrap() {
        Incoming::Line { line } => serde_json::from_str::<Value>(&line).unwrap()["result"].clone(),
        other => panic!("{other:?}"),
    }
}

fn pair_request(core: &Core, name: &str, fingerprint: &str, code: &str) -> PairRequest {
    PairRequest {
        name: name.into(),
        label: "Dev box".into(),
        address: core.address.clone(),
        fingerprint: fingerprint.into(),
        code: code.into(),
        device_name: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_pair_connect_and_remove() {
    within(async {
        let root = scratch();
        let core = Core::start(root.path(), "a").await;
        let c = connections(root.path(), Keepalive::default());

        assert_eq!(c.probe(&core.address).await.unwrap(), core.fingerprint);
        let info = c
            .pair(pair_request(
                &core,
                "devbox",
                &core.fingerprint,
                &core.code().await,
            ))
            .await
            .unwrap();
        assert_eq!(info.kind, "tls");
        // Nothing the webview receives carries the token.
        let seen = serde_json::to_string(&(c.list().unwrap(), &info)).unwrap();
        assert!(
            !seen.contains("kkd_") && !seen.contains(&core.address),
            "{seen}"
        );
        let names: Vec<_> = c.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["local", "devbox"]);
        // The token is in the secrets file, never in the profiles file.
        let profiles = std::fs::read_to_string(root.path().join("client/profiles.json")).unwrap();
        let secrets = std::fs::read_to_string(root.path().join("client/secrets.json")).unwrap();
        assert!(
            secrets.contains("kkd_") && !profiles.contains("kkd_"),
            "{profiles}"
        );

        let bridge = Bridge::default();
        let (s, mut rx) = sink();
        let id = c.connect(&bridge, "devbox", s).await.unwrap();
        let result = hello(&bridge, &id, &mut rx).await;
        assert!(
            result["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("devices"))
        );

        // Removing revokes this device on the core and closes its connection.
        let removed = c.remove("devbox").await.unwrap();
        assert!(removed.revoked);
        loop {
            if let Incoming::Closed { reason } = rx.recv().await.unwrap() {
                assert_eq!(reason, "this device was revoked");
                break;
            }
        }
        assert!(
            !std::fs::read_to_string(root.path().join("client/secrets.json"))
                .unwrap()
                .contains("kkd_")
        );
        assert_eq!(c.list().unwrap().len(), 1);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_code_goes_only_to_the_pinned_certificate() {
    within(async {
        let root = scratch();
        let core = Core::start(root.path(), "a").await;
        let other = Core::start(root.path(), "b").await;
        let c = connections(root.path(), Keepalive::default());
        let code = core.code().await;

        // Pinned to another core's certificate: refused before anything is sent.
        let err = c
            .pair(pair_request(&core, "devbox", &other.fingerprint, &code))
            .await
            .unwrap_err();
        assert_eq!(err.code, "certificate_changed");
        assert!(err.message.contains(&core.fingerprint), "{}", err.message);
        assert!(c.profiles.list().unwrap().is_empty());
        // So the code is still unused and works with the right pin.
        c.pair(pair_request(&core, "devbox", &core.fingerprint, &code))
            .await
            .unwrap();
        core.stop().await;
        other.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_certificate_is_refused_and_a_revoked_device_is_unauthorized() {
    within(async {
        let root = scratch();
        let core = Core::start(root.path(), "a").await;
        let c = connections(root.path(), Keepalive::default());
        c.pair(pair_request(
            &core,
            "devbox",
            &core.fingerprint,
            &core.code().await,
        ))
        .await
        .unwrap();
        let bridge = Bridge::default();

        // A live connection hears the revocation; the next connect is unauthorized.
        let (s, mut rx) = sink();
        let id = c.connect(&bridge, "devbox", s).await.unwrap();
        hello(&bridge, &id, &mut rx).await;
        core.revoke_all().await;
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Closed {
                reason: "this device was revoked".into()
            }
        );
        let (s, _rx) = sink();
        let err = c.connect(&bridge, "devbox", s).await.unwrap_err();
        assert_eq!(err.code, "unauthorized");

        // Another core now answers at the address the profile names.
        let address = core.address.clone();
        core.stop().await;
        let replaced = Core::start(root.path(), "b").await;
        let mut profiles: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("client/profiles.json")).unwrap(),
        )
        .unwrap();
        profiles[0]["address"] = json!(replaced.address);
        std::fs::write(
            root.path().join("client/profiles.json"),
            profiles.to_string(),
        )
        .unwrap();
        let (s, _rx) = sink();
        let err = c.connect(&bridge, "devbox", s).await.unwrap_err();
        assert_eq!(err.code, "certificate_changed", "{err:?} ({address})");
        replaced.stop().await;
    })
    .await;
}

/// A core that stops answering (frozen with SIGSTOP, as a dead network looks) is noticed
/// by the keepalive.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_silent_core_is_noticed() {
    within(async {
        let root = scratch();
        let core = Core::start(root.path(), "a").await;
        let keepalive = Keepalive {
            ping: Duration::from_millis(100),
            idle: Duration::from_millis(500),
        };
        let c = connections(root.path(), keepalive);
        c.pair(pair_request(
            &core,
            "devbox",
            &core.fingerprint,
            &core.code().await,
        ))
        .await
        .unwrap();
        let bridge = Bridge::default();
        let (s, mut rx) = sink();
        let id = c.connect(&bridge, "devbox", s).await.unwrap();
        hello(&bridge, &id, &mut rx).await;
        let pid = rustix::process::Pid::from_raw(core.child.id().unwrap() as i32).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::STOP).unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Closed {
                reason: "the core stopped answering".into()
            }
        );
        rustix::process::kill_process(pid, rustix::process::Signal::CONT).unwrap();
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_profiles_reach_the_core_through_the_bridge_command() {
    within(async {
        let root = scratch();
        let core = Core::start(root.path(), "a").await;
        let c = connections(root.path(), Keepalive::default());
        let command = format!("{} bridge --socket {}", core_bin().display(), core.socket);

        let err = c
            .add_ssh(SshRequest {
                name: "vm".into(),
                label: "VM".into(),
                destination: "denied".into(),
                command: Some(command.clone()),
                auto_sync: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "unreachable");
        assert!(err.message.contains("Permission denied"), "{}", err.message);

        let err = c
            .add_ssh(SshRequest {
                name: "vm".into(),
                label: "VM".into(),
                destination: "-oProxyCommand=x".into(),
                command: Some(command.clone()),
                auto_sync: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "unreachable");

        let info = c
            .add_ssh(SshRequest {
                name: "vm".into(),
                label: "VM".into(),
                destination: "devbox".into(),
                command: Some(command),
                auto_sync: false,
            })
            .await
            .unwrap();
        assert_eq!(info.kind, "ssh");

        let bridge = Bridge::default();
        let (s, mut rx) = sink();
        let id = c.connect(&bridge, "vm", s).await.unwrap();
        let result = hello(&bridge, &id, &mut rx).await;
        assert_eq!(result["protocol"], "0.11");
        // A local connection on the far side: no device, no auth.
        bridge.close(&id);
        assert!(matches!(rx.recv().await.unwrap(), Incoming::Closed { .. }));
        assert!(c.remove("vm").await.unwrap().revoked);
        core.stop().await;
    })
    .await;
}

/// Live: the real `ssh` to `KK_SSH_DEST` (default `localhost`), where `kumikit` is on the
/// `PATH` and a core runs. Manual: `cargo test -p kumikit-shell --test remote -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs ssh access to a machine running a core"]
async fn live_ssh() {
    within(async {
        let root = scratch();
        let mut c = connections(root.path(), Keepalive::default());
        c.ssh = kumikit_shell::ssh::program(|_| None);
        let destination = std::env::var("KK_SSH_DEST").unwrap_or_else(|_| "localhost".into());
        c.add_ssh(SshRequest {
            name: "live".into(),
            label: "Live".into(),
            destination,
            command: None,
            auto_sync: false,
        })
        .await
        .unwrap();
        let bridge = Bridge::default();
        let (s, mut rx) = sink();
        let id = c.connect(&bridge, "live", s).await.unwrap();
        let result = hello(&bridge, &id, &mut rx).await;
        assert_eq!(result["protocol"], "0.11");
    })
    .await;
}
