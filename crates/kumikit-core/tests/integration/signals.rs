//! A SIGTERM right after `ready` shuts the core down cleanly: the signal handlers exist
//! before the line is printed. (They used to be registered lazily, and an early SIGTERM
//! killed the core outright.)

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use support::{Core, Dirs};

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_right_after_ready_is_a_clean_shutdown() {
    for _ in 0..10 {
        let dirs = Dirs::new();
        // `stop` sends SIGTERM and asserts a successful exit.
        Core::start(&dirs).await.stop().await;
    }
}
