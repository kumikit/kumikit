use super::*;
use kumikit_kernel::KernelConfig;

const DECLARATIVE: &str = r#"
id = "test.declarative"
name = "Declarative"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "fake", name = "Fake", acp = { command = "fake-acp-agent" } }]
"#;

struct Env {
    dir: tempfile::TempDir,
    manager: Manager,
}

async fn open(dir: &tempfile::TempDir) -> Manager {
    let kernel = Kernel::open(KernelConfig::new(dir.path().join("core.sqlite")))
        .await
        .unwrap();
    let dirs = Dirs::new(dir.path());
    let runtime = Arc::new(Runtime::new(
        crate::engine::EngineConfig::default(),
        dirs.cache.clone(),
    ));
    Manager::open(kernel, dirs, runtime).await.unwrap()
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let manager = open(&dir).await;
    Env { dir, manager }
}

/// Packs a manifest (and extra files) into a `.kkp` in `dir`.
fn kkp(dir: &Path, name: &str, manifest: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let src = dir.join(format!("{name}-src"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join(MANIFEST_FILE), manifest).unwrap();
    for (path, data) in files {
        std::fs::write(src.join(path), data).unwrap();
    }
    let out = dir.join(format!("{name}.kkp"));
    package::pack(&src, &out).unwrap();
    out
}

fn grant(spawn: &[&str]) -> Grants {
    Grants {
        spawn: spawn.iter().map(|s| (*s).to_owned()).collect(),
        ..Grants::default()
    }
}

#[tokio::test]
async fn install_enable_disable_and_reopen() {
    let e = env().await;
    let file = kkp(e.dir.path(), "p", DECLARATIVE, &[]);
    let mut changes = e.manager.changes();

    let view = e.manager.install(&file).await.unwrap();
    assert_eq!(view.record.state, PluginState::Installed);
    assert_eq!(view.record.granted, None);
    assert_eq!(view.record.sha256, package::sha256_file(&file).unwrap());
    assert_eq!(view.requested(), grant(&["fake-acp-agent"]));
    assert!(
        e.manager
            .dirs()
            .package("test.declarative", "0.1.0")
            .join(MANIFEST_FILE)
            .is_file()
    );
    assert!(e.manager.enabled().is_empty(), "installed is not enabled");
    assert!(!changes.has_changed().unwrap());

    let err = e
        .manager
        .enable("test.declarative", &grant(&[]))
        .await
        .unwrap_err();
    assert!(matches!(err, HostError::Conflict(_)), "{err}");
    let view = e
        .manager
        .enable("test.declarative", &grant(&["fake-acp-agent"]))
        .await
        .unwrap();
    assert_eq!(view.record.state, PluginState::Enabled);
    assert_eq!(e.manager.enabled().len(), 1);
    assert!(changes.has_changed().unwrap());
    changes.mark_unchanged();

    let reopened = open(&e.dir).await;
    let view = reopened.get("test.declarative").unwrap();
    assert_eq!(view.record.state, PluginState::Enabled);
    assert_eq!(view.record.granted, Some(grant(&["fake-acp-agent"])));
    assert!(view.manifest.is_some());

    let view = e.manager.disable("test.declarative").await.unwrap();
    assert_eq!(view.record.state, PluginState::Disabled);
    assert!(e.manager.enabled().is_empty());
    assert!(changes.has_changed().unwrap());
}

#[tokio::test]
async fn installing_twice_is_a_conflict() {
    let e = env().await;
    let file = kkp(e.dir.path(), "p", DECLARATIVE, &[]);
    e.manager.install(&file).await.unwrap();
    let err = e.manager.install(&file).await.unwrap_err();
    assert!(matches!(err, HostError::Conflict(_)), "{err}");
}

#[tokio::test]
async fn invalid_manifests_list_every_problem() {
    let e = env().await;
    let bad = DECLARATIVE
        .replace("version = \"0.1.0\"", "version = \"one\"")
        .replace("spawn = [\"fake-acp-agent\"]", "spawn = [\"/bin/sh\"]");
    let file = kkp(e.dir.path(), "bad", &bad, &[]);
    let err = e.manager.install(&file).await.unwrap_err();
    let HostError::Invalid(message) = &err else {
        panic!("{err:?}")
    };
    assert!(
        message.contains("semver") && message.contains("bare program name"),
        "{message}"
    );
    assert_eq!(err.code(), ErrorCode::InvalidParams);
    assert!(e.manager.list().is_empty());
    // Nothing is left behind in the plugins directory.
    let left: Vec<_> = std::fs::read_dir(&e.manager.dirs().plugins)
        .unwrap()
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn a_package_without_a_manifest_is_invalid() {
    let e = env().await;
    let src = e.dir.path().join("empty");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("README"), "hi").unwrap();
    let out = e.dir.path().join("empty.kkp");
    package::pack(&src, &out).unwrap();
    let err = e.manager.install(&out).await.unwrap_err();
    assert!(err.to_string().contains("plugin.toml is missing"), "{err}");
}

#[tokio::test]
async fn uninstall_keeps_or_deletes_data() {
    let e = env().await;
    let file = kkp(e.dir.path(), "p", DECLARATIVE, &[]);
    let kernel = e.manager.kernel().clone();

    e.manager.install(&file).await.unwrap();
    let data = e.manager.dirs().plugin_data("test.declarative");
    std::fs::create_dir_all(&data).unwrap();
    kernel
        .kv_set("test.declarative", "k", b"v".to_vec())
        .await
        .unwrap();
    e.manager
        .uninstall("test.declarative", false)
        .await
        .unwrap();
    assert!(data.exists());
    assert!(
        kernel
            .kv_get("test.declarative", "k")
            .await
            .unwrap()
            .is_some()
    );
    assert!(!e.manager.dirs().plugins.join("test.declarative").exists());

    e.manager.install(&file).await.unwrap();
    e.manager.uninstall("test.declarative", true).await.unwrap();
    assert!(!data.exists());
    assert!(
        kernel
            .kv_get("test.declarative", "k")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        e.manager.uninstall("test.declarative", true).await,
        Err(HostError::NotFound(_))
    ));
}

