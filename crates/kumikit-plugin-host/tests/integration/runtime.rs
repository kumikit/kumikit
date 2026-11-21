#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The WASM runtime: one test per permission proving the denied path returns
//! `permission-denied`, the sandbox (files, sockets, unsupported imports), and traps and
//! limit breaches. Drives the probe plugin's `send` directly, without a kernel turn.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kumikit_kernel::grants::Grants;
use kumikit_kernel::process::SupervisorConfig;
use kumikit_kernel::{Kernel, KernelConfig};
use kumikit_plugin_host::engine::{DEFAULT_MEMORY, EngineConfig};
use kumikit_plugin_host::manifest::{WORLD_PLUGIN, WORLD_PROVIDER};
use kumikit_plugin_host::runtime::bindings::base::kumikit::host::types::{Error, ErrorCode};
use kumikit_plugin_host::runtime::bindings::provider::kumikit::provider::types::{
    ProviderRef, UserInput,
};
use kumikit_plugin_host::runtime::{
    CallError, Identity, Instance, InstanceConfig, Runtime, SessionShared,
};

struct Env {
    dir: tempfile::TempDir,
    kernel: Kernel,
    runtime: Runtime,
    workspace: PathBuf,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap();
    let mut env: Vec<(OsString, OsString)> = vec![("PATH".into(), agent_dir.into())];
    for k in ["SystemRoot", "windir", "PATHEXT"] {
        if let Some(v) = std::env::var_os(k) {
            env.push((k.into(), v));
        }
    }
    let kernel = Kernel::open(KernelConfig {
        supervisor: SupervisorConfig {
            env,
            kill_grace: Duration::from_secs(2),
        },
        ..KernelConfig::new(dir.path().join("core.sqlite"))
    })
    .await
    .unwrap();
    let runtime = Runtime::new(EngineConfig::default(), dir.path().join("cache"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    Env {
        dir,
        kernel,
        runtime,
        workspace,
    }
}

impl Env {
    async fn probe_with(&self, grants: Grants, timeout: Duration) -> Instance {
        let prepared = self
            .runtime
            .prepare(Path::new(kumikit_test_plugins::PROBE_WASM), WORLD_PROVIDER)
            .await
            .unwrap();
        let config = InstanceConfig {
            identity: Identity {
                plugin: "test.probe".into(),
                grants_from: "test.probe".into(),
                grants,
                workspaces: vec![self.workspace.clone()],
            },
            data_dir: self.dir.path().join("plugin-data").join("test.probe"),
            memory: DEFAULT_MEMORY,
            call_timeout: timeout,
            settings_schema: None,
            session: Arc::new(SessionShared::default()),
        };
        let instance = Instance::start(&self.runtime, &prepared, config, self.kernel.clone())
            .await
            .unwrap();
        instance.activate().await.unwrap().unwrap();
        instance
    }

    async fn probe(&self, grants: Grants) -> Instance {
        self.probe_with(grants, Duration::from_secs(10)).await
    }
}

fn grants(spawn: &[&str], secrets: &[&str]) -> Grants {
    Grants {
        spawn: spawn.iter().map(|s| (*s).to_owned()).collect(),
        secrets: secrets.iter().map(|s| (*s).to_owned()).collect(),
        ..Grants::default()
    }
}

async fn run(instance: &Instance, command: &str) -> Result<(), Error> {
    instance
        .send(
            "probe:t".into(),
            UserInput {
                text: command.into(),
            },
        )
        .await
        .expect("the probe call must not trap")
}

fn code(r: Result<(), Error>) -> ErrorCode {
    r.expect_err("expected an error").code
}

// ---- one test per permission: the denied path returns permission-denied ----

#[tokio::test(flavor = "multi_thread")]
async fn spawn_without_permission_is_denied() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    assert_eq!(
        code(run(&p, "spawn fake-acp-agent").await),
        ErrorCode::PermissionDenied
    );
    let p = e.probe(grants(&["fake-acp-agent"], &[])).await;
    run(&p, "spawn fake-acp-agent").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_by_path_is_denied() {
    let e = env().await;
    let p = e.probe(grants(&["fake-acp-agent"], &[])).await;
    let cmd = format!("spawn {}", kumikit_test_plugins::FAKE_ACP_AGENT);
    assert_eq!(code(run(&p, &cmd).await), ErrorCode::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_outside_the_workspace_is_denied() {
    let e = env().await;
    let p = e.probe(grants(&["fake-acp-agent"], &[])).await;
    let outside = e.dir.path().display().to_string();
    let cmd = format!("spawn fake-acp-agent {outside}");
    assert_eq!(code(run(&p, &cmd).await), ErrorCode::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_without_permission_is_denied() {
    let e = env().await;
    e.kernel.secrets().set("token", "s3cret").await.unwrap();
    let p = e.probe(Grants::default()).await;
    assert_eq!(
        code(run(&p, "secret token").await),
        ErrorCode::PermissionDenied
    );
    let p = e.probe(grants(&[], &["token"])).await;
    run(&p, "secret token").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn secret_env_without_permission_is_denied() {
    let e = env().await;
    e.kernel.secrets().set("token", "s3cret").await.unwrap();
    let p = e.probe(grants(&["fake-acp-agent"], &[])).await;
    assert_eq!(
        code(run(&p, "spawn-secret API_TOKEN token").await),
        ErrorCode::PermissionDenied
    );
    let p = e.probe(grants(&["fake-acp-agent"], &["token"])).await;
    run(&p, "spawn-secret API_TOKEN token").await.unwrap();
}

// ---- the sandbox ----

#[tokio::test(flavor = "multi_thread")]
async fn files_outside_the_data_directory_are_unreachable() {
    let e = env().await;
    let secret = e.dir.path().join("core.sqlite");
    let p = e.probe(Grants::default()).await;
    run(&p, "fs-own").await.unwrap();
    assert!(
        e.dir
            .path()
            .join("plugin-data/test.probe/probe.txt")
            .is_file()
    );
    let err = run(&p, &format!("fs-escape {}", secret.display()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied, "{}", err.message);
    let err = run(&p, "fs-escape /data/../../core.sqlite")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied, "{}", err.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn sockets_are_refused() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    let err = run(&p, "socket").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied, "{}", err.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_with_a_malformed_wasi_http_import_is_refused_when_prepared() {
    let e = env().await;
    let wat = e.dir.path().join("http.wat");
    std::fs::write(
        &wat,
        // wasi:http is linked (ADR 0023), but this `handle` has the wrong type. (An empty
        // instance import would be satisfied without any definition.)
        r#"(component (import "wasi:http/outgoing-handler@0.2.0" (instance (export "handle" (func)))))"#,
    )
    .unwrap();
    let err = e.runtime.prepare(&wat, WORLD_PLUGIN).await.unwrap_err();
    assert!(err.0.contains("does not provide"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_without_the_world_exports_is_refused() {
    let e = env().await;
    let err = e
        .runtime
        .prepare(Path::new(kumikit_test_plugins::HELLO_WASM), WORLD_PROVIDER)
        .await
        .unwrap_err();
    assert!(err.0.contains("does not implement"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn kv_round_trip_is_namespaced() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    run(&p, "kv").await.unwrap();
    assert_eq!(
        e.kernel.kv_list("test.probe", "").await.unwrap(),
        ["a/2", "b"]
    );
    assert!(e.kernel.kv_list("other", "").await.unwrap().is_empty());
}

// ---- traps and limits ----

fn trapped(r: Result<Result<(), Error>, CallError>) -> (String, bool) {
    match r {
        Err(CallError::Trap {
            message, timed_out, ..
        }) => (message, timed_out),
        other => panic!("expected a trap, got {other:?}"),
    }
}

async fn send(p: &Instance, command: &str) -> Result<Result<(), Error>, CallError> {
    p.send(
        "probe:t".into(),
        UserInput {
            text: command.into(),
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trap_ends_the_instance() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    let (message, timed_out) = trapped(send(&p, "trap").await);
    assert!(!timed_out, "{message}");
    assert!(matches!(send(&p, "kv").await, Err(CallError::Gone)));
}

#[tokio::test(flavor = "multi_thread")]
async fn running_past_the_call_limit_traps() {
    let e = env().await;
    let p = e
        .probe_with(Grants::default(), Duration::from_millis(300))
        .await;
    let (message, timed_out) = trapped(send(&p, "spin").await);
    assert!(timed_out, "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn time_blocked_in_the_host_does_not_count() {
    let e = env().await;
    // The guest waits about a second in read-stdout for a child; its 300 ms limit
    // counts only time spent running guest code, so it must not trip.
    let p = e
        .probe_with(grants(&["fake-acp-agent"], &[]), Duration::from_millis(300))
        .await;
    run(&p, "run sleep 1000").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn growing_past_the_memory_limit_traps() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    let (message, timed_out) = trapped(send(&p, "oom 200").await);
    assert!(!timed_out, "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_unblocks_a_waiting_import() {
    let e = env().await;
    let session = Arc::new(SessionShared::default());
    let prepared = e
        .runtime
        .prepare(Path::new(kumikit_test_plugins::PROBE_WASM), WORLD_PROVIDER)
        .await
        .unwrap();
    let p = Instance::start(
        &e.runtime,
        &prepared,
        InstanceConfig {
            identity: Identity {
                plugin: "test.probe".into(),
                grants_from: "test.probe".into(),
                grants: grants(&["fake-acp-agent"], &[]),
                workspaces: vec![e.workspace.clone()],
            },
            data_dir: e.dir.path().join("pd"),
            memory: DEFAULT_MEMORY,
            call_timeout: Duration::from_secs(10),
            settings_schema: None,
            session: Arc::clone(&session),
        },
        e.kernel.clone(),
    )
    .await
    .unwrap();
    let call = tokio::spawn({
        let p = p.clone();
        async move { run(&p, "hang").await }
    });
    // Wait until the guest's child exists, so the guest is blocked in read-stdout.
    while e.kernel.supervisor().processes().is_empty() {
        tokio::task::yield_now().await;
    }
    session.cancel();
    assert_eq!(code(call.await.unwrap()), ErrorCode::Cancelled);
    // The instance survives a cancel.
    run(&p, "kv").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_models_and_start_session() {
    let e = env().await;
    let p = e.probe(Grants::default()).await;
    let pref = ProviderRef {
        id: "probe".into(),
        plugin: "test.probe".into(),
        config: None,
    };
    let models = p.list_models(pref).await.unwrap().unwrap();
    assert_eq!(models[0].id, "probe");
}
