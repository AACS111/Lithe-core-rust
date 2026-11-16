//! Workspace repository projections against their shared fixtures.

use super::{change_groups_command, selection_command, MAX_REPOSITORIES};
use lithe_protocol::CoreError;
use serde_json::{json, Value};

fn check_fixture(fixture: &str, command: fn(Value) -> Result<Value, CoreError>) {
    let fixture: Value = serde_json::from_str(fixture).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let response = command(case["request"].clone())
            .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
        assert_eq!(response, case["response"], "{}", case["name"]);
    }
    for case in fixture["invalid"].as_array().unwrap() {
        let error =
            command(case["request"].clone()).expect_err(case["name"].as_str().unwrap_or_default());
        assert_eq!(
            serde_json::to_value(error.code).unwrap(),
            "invalid_request",
            "{}",
            case["name"]
        );
    }
}

#[test]
fn selection_matches_the_shared_fixture() {
    check_fixture(
        include_str!("../../../../fixtures/git/workspace-repository-selection-v1.json"),
        selection_command,
    );
}

#[test]
fn change_groups_match_the_shared_fixture() {
    check_fixture(
        include_str!("../../../../fixtures/git/workspace-change-groups-v1.json"),
        change_groups_command,
    );
}

#[test]
fn repository_lists_past_the_bound_are_rejected() {
    let repositories = (0..=MAX_REPOSITORIES)
        .map(|index| {
            let id = format!("r{index}");
            json!({"path": format!("/w/{id}"), "id": id, "name": id, "role": "nested"})
        })
        .collect::<Vec<_>>();
    assert!(selection_command(json!({ "repositories": repositories })).is_err());
}

#[test]
fn ids_past_the_byte_bound_are_rejected() {
    let id = "a".repeat(super::MAX_ID_BYTES + 1);
    let request =
        json!({"repositories": [{"path": "/w/a", "id": id, "name": "a", "role": "nested"}]});
    assert!(selection_command(request).is_err());
}

#[test]
fn a_repository_at_a_filesystem_root_still_parents_nested_repositories() {
    let response = change_groups_command(json!({"repositories": [
        {"path": "/", "id": "..", "name": "/", "role": "workspace", "changes": [
            {"path": "w/sub", "staged": false, "worktree": true, "canToggleStaging": true,
             "submodule": {"commitChanged": false}}
        ]},
        {"path": "/w/sub", "id": "sub", "name": "sub", "role": "nested", "changes": [
            {"path": "a", "staged": false, "worktree": true, "canToggleStaging": true}
        ]}
    ]}))
    .unwrap();
    assert_eq!(response["groups"][1]["parentReference"], "unchanged");
}
