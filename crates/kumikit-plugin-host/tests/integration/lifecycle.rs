#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Plugins as kernel providers: real turns through the probe plugin, approvals, cancel,
//! idle unload, failure backoff, disable, and delegated providers. Time is a manual
//! clock, so nothing here sleeps.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kumikit_kernel::grants::Grants;
use kumikit_kernel::process::SupervisorConfig;
use kumikit_kernel::repo::StoredEvent;
use kumikit_kernel::{Kernel, KernelConfig};
use kumikit_plugin_host::engine::EngineConfig;
use kumikit_plugin_host::lifecycle::{HostConfig, PluginHost};
use kumikit_plugin_host::manager::{Dirs, Manager};
use kumikit_plugin_host::package;
use kumikit_plugin_host::runtime::Runtime;
use kumikit_protocol::ThreadEvent;
use kumikit_protocol::model::{Decision, ThreadId, TurnOutcome, UserInput};

const PROBE: &str = r#"
id = "test.probe"
name = "Probe"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:provider/plugin@0.2.0"]
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "probe", name = "Probe" }]
"#;

/// The probe as a provider backend with no permissions of its own.
const BACKEND: &str = r#"
id = "test.backend"
name = "Backend"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:provider/plugin@0.2.0"]
[contributes]
provider-backends = [{ key = "probe-backend", schema = "entry.schema.json" }]
"#;

const ENTRY_SCHEMA: &str = r#"{ "type": "object", "required": ["mode"], "additionalProperties": false,
  "properties": { "mode": { "type": "string" } } }"#;

const DECLARATIVE: &str = r#"
id = "test.declarative"
name = "Declarative"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "delegated", name = "Delegated", probe-backend = { mode = "x" } }]
"#;

#[derive(Clone)]
struct Clock(Arc<Mutex<Instant>>);

