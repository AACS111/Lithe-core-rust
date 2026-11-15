//! Verifies notification fixtures through the public JSON command envelope.

use crate::execute_json;
use serde_json::{json, Value};

#[test]
fn notification_read_fixture_crosses_the_core_dispatch() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/workbench/notification-read-state-v1.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let response: Value = serde_json::from_str(&execute_json(
            &json!({
                "id": "notification-read-fixture",
                "command": fixture["command"],
                "payload": case["payload"]
            })
            .to_string(),
        ))
        .unwrap();
        assert_eq!(response["ok"], true, "{}: {response}", case["name"]);
        assert_eq!(response["data"], case["expected"], "{}", case["name"]);
    }
    for case in fixture["errors"].as_array().unwrap() {
        let response: Value = serde_json::from_str(&execute_json(
            &json!({
                "id": "notification-read-invalid-fixture",
                "command": fixture["command"],
                "payload": case["payload"]
            })
            .to_string(),
        ))
        .unwrap();
        assert_eq!(response["ok"], false, "{}", case["name"]);
        assert_eq!(response["error"]["code"], case["error"], "{}", case["name"]);
    }
}
