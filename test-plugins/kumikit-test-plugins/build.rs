//! Builds the test plugins for `wasm32-wasip2` with a nested `cargo build`, in a target
//! directory of their own so it never waits on the outer build's lock.
//!
//! The outer build's environment describes the host build (flags, target, profile), so
//! it is cleared before the nested cargo runs.

// A build script reports failure by panicking; the messages say what to fix.
#![allow(clippy::expect_used)]

use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Guest packages built for wasm32-wasip2: (package, output file stem).
const GUESTS: &[(&str, &str)] = &[
    ("kumikit-test-hello", "kumikit_test_hello"),
    ("kumikit-test-probe", "kumikit_test_probe"),
];

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest_dir
        .ancestors()
        .nth(2)
        .expect("test-plugins/<crate> is two levels below the workspace root")
        .to_path_buf();
    let target_dir = root.join("target").join("kumikit-test");

    for dir in ["wit", "sdk/rust", "test-plugins", "plugins", "Cargo.lock"] {
        let path = root.join(dir);
        // A path that does not exist would make cargo rerun this script on every build.
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(&root)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip2",
        ])
        .arg("--target-dir")
        .arg(&target_dir);
    for (package, _) in GUESTS {
        cmd.args(["-p", package]);
    }
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("CARGO_") && key != "CARGO_HOME"
            || key == "RUSTFLAGS"
            || key == "RUSTC_WRAPPER"
            || key == "RUSTC_WORKSPACE_WRAPPER"
        {
            cmd.env_remove(key.as_ref());
        }
    }
    let status = cmd.status().expect("running cargo for the test plugins");
    if !status.success() {
        panic!(
            "building the test plugins for wasm32-wasip2 failed. The target must be installed: \
             `rustup target add wasm32-wasip2` (rust-toolchain.toml lists it), or your \
             distribution's wasm package for a system toolchain."
        );
    }

    let out = target_dir.join("wasm32-wasip2").join("release");
    for (package, stem) in GUESTS {
        let path = out.join(format!("{stem}.wasm"));
        assert!(
            path.is_file(),
            "{package} built but {} is missing",
            path.display()
        );
        let var = format!("KUMIKIT_TEST_WASM_{}", stem.to_uppercase());
        println!("cargo:rustc-env={var}={}", path.display());
    }
}
