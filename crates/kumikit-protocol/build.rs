//! Build script: stamps the release version (`KUMIKIT_RELEASE_VERSION`, falling back to
//! `CARGO_PKG_VERSION`) into `KUMIKIT_APP_VERSION` so `kumikit`, `core.hello`, and the
//! desktop shell all report the exact release version.

fn main() {
    println!("cargo:rerun-if-env-changed=KUMIKIT_RELEASE_VERSION");
    let raw = std::env::var("KUMIKIT_RELEASE_VERSION").unwrap_or_default();
    let trimmed = raw.trim();
    let version = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let effective = if version.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        version
    };
    println!("cargo:rustc-env=KUMIKIT_APP_VERSION={effective}");
}
