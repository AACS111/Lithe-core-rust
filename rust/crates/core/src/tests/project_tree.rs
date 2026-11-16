//! `workspace.projectTree.rows` against the shared project tree rows fixture.

use serde_json::{json, Value};

fn execute(command: &str, payload: Value) -> Value {
    serde_json::from_str(&crate::execute_json(
        &json!({"command": command, "payload": payload}).to_string(),
    ))
    .unwrap()
}

fn run(request: &Value) -> Value {
    execute("workspace.projectTree.rows", request.clone())
}

#[test]
fn shared_project_tree_rows_fixture_runs_every_case() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/workspace/project-tree-rows-v1.json"
    )))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let response = run(&case["request"]);
        assert_eq!(response["ok"], true, "case {}: {response}", case["name"]);
        assert_eq!(response["data"], case["response"], "case {}", case["name"]);
    }
    for case in fixture["invalid"].as_array().unwrap() {
        let response = run(&case["request"]);
        assert_eq!(response["ok"], false, "case {}: {response}", case["name"]);
        assert_eq!(
            response["error"]["code"], "invalid_request",
            "case {}",
            case["name"]
        );
    }
}

/// Issue 2009: Spring Boot on macOS writes a Windows-style `logging.file.path`
/// such as `D:\logs` as one folder name. On Unix `\` and `:` are ordinary
/// file-name characters, so the listing keeps each entry one level below its
/// folder and the project tree accepts the rows built from it.
#[cfg(unix)]
#[test]
fn names_with_backslashes_stay_one_level_below_their_folder_on_unix() {
    let root = super::support::temporary_root("backslash-file-names");
    std::fs::create_dir_all(root.join(r"D:\logs")).unwrap();
    std::fs::write(root.join(r"D:\logs").join("spring.log"), "").unwrap();
    std::fs::write(root.join(r"D:\logs\app.log"), "").unwrap();

    let listing = execute(
        "workspace.listDirectories",
        json!({"root": root, "paths": ["", r"D:\logs"]}),
    );
    let removed = std::fs::remove_dir_all(&root);
    assert_eq!(listing["ok"], true, "{listing}");
    removed.expect("temporary fixture should be removable");

    let children = |index: usize| {
        listing["data"]["directories"][index]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|child| child["path"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(children(0), [r"D:\logs", r"D:\logs\app.log"]);
    assert_eq!(listing["data"]["directories"][1]["path"], r"D:\logs");
    assert_eq!(children(1), [r"D:\logs/spring.log"]);

    let rows = run(&json!({"nodes": [
        {"path": "", "depth": 0, "open": true},
        {"path": r"D:\logs", "depth": 1, "open": true},
        {"path": r"D:\logs/spring.log", "depth": 2},
        {"path": r"D:\logs\app.log", "depth": 1},
    ]}));
    assert_eq!(rows["ok"], true, "{rows}");
    assert_eq!(rows["data"]["rows"], json!([0, 1, 2, 3]));
}
