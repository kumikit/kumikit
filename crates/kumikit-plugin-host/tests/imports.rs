#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Components import only what their code uses: a base-world plugin does not import the
//! provider world's interfaces, and a provider that links both SDK crates still forms
//! one valid component.

use kumikit_plugin_host::engine::{EngineConfig, PluginEngine};
use wasmtime::component::Component;

fn imports(path: &str) -> Vec<String> {
    let cache = tempfile::tempdir().unwrap();
    let engine = PluginEngine::new(&EngineConfig::default(), cache.path()).unwrap();
    let component = Component::from_file(engine.engine(), path).unwrap();
    let ty = component.component_type();
    let mut names: Vec<String> = ty
        .imports(engine.engine())
        .map(|(name, _)| name.to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn base_plugin_imports_no_provider_interfaces() {
    let names = imports(kumikit_test_plugins::HELLO_WASM);
    assert!(
        names.contains(&"kumikit:host/log@0.2.0".to_owned()),
        "{names:?}"
    );
    for denied in ["events", "process", "secrets"] {
        let name = format!("kumikit:host/{denied}@0.1.0");
        assert!(!names.contains(&name), "{name} in {names:?}");
    }
}

#[test]
fn provider_plugin_imports_events() {
    let names = imports(kumikit_test_plugins::PROBE_WASM);
    assert!(
        names.contains(&"kumikit:host/events@0.2.0".to_owned()),
        "{names:?}"
    );
}
