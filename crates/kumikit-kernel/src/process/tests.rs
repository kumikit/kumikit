//! Supervisor tests. They start `fake-acp-agent` (built by `kumikit-test-plugins`), found
//! through a `PATH` that contains only its directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;

const AGENT: &str = "fake-acp-agent";

struct Fixture {
    supervisor: Supervisor,
    workspace: tempfile::TempDir,
    grants: Grants,
    owner: OwnerId,
}

fn agent_dir() -> PathBuf {
    Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture(extra_env: &[(&str, &str)]) -> Fixture {
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), agent_dir().into_os_string()),
        ("HOME".into(), "/home/test".into()),
        ("LC_ALL".into(), "C".into()),
        ("SECRET_FROM_CORE".into(), "do-not-pass".into()),
    ];
    for (k, v) in extra_env {
        env.push(((*k).into(), (*v).into()));
    }
    if cfg!(windows) {
        // Programs on Windows need these to start at all.
        for k in ["SystemRoot", "windir", "PATHEXT"] {
            if let Some(v) = std::env::var_os(k) {
                env.push((k.into(), v));
            }
        }
    }
    let mut grants = Grants::default();
    grants.spawn.insert(AGENT.into());
    grants.secrets.insert("token".into());
    Fixture {
        supervisor: Supervisor::new(SupervisorConfig {
            env,
            kill_grace: Duration::from_secs(2),
        }),
        workspace: tempfile::tempdir().unwrap(),
        grants,
        owner: OwnerId("test-owner".into()),
    }
}

fn secret(name: &str) -> Option<String> {
    (name == "token").then(|| "s3cret".to_owned())
}

impl Fixture {
    async fn spawn(&self, req: SpawnRequest) -> Result<Child, ProcessError> {
        let roots = [self.workspace.path().to_path_buf()];
        let ctx = SpawnContext {
            plugin: "test.plugin",
            owner: &self.owner,
            grants: &self.grants,
            workspaces: &roots,
            secret: &secret,
        };
        self.supervisor.spawn(&ctx, req).await
    }
}

fn req(args: &[&str]) -> SpawnRequest {
    SpawnRequest {
        program: AGENT.into(),
        args: args.iter().map(|s| (*s).to_owned()).collect(),
        ..SpawnRequest::default()
    }
}

async fn read_all(child: &Child) -> String {
    let mut out = Vec::new();
    while let Some(chunk) = child.read_stdout(4096).await {
        out.extend(chunk);
    }
    String::from_utf8(out).unwrap()
}

fn denied(r: Result<Child, ProcessError>) -> String {
    match r {
        Err(ProcessError::PermissionDenied(m)) => m,
        other => panic!("expected permission_denied, got {other:?}"),
    }
}

#[tokio::test]
async fn unlisted_program_is_denied() {
    let f = fixture(&[]);
    let mut r = req(&["env"]);
    r.program = "sh".into();
    assert!(denied(f.spawn(r).await).contains("not in permissions.spawn"));
}

#[tokio::test]
async fn path_qualified_program_is_denied_even_if_its_name_is_listed() {
    let f = fixture(&[]);
    let mut r = req(&["env"]);
    r.program = kumikit_test_plugins::FAKE_ACP_AGENT.into();
    assert!(denied(f.spawn(r).await).contains("bare program name"));
    for name in ["./fake-acp-agent", "..", "", "a\\b", "c:x"] {
        let mut r = req(&["env"]);
        r.program = name.into();
        denied(f.spawn(r).await);
    }
}

#[tokio::test]
async fn cwd_outside_the_workspace_is_denied() {
    let f = fixture(&[]);
    let outside = tempfile::tempdir().unwrap();
    let mut r = req(&["cwd"]);
    r.cwd = Some(outside.path().to_string_lossy().into_owned());
    assert!(denied(f.spawn(r).await).contains("outside the workspace"));
    let mut r = req(&["cwd"]);
    r.cwd = Some("..".into());
    denied(f.spawn(r).await);
}

#[cfg(unix)]
#[tokio::test]
async fn cwd_through_a_symlink_out_of_the_workspace_is_denied() {
    let f = fixture(&[]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), f.workspace.path().join("link")).unwrap();
    let mut r = req(&["cwd"]);
    r.cwd = Some("link".into());
    denied(f.spawn(r).await);
}

#[tokio::test]
async fn no_workspace_means_no_spawn() {
    let f = fixture(&[]);
    let ctx = SpawnContext {
        plugin: "test.plugin",
        owner: &f.owner,
        grants: &f.grants,
        workspaces: &[],
        secret: &secret,
    };
    denied(f.supervisor.spawn(&ctx, req(&["cwd"])).await);
}

#[tokio::test]
async fn cwd_defaults_to_the_workspace_root_and_may_be_a_subdirectory() {
    let f = fixture(&[]);
    let root = std::fs::canonicalize(f.workspace.path()).unwrap();
    let child = f.spawn(req(&["cwd"])).await.unwrap();
    assert_eq!(PathBuf::from(read_all(&child).await.trim()), root);

    std::fs::create_dir(root.join("sub")).unwrap();
    let mut r = req(&["cwd"]);
    r.cwd = Some("sub".into());
    let child = f.spawn(r).await.unwrap();
    assert_eq!(
        PathBuf::from(read_all(&child).await.trim()),
        root.join("sub")
    );
}

