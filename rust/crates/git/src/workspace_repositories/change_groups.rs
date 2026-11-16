//! Groups the changes of every workspace repository and plans their staging.
//!
//! The Commit tool window of a workspace with several repositories lists one
//! line for all changes and one group per repository that has changes. Each
//! group's checkbox and the line's checkbox stage everything they can, or
//! unstage everything once it is all staged. A nested repository that is a
//! submodule of a listed ancestor also learns whether that ancestor's recorded
//! reference to it moved, so views can say a commit here still has to be
//! recorded in the parent.

use super::{invalid, validate_identities, Identity};
use crate::contracts::WorkspaceRepositoryRole;
use lithe_platform::paths::slash_separated_path;
use lithe_platform::Platform;
use lithe_protocol::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Bounds the changes of one request across every repository.
const MAX_CHANGES: usize = 1_000_000;

/// One `workspace.repositories` entry with the changes `git.status` read for it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Repository {
    path: String,
    id: String,
    name: String,
    role: WorkspaceRepositoryRole,
    /// `git.status` changes in the order status returned them.
    changes: Vec<Change>,
}

/// The fields of a `git.status` change this command reads. Other fields, such as
/// `status` and `untracked`, are accepted so hosts pass changes through verbatim.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    path: String,
    #[serde(default)]
    original_path: Option<String>,
    staged: bool,
    worktree: bool,
    can_toggle_staging: bool,
    #[serde(default)]
    submodule: Option<Submodule>,
    #[serde(default)]
    untracked_file_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Submodule {
    commit_changed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    /// Discovery entries in discovery order.
    repositories: Vec<Repository>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Operation {
    Stage,
    Unstage,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Staging {
    operation: Operation,
    /// Repository-relative paths in change order, both sides of a rename.
    paths: Vec<String>,
}

/// Whether the listed ancestor that records a nested repository as a submodule
/// moved its recorded commit.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum ParentReference {
    /// The ancestor's status lists the gitlink with a new commit.
    Moved,
    /// The ancestor's status lists the gitlink with only content changes.
    Unchanged,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    id: String,
    name: String,
    role: WorkspaceRepositoryRole,
    /// Files the changes stand for: a summarized untracked directory counts each file.
    file_count: usize,
    staged_count: usize,
    /// Every change that can toggle is staged in full, and there is one.
    all_staged: bool,
    /// Some change can be staged or unstaged.
    toggleable: bool,
    /// What the group's checkbox writes, absent when it writes nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    staging: Option<Staging>,
    /// Absent when no listed ancestor records the repository as a submodule.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_reference: Option<ParentReference>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryStaging {
    id: String,
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StageAll {
    operation: Operation,
    /// Repositories with something to write, in discovery order.
    repositories: Vec<RepositoryStaging>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    /// Repositories with changes, in discovery order.
    groups: Vec<Group>,
    file_count: usize,
    staged_count: usize,
    all_staged: bool,
    /// What the line's checkbox writes, absent when it writes nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    stage_all: Option<StageAll>,
}

/// Handles `git.workspaceChangeGroups`.
pub fn command(payload: Value) -> Result<Value, CoreError> {
    let request: Request = serde_json::from_value(payload).map_err(|error| {
        invalid("Invalid workspace change groups request").with_details(error.to_string())
    })?;
    let repositories = request.repositories;
    validate_identities(repositories.iter().map(|repository| Identity {
        id: &repository.id,
        path: &repository.path,
        name: &repository.name,
        role: repository.role,
    }))?;
    validate_changes(&repositories)?;

    let parent_references = parent_references(&repositories);
    let all_changes = || {
        repositories
            .iter()
            .flat_map(|repository| &repository.changes)
    };
    let everything_staged = all_staged(all_changes());
    let stage_all_operation = operation(everything_staged);
    let mut stage_all = Vec::new();
    let mut groups = Vec::new();
    for (index, repository) in repositories.iter().enumerate() {
        let paths = staging_paths(&repository.changes, stage_all_operation);
        if !paths.is_empty() {
            stage_all.push(RepositoryStaging {
                id: repository.id.clone(),
                paths,
            });
        }
        if repository.changes.is_empty() {
            continue;
        }
        let group_all_staged = all_staged(repository.changes.iter());
        let group_operation = operation(group_all_staged);
        let paths = staging_paths(&repository.changes, group_operation);
        groups.push(Group {
            id: repository.id.clone(),
            name: repository.name.clone(),
            role: repository.role,
            file_count: file_count(repository.changes.iter()),
            staged_count: staged_count(repository.changes.iter()),
            all_staged: group_all_staged,
            toggleable: repository
                .changes
                .iter()
                .any(|change| change.can_toggle_staging),
            staging: (!paths.is_empty()).then_some(Staging {
                operation: group_operation,
                paths,
            }),
            parent_reference: parent_references[index],
        });
    }
    Ok(serde_json::to_value(Response {
        groups,
        file_count: file_count(all_changes()),
        staged_count: staged_count(all_changes()),
        all_staged: everything_staged,
        stage_all: (!stage_all.is_empty()).then_some(StageAll {
            operation: stage_all_operation,
            repositories: stage_all,
        }),
    })
    .expect("workspace change groups should encode"))
}

fn validate_changes(repositories: &[Repository]) -> Result<(), CoreError> {
    let count = repositories
        .iter()
        .map(|repository| repository.changes.len())
        .sum::<usize>();
    if count > MAX_CHANGES {
        return Err(invalid("Too many workspace changes"));
    }
    let empty = |path: &str| path.is_empty() || path.contains('\0');
    if repositories
        .iter()
        .flat_map(|repository| &repository.changes)
        .any(|change| empty(&change.path) || change.original_path.as_deref().is_some_and(empty))
    {
        return Err(invalid("Invalid workspace change path"));
    }
    Ok(())
}

/// A checkbox that shows everything staged unstages; any other state stages.
fn operation(all_staged: bool) -> Operation {
    if all_staged {
        Operation::Unstage
    } else {
        Operation::Stage
    }
}

/// True when every change that can toggle is staged with nothing left in the
/// worktree, and at least one such change exists.
fn all_staged<'a>(changes: impl Iterator<Item = &'a Change>) -> bool {
    let mut found = false;
    for change in changes.filter(|change| change.can_toggle_staging) {
        if !change.staged || change.worktree {
            return false;
        }
        found = true;
    }
    found
}

fn file_count<'a>(changes: impl Iterator<Item = &'a Change>) -> usize {
    changes
        .map(|change| {
            change
                .untracked_file_count
                .filter(|count| *count > 0)
                .unwrap_or(1)
        })
        .sum()
}