#[tokio::test]
async fn a_damaged_install_is_listed_with_its_error() {
    let e = env().await;
    let file = kkp(e.dir.path(), "p", DECLARATIVE, &[]);
    e.manager.install(&file).await.unwrap();
    std::fs::remove_file(
        e.manager
            .dirs()
            .package("test.declarative", "0.1.0")
            .join(MANIFEST_FILE),
    )
    .unwrap();
    let reopened = open(&e.dir).await;
    let view = reopened.get("test.declarative").unwrap();
    assert!(view.manifest.is_none());
    assert!(view.error.unwrap().contains("cannot read plugin.toml"));
    assert!(matches!(
        reopened
            .enable("test.declarative", &grant(&["fake-acp-agent"]))
            .await,
        Err(HostError::Conflict(_))
    ));
}

fn component_manifest(world: &str) -> String {
    format!(
        r#"
id = "test.probe"
name = "Probe"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[runtime]
wasm = "plugin.wasm"
worlds = ["{world}"]
[contributes]
providers = [{{ id = "probe", name = "Probe" }}]
"#
    )
}

#[tokio::test]
async fn enabling_compiles_the_component() {
    let e = env().await;
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    let file = kkp(
        e.dir.path(),
        "probe",
        &component_manifest(crate::manifest::WORLD_PROVIDER),
        &[("plugin.wasm", &wasm)],
    );
    e.manager.install(&file).await.unwrap();
    assert!(
        !e.manager.runtime().is_started(),
        "installing starts no WASM"
    );
    e.manager
        .enable("test.probe", &Grants::default())
        .await
        .unwrap();
    assert!(e.manager.runtime().is_started());
    e.manager.prepared("test.probe").await.unwrap();
}

#[tokio::test]
async fn a_component_that_does_not_match_its_world_cannot_be_enabled() {
    let e = env().await;
    // The hello plugin implements only the base world but claims the provider world.
    let wasm = std::fs::read(kumikit_test_plugins::HELLO_WASM).unwrap();
    let file = kkp(
        e.dir.path(),
        "hello",
        &component_manifest(crate::manifest::WORLD_PROVIDER),
        &[("plugin.wasm", &wasm)],
    );
    e.manager.install(&file).await.unwrap();
    let err = e
        .manager
        .enable("test.probe", &Grants::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not implement"), "{err}");
    assert_eq!(
        e.manager.get("test.probe").unwrap().record.state,
        PluginState::Installed
    );
}

#[tokio::test]
async fn opening_starts_no_wasm() {
    let e = env().await;
    let file = kkp(e.dir.path(), "p", DECLARATIVE, &[]);
    e.manager.install(&file).await.unwrap();
    e.manager
        .enable("test.declarative", &grant(&["fake-acp-agent"]))
        .await
        .unwrap();
    let reopened = open(&e.dir).await;
    assert!(!reopened.runtime().is_started());
}
