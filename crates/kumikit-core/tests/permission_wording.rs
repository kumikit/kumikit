//! The CLI's permission wording against the fixture the desktop plugin manager also
//! checks, so both say the same.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kumikit_core::cli::describe;
use kumikit_protocol::methods::Permissions;
use serde_json::Value;

#[test]
fn wording_matches_the_shared_fixture() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/permission-wording.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let p: Permissions = serde_json::from_value(case["permissions"].clone()).unwrap();
        let lines: Vec<String> = serde_json::from_value(case["lines"].clone()).unwrap();
        assert_eq!(describe(&p), lines);
    }
}
