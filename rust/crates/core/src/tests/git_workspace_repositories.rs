//! `git.workspaceRepositorySelection` and `git.workspaceChangeGroups` against
//! their shared fixtures, and against real discovery and status output passed
//! through unchanged.

use super::support::temporary_root;
use crate::execute_json;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn call(command: &str, payload: Value) -> Value {
    serde_json::from_str(&execute_json(
        &json!({"id":"workspace-repositories-test", "command":command, "payload":payload})
            .to_string(),
    ))
    .unwrap()
}

fn data(command: &str, payload: Value) -> Value {
    let response = call(command, payload);
    assert_eq!(response["ok"], true, "{command}: {response}");
    response["data"].clone()
}

fn git(root: &Path, arguments: &[&str]) {
    // The shared host owns the subprocess deadline and process-tree cleanup.
    let result = data("git.command", json!({"root": root, "arguments": arguments}));
    assert_eq!(result["exitCode"], 0, "{arguments:?}: {result}");
}

fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Workspace Test"]);
    git(root, &["config", "user.email", "workspace@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.autocrlf", "false"]);
}

fn check_fixture(command: &str, fixture: &str) {
    let fixture: Value = serde_json::from_str(fixture).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let response = call(command, case["request"].clone());
        assert_eq!(response["ok"], true, "{}: {response}", case["name"]);
        assert_eq!(response["data"], case["response"], "{}", case["name"]);
    }
    for case in fixture["invalid"].as_array().unwrap() {
        let response = call(command, case["request"].clone());
        assert_eq!(response["ok"], false, "{}: {response}", case["name"]);
        assert_eq!(
            response["error"]["code"], "invalid_request",
            "{}",
            case["name"]
        );
    }
}

#[test]
fn selection_and_change_groups_match_their_shared_fixtures() {
    check_fixture(
        "git.workspaceRepositorySelection",
        include_str!("../../../../fixtures/git/workspace-repository-selection-v1.json"),
    );
    check_fixture(
        "git.workspaceChangeGroups",
        include_str!("../../../../fixtures/git/workspace-change-groups-v1.json"),
    );
}

/// Discovery entries with the status each repository reports, as hosts send them.
fn repositories_with_changes(root: &Path) -> Value {
    let mut repositories =
        data("workspace.repositories", json!({"root": root}))["repositories"].clone();
    for repository in repositories.as_array_mut().unwrap() {
        let status = data("git.status", json!({"root": repository["path"]}));
        repository["changes"] = status["changes"].clone();
    }
    repositories
}

fn group<'a>(groups: &'a Value, id: &str) -> &'a Value {
    groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["id"] == id)
        .unwrap_or_else(|| panic!("no group {id}: {groups}"))
}

#[test]
fn discovery_and_status_pass_through_to_selection_and_change_groups() {
    let parent = Folder(temporary_root("workspace-change-groups"));
    init(&parent.0);
    let child = parent.0.join("libs/B");
    init(&child);
    fs::write(child.join("hello.ts"), "one").unwrap();
    git(&child, &["add", "hello.ts"]);
    git(&child, &["commit", "-qm", "child initial"]);
    fs::write(parent.0.join("parent.txt"), "parent").unwrap();
    fs::write(parent.0.join(".gitignore"), "extras/\n").unwrap();
    fs::write(
        parent.0.join(".gitmodules"),
        "[submodule \"B\"]\npath = libs/B\nurl = ./libs/B\n",
    )
    .unwrap();
    git(&parent.0, &["add", "."]);
    git(&parent.0, &["commit", "-qm", "parent initial"]);
    let extras = parent.0.join("extras");
    init(&extras);
    fs::write(extras.join("notes.txt"), "notes").unwrap();
    fs::write(child.join("hello.ts"), "two").unwrap();

    let discovered =
        data("workspace.repositories", json!({"root": parent.0}))["repositories"].clone();
    let selection = data(
        "git.workspaceRepositorySelection",
        json!({"repositories": discovered, "selected": "libs/B"}),
    );
    assert_eq!(
        selection,
        json!({"active": "libs/B", "fellBack": false, "others": [".", "extras"]})
    );

    // Child content alone leaves the parent's recorded commit where it was.
    let groups = data(
        "git.workspaceChangeGroups",
        json!({"repositories": repositories_with_changes(&parent.0)}),
    );
    let ids = groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| group["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, [".", "extras", "libs/B"], "{groups}");
    assert_eq!(group(&groups, ".")["toggleable"], false, "{groups}");
    assert!(group(&groups, ".").get("staging").is_none(), "{groups}");
    assert!(group(&groups, "extras").get("parentReference").is_none());
    assert_eq!(
        group(&groups, "extras")["staging"],
        json!({"operation": "stage", "paths": ["notes.txt"]})
    );
    assert_eq!(group(&groups, "libs/B")["parentReference"], "unchanged");
    assert_eq!(
        groups["stageAll"],
        json!({"operation": "stage", "repositories": [
            {"id": "extras", "paths": ["notes.txt"]},
            {"id": "libs/B", "paths": ["hello.ts"]}
        ]})
    );

    // A child commit moves the gitlink; the child keeps changes so its group stays.
    git(&child, &["commit", "-qam", "child update"]);
    fs::write(child.join("hello.ts"), "three").unwrap();
    let groups = data(
        "git.workspaceChangeGroups",
        json!({"repositories": repositories_with_changes(&parent.0)}),
    );
    assert_eq!(
        group(&groups, "libs/B")["parentReference"],
        "moved",
        "{groups}"
    );
}