impl Clock {
    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct Env {
    dir: tempfile::TempDir,
    kernel: Kernel,
    host: PluginHost,
    clock: Clock,
    workspace: String,
}

fn kkp(dir: &Path, name: &str, manifest: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let src = dir.join(format!("{name}-src"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("plugin.toml"), manifest).unwrap();
    for (path, data) in files {
        std::fs::write(src.join(path), data).unwrap();
    }
    let out = dir.join(format!("{name}.kkp"));
    package::pack(&src, &out).unwrap();
    out
}

async fn env() -> Env {
    env_with_limits_timeout(HostConfig::default().limits_probe_timeout).await
}

async fn env_with_limits_timeout(limits_probe_timeout: Duration) -> Env {
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
    let dirs = Dirs::new(dir.path());
    let runtime = Arc::new(Runtime::new(EngineConfig::default(), dirs.cache.clone()));
    let manager = Manager::open(kernel.clone(), dirs, runtime).await.unwrap();
    let clock = Clock(Arc::new(Mutex::new(Instant::now())));
    let c = clock.clone();
    let config = HostConfig {
        cancel_grace: Duration::from_millis(200),
        limits_probe_timeout,
        now: Arc::new(move || c.now()),
        ..HostConfig::default()
    };
    let host = PluginHost::open(manager, config).await;
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = workspace.to_str().unwrap().to_owned();
    Env {
        dir,
        kernel,
        host,
        clock,
        workspace,
    }
}

fn grant(spawn: &[&str]) -> Grants {
    Grants {
        spawn: spawn.iter().map(|s| (*s).to_owned()).collect(),
        ..Grants::default()
    }
}

impl Env {
    async fn install(&self, name: &str, manifest: &str, files: &[(&str, &[u8])], spawn: &[&str]) {
        let file = kkp(self.dir.path(), name, manifest, files);
        let view = self
            .host
            .install(&file, kumikit_kernel::plugins::PluginSource::File)
            .await
            .unwrap();
        self.host
            .enable(&view.record.id, &grant(spawn))
            .await
            .unwrap();
    }

    async fn install_probe(&self) {
        let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
        self.install(
            "probe",
            PROBE,
            &[("plugin.wasm", &wasm)],
            &["fake-acp-agent"],
        )
        .await;
    }

    async fn thread(&self, provider: &str, model: &str) -> ThreadId {
        let ws = self.kernel.workspace_open(&self.workspace).await.unwrap();
        self.kernel
            .thread_create(
                &ws.id,
                Some(provider.into()),
                Some(model.into()),
                None,
                None,
            )
            .await
            .unwrap()
            .id
    }

    async fn send(&self, thread: &ThreadId, text: &str) {
        self.kernel
            .thread_send(thread, UserInput::text(text))
            .await
            .unwrap();
    }

    /// Events after `after` until one of `kind` (inclusive).
    async fn until(&self, thread: &ThreadId, after: u64, kind: &str) -> Vec<StoredEvent> {
        let mut sub = self.kernel.subscribe(thread, after).await.unwrap();
        let mut out = Vec::new();
        loop {
            for e in sub.next_batch().await.unwrap().unwrap() {
                let done = e.event.kind() == kind;
                out.push(e);
                if done {
                    return out;
                }
            }
        }
    }

    /// Runs one turn and returns its events and the last seq.
    async fn turn(&self, thread: &ThreadId, after: u64, text: &str) -> Vec<StoredEvent> {
        self.send(thread, text).await;
        self.until(thread, after, "turn.ended").await
    }
}

fn kinds(events: &[StoredEvent]) -> Vec<&'static str> {
    events.iter().map(|e| e.event.kind()).collect()
}

fn ended(events: &[StoredEvent]) -> (TurnOutcome, Option<String>) {
    match &events.last().unwrap().event {
        ThreadEvent::TurnEnded(e) => (e.outcome, e.error.clone()),
        other => panic!("last event is {other:?}"),
    }
}

fn last_seq(events: &[StoredEvent]) -> u64 {
    events.last().unwrap().seq
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_provider_runs_a_turn() {
    let e = env().await;
    e.install_probe().await;
    assert_eq!(
        e.host.instances("test.probe"),
        (false, 0),
        "enabled, not instantiated"
    );
    let t = e.thread("test.probe/probe", "echo").await;
    let events = e.turn(&t, 0, "hello there").await;
    assert_eq!(
        kinds(&events),
        [
            "turn.started",
            "text.delta",
            "text.delta",
            "text.done",
            "usage",
            "turn.ended"
        ]
    );
    assert_eq!(ended(&events).0, TurnOutcome::Done);
    assert_eq!(e.host.instances("test.probe"), (true, 1));
    let enabled = e.host.enabled();
    assert_eq!(enabled[0].providers, ["test.probe/probe"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn approvals_reach_the_blocked_guest() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "tool-approval").await;
    e.send(&t, "go").await;
    let events = e.until(&t, 0, "approval.requested").await;
    let ThreadEvent::ApprovalRequested(req) = &events.last().unwrap().event else {
        panic!()
    };
    e.kernel
        .respond_approval(&req.approval.id, Decision::Allow, None)
        .await
        .unwrap();
    let rest = e.until(&t, last_seq(&events), "turn.ended").await;
    assert_eq!(
        kinds(&rest),
        [
            "approval.resolved",
            "tool.output",
            "tool.finished",
            "text.delta",
            "text.done",
            "usage",
            "turn.ended"
        ]
    );
    assert_eq!(ended(&rest).0, TurnOutcome::Done);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_an_approval_and_during_a_blocked_read() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "tool-approval").await;
    e.send(&t, "go").await;
    let events = e.until(&t, 0, "approval.requested").await;
    e.kernel.thread_cancel(&t).await.unwrap();
    let rest = e.until(&t, last_seq(&events), "turn.ended").await;
    assert_eq!(ended(&rest).0, TurnOutcome::Cancelled);

    let t2 = e.thread("test.probe/probe", "probe").await;
    e.send(&t2, "hang").await;
    let started = e.until(&t2, 0, "turn.started").await;
    // The guest is blocked reading the child's stdout once the child exists.
    while e.kernel.supervisor().processes().is_empty() {
        tokio::task::yield_now().await;
    }
    e.kernel.thread_cancel(&t2).await.unwrap();
    let rest = e.until(&t2, last_seq(&started), "turn.ended").await;
    assert_eq!(ended(&rest).0, TurnOutcome::Cancelled);
    // The session's instance survives the cancel and runs the next turn.
    let next = e.turn(&t2, last_seq(&rest), "kv").await;
    assert_eq!(ended(&next).0, TurnOutcome::Done, "{:?}", ended(&next));
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_instances_are_unloaded_and_sessions_resume() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "echo").await;
    let first = e.turn(&t, 0, "one").await;
    assert_eq!(e.host.instances("test.probe"), (true, 1));

    assert_eq!(e.host.unload_idle(e.clock.now()).await, 0, "not idle yet");
    e.clock.advance(Duration::from_secs(601));
    assert_eq!(e.host.unload_idle(e.clock.now()).await, 2);
    assert_eq!(e.host.instances("test.probe"), (false, 0));

    let second = e.turn(&t, last_seq(&first), "two").await;
    assert_eq!(ended(&second).0, TurnOutcome::Done);
    assert_eq!(
        e.host.instances("test.probe"),
        (false, 1),
        "the session started again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trap_fails_the_plugin_with_backoff_then_it_restarts() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "probe").await;
    let events = e.turn(&t, 0, "trap").await;
    let (outcome, error) = ended(&events);
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(error.unwrap().contains("failed"));
    assert_eq!(
        e.host.instances("test.probe"),
        (false, 0),
        "every instance discarded"
    );
    let failure = e.host.failure("test.probe").unwrap();
    assert_eq!(failure.retry_in, Some(Duration::from_secs(1)));

    let during = e.turn(&t, last_seq(&events), "kv").await;
    let (outcome, error) = ended(&during);
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(error.unwrap().contains("retrying in"));

    e.clock.advance(Duration::from_secs(2));
    let after = e.turn(&t, last_seq(&during), "kv").await;
    assert_eq!(ended(&after).0, TurnOutcome::Done, "{:?}", ended(&after));

    // A second failure in a row doubles the backoff.
    let again = e.turn(&t, last_seq(&after), "trap").await;
    assert_eq!(ended(&again).0, TurnOutcome::Failed);
    assert_eq!(
        e.host.failure("test.probe").unwrap().retry_in,
        Some(Duration::from_secs(2))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_cancels_running_turns_and_removes_providers() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "probe").await;
    e.send(&t, "hang").await;
    e.until(&t, 0, "turn.started").await;
    // Wait until the guest runs the turn (its child exists).
    while e.kernel.supervisor().processes().is_empty() {
        tokio::task::yield_now().await;
    }
    e.host.disable("test.probe").await.unwrap();
    let events = e.until(&t, 0, "turn.ended").await;
    assert_eq!(ended(&events).0, TurnOutcome::Cancelled);
    assert_eq!(e.host.instances("test.probe"), (false, 0));
    let ws = e.kernel.workspace_open(&e.workspace).await.unwrap();
    assert!(
        e.kernel
            .thread_create(&ws.id, Some("test.probe/probe".into()), None, None, None)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delegated_providers_appear_with_their_backend_and_use_the_contributors_grants() {
    let e = env().await;
    e.install("declarative", DECLARATIVE, &[], &["fake-acp-agent"])
        .await;
    assert!(e.host.enabled()[0].providers.is_empty(), "no backend yet");

    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &wasm),
            ("entry.schema.json", ENTRY_SCHEMA.as_bytes()),
        ],
        &[],
    )
    .await;
    let declarative = e
        .host
        .enabled()
        .into_iter()
        .find(|p| p.record.id == "test.declarative")
        .unwrap();
    assert_eq!(declarative.providers, ["test.declarative/delegated"]);

    // The backend has no spawn permission; the session runs with the contributor's.
    let t = e.thread("test.declarative/delegated", "probe").await;
    let events = e.turn(&t, 0, "spawn fake-acp-agent").await;
    assert_eq!(ended(&events).0, TurnOutcome::Done, "{:?}", ended(&events));
    assert_eq!(e.host.instances("test.backend").1, 1);

    e.host.disable("test.backend").await.unwrap();
    let declarative = e
        .host
        .enabled()
        .into_iter()
        .find(|p| p.record.id == "test.declarative")
        .unwrap();
    assert!(declarative.providers.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delegated_entry_that_fails_the_backend_schema_is_not_offered() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &wasm),
            ("entry.schema.json", ENTRY_SCHEMA.as_bytes()),
        ],
        &[],
    )
    .await;
    let bad = DECLARATIVE.replace("mode = \"x\"", "mode = 3");
    e.install("declarative", &bad, &[], &["fake-acp-agent"])
        .await;
    let declarative = e
        .host
        .enabled()
        .into_iter()
        .find(|p| p.record.id == "test.declarative")
        .unwrap();
    assert!(declarative.providers.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_that_ignores_cancel_is_discarded_after_the_grace_period() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "probe").await;
    e.send(&t, "ignore-cancel").await;
    let started = e.until(&t, 0, "turn.started").await;
    while e.kernel.supervisor().processes().is_empty() {
        tokio::task::yield_now().await;
    }
    e.kernel.thread_cancel(&t).await.unwrap();
    // The kernel ends the turn at once; it does not wait for the guest.
    let rest = e.until(&t, last_seq(&started), "turn.ended").await;
    assert_eq!(ended(&rest).0, TurnOutcome::Cancelled);
    // After the grace period the session is discarded and its child killed.
    while e.host.instances("test.probe").1 > 0 || !e.kernel.supervisor().processes().is_empty() {
        tokio::task::yield_now().await;
    }
    // The next turn starts a new session.
    let next = e.turn(&t, last_seq(&rest), "kv").await;
    assert_eq!(ended(&next).0, TurnOutcome::Done, "{:?}", ended(&next));
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_a_session_that_is_mid_turn_does_not_wait_for_it() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "tool-approval").await;
    e.send(&t, "go").await;
    e.until(&t, 0, "approval.requested").await;
    // What the core does when it stops: the guest is blocked waiting for the approval.
    e.kernel.close().await;
    assert_eq!(e.host.instances("test.probe").1, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checked_delegated_entry_is_not_checked_again_after_a_restart() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &wasm),
            ("entry.schema.json", ENTRY_SCHEMA.as_bytes()),
        ],
        &[],
    )
    .await;
    e.install("declarative", DECLARATIVE, &[], &["fake-acp-agent"])
        .await;
    let package = e.dir.path().join("plugins/test.declarative/0.1.0");
    let marker = package.join(".checked-test.backend-0.1.0-delegated");
    assert!(marker.is_file(), "the passed check is recorded");

    // Restart on the same data directory: the recorded check is trusted.
    e.kernel.close().await;
    let kernel = Kernel::open(KernelConfig::new(e.dir.path().join("core.sqlite")))
        .await
        .unwrap();
    let dirs = Dirs::new(e.dir.path());
    let runtime = Arc::new(Runtime::new(EngineConfig::default(), dirs.cache.clone()));
    let manager = Manager::open(kernel, dirs, runtime).await.unwrap();
    let reopened = PluginHost::open(manager, HostConfig::default()).await;
    let declarative = reopened
        .enabled()
        .into_iter()
        .find(|p| p.record.id == "test.declarative")
        .unwrap();
    assert_eq!(declarative.providers, ["test.declarative/delegated"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstalling_stops_running_instances_before_deleting_data() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "probe").await;
    let events = e.turn(&t, 0, "kv").await;
    assert_eq!(ended(&events).0, TurnOutcome::Done, "{:?}", ended(&events));
    assert_eq!(e.host.instances("test.probe"), (true, 1));
    let data = e.host.manager().dirs().plugin_data("test.probe");
    assert!(data.is_dir());

    // On Windows an active WASI preopened_dir handle keeps remove_dir_all from
    // removing the plugin's data directory unless instances are stopped first.
    e.host.uninstall("test.probe", true).await.unwrap();
    assert_eq!(e.host.instances("test.probe"), (false, 0));
    assert!(!data.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn delegated_list_models_runs_in_ephemeral_probe_and_caches() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &wasm),
            ("entry.schema.json", ENTRY_SCHEMA.as_bytes()),
        ],
        &[],
    )
    .await;
    e.install("declarative", DECLARATIVE, &[], &["fake-acp-agent"])
        .await;

    let first = e
        .kernel
        .provider_models("test.declarative/delegated")
        .await
        .unwrap();
    assert!(!first.is_empty());
    // The ephemeral probe instance is dropped as soon as list_models finishes.
    assert_eq!(e.host.instances("test.backend"), (false, 0));
    // Subsequent call within TTL is served from cache without leaving instances behind.
    let second = e
        .kernel
        .provider_models("test.declarative/delegated")
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(e.host.instances("test.backend"), (false, 0));
}

