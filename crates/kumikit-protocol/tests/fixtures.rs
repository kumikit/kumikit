//! Contract tests: recorded protocol messages must parse into their Rust types and
//! serialize back to the same JSON. A change that breaks a fixture is a breaking protocol
//! change (`docs/spec/02-protocol.md#versioning`).
//!
//! Layout: `fixtures/methods/<method>.json` holds an array of requests and their
//! responses (success or error); `fixtures/events/<kind>.json` holds `events.event`
//! notifications of that kind.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::jsonrpc::{ErrorResponse, Notification, Request, SuccessResponse};
use kumikit_protocol::methods::{self, Method, MethodVisitor, NotificationMethod};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

type Check = fn(&Value) -> Result<(), String>;

/// Parses `value` as `T` and checks it serializes back to the same JSON.
fn round_trip<T: Serialize + DeserializeOwned>(value: &Value) -> Result<(), String> {
    let parsed: T = serde_json::from_value(value.clone()).map_err(|e| format!("parse: {e}"))?;
    let back = serde_json::to_value(&parsed).map_err(|e| format!("serialize: {e}"))?;
    if &back == value {
        Ok(())
    } else {
        Err(format!(
            "round trip changed the message:\n  in:  {value}\n  out: {back}"
        ))
    }
}

#[derive(Default)]
struct Table {
    params: BTreeMap<&'static str, Check>,
    results: BTreeMap<&'static str, Check>,
    notifications: BTreeMap<&'static str, Check>,
}

impl MethodVisitor for Table {
    fn method<M: Method>(&mut self) {
        self.params.insert(M::NAME, round_trip::<M::Params>);
        self.results.insert(M::NAME, round_trip::<M::Result>);
    }
    fn notification<N: NotificationMethod>(&mut self) {
        self.notifications.insert(N::NAME, round_trip::<N::Params>);
    }
}

fn table() -> Table {
    let mut t = Table::default();
    methods::visit_all(&mut t);
    t
}

fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
}

/// `(stem, messages)` for every fixture file in a directory.
fn load(sub: &str) -> Vec<(String, Vec<Value>)> {
    let mut out: Vec<_> = fs::read_dir(dir(sub))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| {
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = fs::read_to_string(&p).unwrap();
            let messages: Vec<Value> = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{}: not a JSON array: {e}", p.display()));
            (stem, messages)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn method_fixtures_round_trip() {
    let t = table();
    let mut failures = Vec::new();
    for (method, messages) in load("methods") {
        let Some(check_params) = t.params.get(method.as_str()) else {
            failures.push(format!("{method}: no such method in the table"));
            continue;
        };
        let check_result = t.results[method.as_str()];
        for (i, msg) in messages.iter().enumerate() {
            let at = format!("{method}[{i}]");
            let outcome = if msg.get("method").is_some() {
                round_trip::<Request>(msg).and_then(|()| {
                    let req: Request = serde_json::from_value(msg.clone()).unwrap();
                    if req.method != method {
                        return Err(format!("request names method {}", req.method));
                    }
                    req.params.as_ref().map_or(Ok(()), check_params)
                })
            } else if msg.get("result").is_some() {
                round_trip::<SuccessResponse>(msg).and_then(|()| check_result(&msg["result"]))
            } else if msg.get("error").is_some() {
                round_trip::<ErrorResponse>(msg)
            } else {
                Err("neither a request nor a response".into())
            };
            if let Err(e) = outcome {
                failures.push(format!("{at}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn event_fixtures_round_trip() {
    let t = table();
    let check = t.notifications["events.event"];
    let mut failures = Vec::new();
    for (kind, messages) in load("events") {
        for (i, msg) in messages.iter().enumerate() {
            let outcome = round_trip::<Notification>(msg).and_then(|()| {
                if msg["method"] != "events.event" {
                    return Err("not an events.event notification".into());
                }
                if msg["params"]["kind"] != kind.as_str() {
                    return Err(format!("kind is {}", msg["params"]["kind"]));
                }
                check(&msg["params"])
            });
            if let Err(e) = outcome {
                failures.push(format!("{kind}[{i}]: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_method_and_event_kind_has_a_fixture() {
    let methods: BTreeMap<String, Vec<Value>> = load("methods").into_iter().collect();
    let events: BTreeMap<String, Vec<Value>> = load("events").into_iter().collect();
    let mut missing = Vec::new();
    for name in methods::METHOD_NAMES {
        let msgs = methods.get(*name).map(Vec::as_slice).unwrap_or_default();
        if !msgs.iter().any(|m| m.get("method").is_some()) {
            missing.push(format!("request for {name}"));
        }
        if !msgs.iter().any(|m| m.get("result").is_some()) {
            missing.push(format!("success response for {name}"));
        }
    }
    for kind in ThreadEvent::KINDS {
        if events.get(kind).is_none_or(Vec::is_empty) {
            missing.push(format!("event {kind}"));
        }
    }
    assert!(
        missing.is_empty(),
        "missing fixtures: {}",
        missing.join(", ")
    );
}