#[tokio::test]
async fn environment_is_the_allowlist_plus_what_the_plugin_sets() {
    let f = fixture(&[]);
    let mut r = req(&["env"]);
    r.env = vec![("FOO".into(), "bar".into())];
    r.secret_env = vec![("API_TOKEN".into(), "token".into())];
    let out = read_all(&f.spawn(r).await.unwrap()).await;
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"HOME=/home/test"), "{out}");
    assert!(lines.contains(&"LC_ALL=C"), "{out}");
    assert!(lines.contains(&"FOO=bar"), "{out}");
    assert!(lines.contains(&"API_TOKEN=s3cret"), "{out}");
    assert!(!out.contains("SECRET_FROM_CORE"), "{out}");
}

#[tokio::test]
async fn loader_variables_are_refused() {
    let f = fixture(&[]);
    for name in ["LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "ld_library_path"] {
        let mut r = req(&["env"]);
        r.env = vec![(name.into(), "/tmp/x.so".into())];
        denied(f.spawn(r).await);
    }
}

#[tokio::test]
async fn undeclared_secret_env_is_denied_and_unset_one_is_not_found() {
    let mut f = fixture(&[]);
    let mut r = req(&["env"]);
    r.secret_env = vec![("X".into(), "other".into())];
    assert!(denied(f.spawn(r).await).contains("permissions.secrets"));

    f.grants.secrets.insert("unset".into());
    let mut r = req(&["env"]);
    r.secret_env = vec![("X".into(), "unset".into())];
    assert!(matches!(f.spawn(r).await, Err(ProcessError::NotFound(_))));
}

#[tokio::test]
async fn missing_program_is_not_found() {
    let mut f = fixture(&[]);
    f.grants.spawn.insert("no-such-program".into());
    let mut r = req(&[]);
    r.program = "no-such-program".into();
    assert!(matches!(f.spawn(r).await, Err(ProcessError::NotFound(_))));
}

#[tokio::test]
async fn resolved_path_is_recorded() {
    let f = fixture(&[]);
    let child = f.spawn(req(&["hang"])).await.unwrap();
    let procs = f.supervisor.processes();
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].program, AGENT);
    assert_eq!(
        std::fs::canonicalize(&procs[0].resolved).unwrap(),
        std::fs::canonicalize(kumikit_test_plugins::FAKE_ACP_AGENT).unwrap()
    );
    assert_eq!(procs[0].plugin, "test.plugin");
    child.kill().await;
}

#[tokio::test]
async fn stdin_and_stdout_round_trip() {
    let f = fixture(&[]);
    let child = f.spawn(req(&["cat"])).await.unwrap();
    child.write_stdin(b"hello\n").await.unwrap();
    child.close_stdin().await;
    assert_eq!(read_all(&child).await, "hello\n");
    assert_eq!(child.wait().await.code, Some(0));
}

#[tokio::test]
async fn unread_stderr_does_not_stall_the_child() {
    let f = fixture(&[]);
    // Far more than a pipe buffer; the child finishes only if stderr keeps draining.
    let child = f.spawn(req(&["stderr-flood", "4000000"])).await.unwrap();
    assert_eq!(read_all(&child).await, "done\n");
    let mut kept = 0;
    while let Some(chunk) = child.read_stderr(1 << 20).await {
        kept += chunk.len();
    }
    assert_eq!(kept, STDERR_KEEP);
}

/// Stdout reaches EOF only when every process holding it is gone, so this proves the
/// whole tree died without timing.
#[tokio::test]
async fn killing_an_owner_kills_the_whole_tree() {
    let f = fixture(&[]);
    let child = f.spawn(req(&["tree"])).await.unwrap();
    assert_eq!(child.read_stdout(6).await.unwrap(), b"ready\n");
    f.supervisor.kill_owner(&f.owner).await;
    assert_eq!(read_all(&child).await, "");
    assert!(f.supervisor.processes().is_empty());
}

#[tokio::test]
async fn shutdown_kills_every_tree() {
    let f = fixture(&[]);
    let a = f.spawn(req(&["tree"])).await.unwrap();
    let b = f.spawn(req(&["tree"])).await.unwrap();
    assert_eq!(a.read_stdout(6).await.unwrap(), b"ready\n");
    assert_eq!(b.read_stdout(6).await.unwrap(), b"ready\n");
    f.supervisor.shutdown().await;
    assert_eq!(read_all(&a).await, "");
    assert_eq!(read_all(&b).await, "");
}

#[tokio::test]
async fn dropping_the_last_handle_kills_the_tree() {
    let f = fixture(&[]);
    let child = f.spawn(req(&["tree"])).await.unwrap();
    assert_eq!(child.read_stdout(6).await.unwrap(), b"ready\n");
    // Keep the stdout channel to watch for EOF after every handle is gone.
    let mut rx = {
        let mut out = child.inner.stdout.lock().await;
        std::mem::replace(&mut out.rx, mpsc::channel(1).1)
    };
    drop(child);
    while rx.recv().await.is_some() {}
}

#[test]
fn env_patterns() {
    assert!(env_allowed("PATH"));
    assert!(env_allowed("LC_ALL"));
    assert!(env_allowed("XDG_CONFIG_HOME"));
    assert!(!env_allowed("LD_PRELOAD"));
    assert!(!env_allowed("AWS_SECRET_ACCESS_KEY"));
    assert!(!env_allowed("LC"));
    assert!(is_bare_name("claude"));
    assert!(!is_bare_name("bin/claude"));
}