fn reply(events: &[StoredEvent]) -> String {
    events
        .iter()
        .rev()
        .find_map(|e| match &e.event {
            ThreadEvent::TextDone(t) => Some(t.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_plugin_still_declares_its_providers() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
    let file = kkp(
        e.dir.path(),
        "probe",
        &PROBE.replace(
            "{ id = \"probe\", name = \"Probe\" }",
            "{ id = \"probe\", name = \"Probe\", icon = \"icon.svg\" }",
        ),
        &[("plugin.wasm", &wasm), ("icon.svg", svg)],
    );
    // Installed but never enabled: nothing is available, but the manifest's providers are.
    let view = e
        .host
        .install(&file, kumikit_kernel::plugins::PluginSource::File)
        .await
        .unwrap();
    assert!(e.kernel.provider_list().is_empty());
    let declared = e.host.declared_providers(&view).await;
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].id, "test.probe/probe");
    assert_eq!(declared[0].name, "Probe");
    let icon = declared[0].icon.as_deref().unwrap();
    assert!(icon.starts_with("data:image/svg+xml;base64,"), "{icon}");

    // A plugin without providers declares none.
    let echo = kkp(
        e.dir.path(),
        "plain",
        &PROBE
            .replace("test.probe", "test.plain")
            .replace("providers = [{ id = \"probe\", name = \"Probe\" }]", ""),
        &[("plugin.wasm", &wasm)],
    );
    let plain = e
        .host
        .install(&echo, kumikit_kernel::plugins::PluginSource::File)
        .await
        .unwrap();
    assert!(e.host.declared_providers(&plain).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn icons_and_model_settings_reach_clients_and_sessions() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
    e.install(
        "probe",
        &PROBE.replace(
            "{ id = \"probe\", name = \"Probe\" }",
            "{ id = \"probe\", name = \"Probe\", icon = \"icon.svg\" }",
        ),
        &[("plugin.wasm", &wasm), ("icon.svg", svg)],
        &["fake-acp-agent"],
    )
    .await;

    // provider.list carries the icon as a data URL.
    let list = e.kernel.provider_list();
    let icon = list[0].icon.as_deref().unwrap();
    assert!(icon.starts_with("data:image/svg+xml;base64,"), "{icon}");

    // The probe model offers an effort setting.
    let models = e.kernel.provider_models("test.probe/probe").await.unwrap();
    assert_eq!(models[0].options[0].id, "effort");

    // A thread created with a setting passes it to the session as `option.<id>`.
    let ws = e.kernel.workspace_open(&e.workspace).await.unwrap();
    let options = std::collections::BTreeMap::from([("effort".to_owned(), "high".to_owned())]);
    let t = e
        .kernel
        .thread_create_opts(
            &ws.id,
            Some("test.probe/probe".into()),
            Some("probe".into()),
            None,
            None,
            Some(options),
            None,
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(t.options["effort"], "high");
    let events = e.turn(&t.id, 0, &format!("session-options {}", t.id)).await;
    assert!(
        reply(&events).contains("option.effort=high"),
        "{}",
        reply(&events)
    );

    // An unknown value is refused; a valid change restarts the session with it.
    let bad = std::collections::BTreeMap::from([("effort".to_owned(), "max".to_owned())]);
    let err = e
        .kernel
        .thread_update(&t.id, None, None, None, None, None, Some(bad))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no value max"), "{err}");
    let low = std::collections::BTreeMap::from([("effort".to_owned(), "low".to_owned())]);
    e.kernel
        .thread_update(&t.id, None, None, None, None, None, Some(low))
        .await
        .unwrap();
    let after = last_seq(&events);
    let events = e
        .turn(&t.id, after, &format!("session-options {}", t.id))
        .await;
    assert!(
        reply(&events).contains("option.effort=low"),
        "{}",
        reply(&events)
    );

    // Switching to a model without the setting drops it.
    let t2 = e
        .kernel
        .thread_update(&t.id, None, Some("echo".into()), None, None, None, None)
        .await
        .unwrap();
    assert!(t2.options.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_limits_snapshots_are_refused_and_keep_the_stored_snapshot() {
    let e = env().await;
    e.install_probe().await;
    let t = e.thread("test.probe/probe", "probe").await;

    let ok = e.turn(&t, 0, "limits 26").await;
    assert_eq!(ended(&ok).0, TurnOutcome::Done);
    let mut after = last_seq(&ok);

    for bad in [
        "limits-invalid nan",
        "limits-invalid reset",
        "limits-invalid too-many",
    ] {
        let evs = e.turn(&t, after, bad).await;
        after = last_seq(&evs);
        let (outcome, err) = ended(&evs);
        assert_eq!(outcome, TurnOutcome::Failed, "{bad}: {err:?}");
    }

    let stored = e
        .kernel
        .usage_limits(Some("test.probe/probe"))
        .await
        .unwrap();
    assert_eq!(
        stored[0].limits.as_ref().unwrap().windows[0].used_percent,
        26.0,
        "invalid snapshots must not overwrite valid stored limits"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_limits_timeout_cancels_a_hanging_reader() {
    let e = env_with_limits_timeout(Duration::from_millis(150)).await;
    let backend_wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    let schema = r#"{ "type": "object", "properties": { "env": { "type": "object" } } }"#;
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &backend_wasm),
            ("entry.schema.json", schema.as_bytes()),
        ],
        &[],
    )
    .await;
    let limits_wasm = std::fs::read(kumikit_test_plugins::LIMITS_WASM).unwrap();
    let manifest = r#"
id = "test.limits-hang"
name = "Hanging limits"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:plugin/plugin@0.2.0", "kumikit:usage/reader@0.1.0"]
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "hang", name = "Hang", probe-backend = { env = { LIMITS = "hang" } } }]
"#;
    e.install(
        "limits-hang",
        manifest,
        &[("plugin.wasm", &limits_wasm)],
        &["fake-acp-agent"],
    )
    .await;

    let err = e
        .kernel
        .usage_refresh("test.limits-hang/hang")
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::PluginFailed);
    assert_eq!(e.host.instances("test.limits-hang"), (false, 0));
    assert!(
        e.kernel.supervisor().processes().is_empty(),
        "timed-out reader child must be cleaned up"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_limits_trap_and_invalid_snapshot_fail_cleanly() {
    let e = env().await;
    let backend_wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    let schema = r#"{ "type": "object", "properties": { "env": { "type": "object" } } }"#;
    e.install(
        "backend",
        BACKEND,
        &[
            ("plugin.wasm", &backend_wasm),
            ("entry.schema.json", schema.as_bytes()),
        ],
        &[],
    )
    .await;
    let limits_wasm = std::fs::read(kumikit_test_plugins::LIMITS_WASM).unwrap();
    let manifest = r#"
id = "test.limits-fault"
name = "Faulting limits"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:plugin/plugin@0.2.0", "kumikit:usage/reader@0.1.0"]
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [
  { id = "invalid", name = "Invalid", probe-backend = { env = { LIMITS = "invalid" } } },
  { id = "trap", name = "Trap", probe-backend = { env = { LIMITS = "trap" } } },
]
"#;
    e.install(
        "limits-fault",
        manifest,
        &[("plugin.wasm", &limits_wasm)],
        &["fake-acp-agent"],
    )
    .await;

    let invalid = e
        .kernel
        .usage_refresh("test.limits-fault/invalid")
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), kumikit_protocol::ErrorCode::InvalidParams);
    assert!(e.host.failure("test.limits-fault").is_none());

    let trapped = e
        .kernel
        .usage_refresh("test.limits-fault/trap")
        .await
        .unwrap_err();
    assert_eq!(trapped.code(), kumikit_protocol::ErrorCode::PluginFailed);
    let failure = e
        .host
        .failure("test.limits-fault")
        .expect("a trapping reader records a plugin failure");
    assert!(failure.retry_in.is_some());
    assert_eq!(e.host.instances("test.limits-fault"), (false, 0));
}
