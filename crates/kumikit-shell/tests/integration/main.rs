//! Consolidated integration test harness for `kumikit-shell` (`remote` stays separate
//! because docs invoke `cargo test -p kumikit-shell --test remote -- --ignored`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod local_core;
mod registry;