fn staged_count<'a>(changes: impl Iterator<Item = &'a Change>) -> usize {
    changes.filter(|change| change.staged).count()
}

/// Paths a checkbox writes: unstaging takes every staged change, staging every
/// change with something left to stage. Changes that cannot toggle, such as a
/// gitlink with only uncommitted child content, are never written.
fn staging_paths(changes: &[Change], operation: Operation) -> Vec<String> {
    changes
        .iter()
        .filter(|change| change.can_toggle_staging)
        .filter(|change| match operation {
            Operation::Unstage => change.staged,
            Operation::Stage => !change.staged || change.worktree,
        })
        .flat_map(|change| change.original_path.iter().chain([&change.path]))
        .cloned()
        .collect()
}

/// For each repository, what its deepest listed ancestor records for it as a
/// submodule. Paths are compared as host paths, so a remote workspace's
/// enclosing repository and its nested ones share ancestry.
fn parent_references(repositories: &[Repository]) -> Vec<Option<ParentReference>> {
    let paths = repositories
        .iter()
        .map(|repository| lithe_workspace::remote_path(Path::new(&repository.path)))
        .collect::<Vec<PathBuf>>();
    let by_path = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_path(), index))
        .collect::<HashMap<_, _>>();
    let gitlinks = repositories
        .iter()
        .map(|repository| {
            repository
                .changes
                .iter()
                .filter_map(|change| {
                    let submodule = change.submodule.as_ref()?;
                    Some((change.path.trim_end_matches('/'), submodule.commit_changed))
                })
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();
    paths
        .iter()
        .map(|path| {
            let (parent, ancestor) = path
                .ancestors()
                .skip(1)
                .find_map(|ancestor| Some((*by_path.get(ancestor)?, ancestor)))?;
            let relative = path.strip_prefix(ancestor).ok()?;
            let moved =
                gitlinks[parent].get(slash_separated_path(relative, Platform::HOST).as_str())?;
            Some(if *moved {
                ParentReference::Moved
            } else {
                ParentReference::Unchanged
            })
        })
        .collect()
}
