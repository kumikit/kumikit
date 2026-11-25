//! Consolidated integration test harness for `kumikit-core` (keeps link count low;
//! `footprint`, `exit_m0` and `exit_m1` stay separate because scripts/docs invoke them by
//! `--test` name).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../support/mod.rs"]
mod support;

mod cli;
mod client_api;
mod client_interface;
mod commands;
mod defer;
mod exit_m4;
mod exit_m5;
mod fork_and_side;
mod gateway;
mod handoff;
mod http;
mod permission_wording;
mod plugins;
mod queue;
mod registry;
mod remote;
mod schedules;
mod settings;
#[cfg(unix)]
mod signals;
mod steer;
mod web;
