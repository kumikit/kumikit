//! The kernel's status rules against the fixture the UI kernel also checks, so the
//! client derives the same status as the core.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kumikit_kernel::fold::Fold;
use kumikit_protocol::ThreadEvent;
use kumikit_protocol::model::{ThreadId, ThreadStatus, Timestamp};
use serde_json::Value;

#[test]
fn statuses_match_the_shared_fixture() {
    let text = include_str!("fixtures/status.json");
    let fixture: Value = serde_json::from_str(text).unwrap();
    let thread = ThreadId(fixture["thread"].as_str().unwrap().into());
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut fold = Fold::new(thread.clone());
        let expected: Vec<ThreadStatus> = serde_json::from_value(case["statuses"].clone()).unwrap();
        let events = case["events"].as_array().unwrap();
        assert_eq!(events.len(), expected.len(), "{name}");
        for (i, (e, want)) in events.iter().zip(expected).enumerate() {
            let event: ThreadEvent = serde_json::from_value(e.clone()).unwrap();
            fold.apply(i as u64 + 1, &Timestamp("t".into()), &event);
            assert_eq!(fold.status, want, "{name}, after event {}", i + 1);
        }
    }
}
