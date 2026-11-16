//! Commands that delete something stay unfolded, so the console shows all a deletion did.
use crate::execution_policy::command_index;

/// Whether `arguments` delete branches, tags, references, remote branches, configuration,
/// files, stashes or worktrees, or list what a clean would delete. The console folds none
/// of such a command's configuration or output. Global options before the subcommand are
/// skipped, and only options before `--` count, so a path named like an option is a path.
pub(super) fn deletes(arguments: &[String]) -> bool {
    let Some(index) = command_index(arguments) else {
        return false;
    };
    let rest = &arguments[index + 1..];
    let has = |names: &[&str]| {
        rest.iter()
            .take_while(|argument| argument.as_str() != "--")
            .any(|argument| names.contains(&argument.as_str()))
    };
    let action = rest
        .iter()
        .find(|argument| !argument.starts_with('-'))
        .map(String::as_str);
    match arguments[index].as_str() {
        // A dry-run clean lists what a clean deletes, so it is shown whole as well.
        "rm" | "clean" => true,
        "branch" | "tag" | "update-ref" | "symbolic-ref" => has(&["-d", "-D", "--delete"]),
        // A refspec without a source, `:refs/heads/topic`, deletes the remote branch.
        "push" => {
            has(&["-d", "--delete", "--prune"])
                || rest
                    .iter()
                    .any(|argument| argument.starts_with(':') && argument.len() > 1)
        }
        "fetch" | "pull" => has(&["-p", "--prune", "-P", "--prune-tags"]),
        "remote" => {
            matches!(action, Some("remove" | "rm" | "prune"))
                || (action == Some("update") && has(&["-p", "--prune"]))
        }
        "config" => {
            has(&["--unset", "--unset-all", "--remove-section"])
                || matches!(action, Some("unset" | "remove-section"))
        }
        // `pop` drops the entry it applied.
        "stash" => matches!(action, Some("drop" | "clear" | "pop")),
        "worktree" => matches!(action, Some("remove" | "prune")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::deletes;
    use serde_json::Value;

    #[test]
    fn deletions_match_the_shared_console_cases() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/git/console-presentation-v1.json"
        ))
        .unwrap();
        let cases = fixture["deletions"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let arguments: Vec<String> = serde_json::from_value(case["arguments"].clone()).unwrap();
            assert_eq!(
                deletes(&arguments),
                case["deletes"].as_bool().unwrap(),
                "{arguments:?}"
            );
        }
    }
}
