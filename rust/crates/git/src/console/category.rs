//! The console category of each Git request command's internal queries.
use super::types::QueryCategory;

/// The category a request command's internal queries read, or `None` outside Git. Commands
/// that only show what Git has go by what they show; previews, preflights, plans and the
/// operations themselves are checks, so a command not listed here counts as one.
pub fn of_command(command: &str) -> Option<QueryCategory> {
    use QueryCategory::*;
    let category = match command {
        "git.diff" | "git.comparison" | "git.planDiffBlocks" => Diff,
        "git.history"
        | "git.historyQuery"
        | "git.historyPage"
        | "git.historyWindow"
        | "git.historyLocate"
        | "git.historyActionCapabilities"
        | "git.references"
        | "git.referenceTree"
        | "git.graphProjection"
        | "git.tags"
        | "git.commit"
        | "git.commitFiles"
        | "git.changedFileGroups" => Log,
        "git.blame" | "git.lineHistory" => Annotate,
        "git.worktrees"
        | "git.stashes"
        | "git.remotes"
        | "git.remoteUrl"
        | "git.pullRequestContext"
        | "git.untrackedDirectory"
        | "git.repositorySetup"
        | "git.executionInspect" => Panels,
        command if command.starts_with("git.") => Checks,
        _ => return None,
    };
    Some(category)
}
