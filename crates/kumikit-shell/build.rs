fn main() {
    // Declaring the app's commands makes Tauri require a capability for each, so only
    // the windows `capabilities/` names can call them (ADR 0013).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "bridge_connect",
        "bridge_send",
        "bridge_close",
        "client_plugins_list",
        "client_plugins_install",
        "client_plugins_remove",
        "client_plugins_install_registry",
        "client_plugins_updates",
        "client_plugins_update",
        "client_registry_search",
        "client_registries_get",
        "client_registries_set",
        "client_lock",
        "profiles_list",
        "profile_probe",
        "profile_pair",
        "profile_add_ssh",
        "profile_update_ssh",
        "profile_remove",
        "ssh_probe",
        "ssh_sync",
        "app_update_info",
        "app_update_check",
        "app_update_download",
        "app_update_apply",
        "window_chrome",
        "pick_attachments",
        "test_report",
    ]);
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)) {
        panic!("tauri build failed: {e:#}");
    }
}
