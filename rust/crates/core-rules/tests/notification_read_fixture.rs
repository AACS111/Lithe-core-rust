//! Checks shared notification read transitions through the dispatch used by Core.

use lithe_core_rules::{execute, RuleErrorCode};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("../../../fixtures/workbench/notification-read-state-v1.json");
const COMMAND: &str = "workbench.notifications.reduceReadState";

#[test]
fn shared_read_transitions_match() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["version"], 1);
    for case in fixture["cases"].as_array().unwrap() {
        let actual = execute(COMMAND, case["payload"].clone()).unwrap();
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}

#[test]
fn malformed_read_transitions_are_rejected() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    for case in fixture["errors"].as_array().unwrap() {
        let error = execute(COMMAND, case["payload"].clone()).unwrap_err();
        assert_eq!(
            error.code,
            RuleErrorCode::InvalidRequest,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn unread_metadata_is_bounded() {
    let ids: Vec<u64> = (1..=1024).collect();
    let state = json!({"unreadIds": ids});
    let payload =
        json!({"state": state, "action": {"kind": "arrived", "id": 1025, "historyVisible": false}});
    assert_eq!(
        execute(COMMAND, payload).unwrap_err().code,
        RuleErrorCode::InvalidRequest
    );
    let payload = json!({"state": state, "action": {"kind": "arrived", "id": 1025, "forgottenId": 4, "historyVisible": false}});
    assert_eq!(execute(COMMAND, payload).unwrap()["unreadCount"], 1024);
}
