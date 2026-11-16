//! Finds the branches every repository of a workspace has in common.
//!
//! The branch popup of a folder that holds several repositories lists, below the
//! repositories, the local and remote branches that every one of them has, as
//! IntelliJ IDEA lists its Common Local Branches and Common Remote Branches. An
//! action on such a branch, a checkout or a new branch from it, runs in every
//! repository at once. A branch that one repository lacks is not common, so the
//! popup offers it only in the panels of the repositories that have it.
//!
//! The lists keep the order of the first repository, the order the host read its
//! references in. A local branch is on in the workspace when every repository has
//! it checked out, and its upstream is common when every repository tracks the
//! same remote branch with it; otherwise the popup shows no upstream.

use super::{invalid, valid_id, MAX_REPOSITORIES};
use lithe_protocol::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Bounds the branch names across one request; larger lists are host bugs.
const MAX_BRANCHES: usize = 1_000_000;
/// Git refuses longer reference names on every supported platform.
const MAX_NAME_BYTES: usize = 4096;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    /// The repositories the popup lists, in the order it lists them.
    repositories: Vec<RepositoryReferences>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryReferences {
    /// The repository's `workspace.repositories` ID.
    id: String,
    /// The local branch the repository has checked out, absent when its HEAD is
    /// detached or unborn.
    #[serde(default)]
    current: Option<String>,
    /// The repository's local branches by short name, in the order the host read them.
    #[serde(default)]
    local: Vec<LocalBranch>,
    /// The repository's remote branches by short name, such as `origin/main`.
    #[serde(default)]
    remote: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalBranch {
    name: String,
    /// The short name of the remote branch the branch tracks, absent when none.
    #[serde(default)]
    upstream: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Response {
    /// The branch every repository has checked out, `null` when they differ or
    /// one is detached.
    current: Option<String>,
    /// The local branches every repository has, in the first repository's order.
    local: Vec<CommonLocalBranch>,
    /// The remote branches every repository has, in the first repository's order.
    remote: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct CommonLocalBranch {
    name: String,
    /// The remote branch every repository tracks with this branch, `null` when
    /// one tracks nothing or they track different branches.
    upstream: Option<String>,
    /// True when every repository has this branch checked out.
    current: bool,
}

/// Handles `git.workspaceCommonReferences`.
pub fn command(payload: Value) -> Result<Value, CoreError> {
    let request: Request = serde_json::from_value(payload).map_err(|error| {
        invalid("Invalid workspace common references request").with_details(error.to_string())
    })?;
    validate(&request.repositories)?;
    Ok(serde_json::to_value(common(&request.repositories))
        .expect("workspace common references should encode"))
}

/// Rejects requests a popup cannot send: too many repositories or branches, an
/// invalid or repeated ID, an empty or oversized branch name, or a branch listed
/// twice in one repository.
fn validate(repositories: &[RepositoryReferences]) -> Result<(), CoreError> {
    if repositories.len() > MAX_REPOSITORIES {
        return Err(invalid("Too many workspace repositories"));
    }
    let mut ids = HashSet::new();
    let mut total = 0usize;
    for repository in repositories {
        if !valid_id(&repository.id) || !ids.insert(repository.id.as_str()) {
            return Err(invalid("Invalid or repeated workspace repository ID"));
        }
        total = total
            .saturating_add(repository.local.len())
            .saturating_add(repository.remote.len());
        if total > MAX_BRANCHES {
            return Err(invalid("Too many workspace branches"));
        }
        if repository
            .current
            .as_deref()
            .is_some_and(|name| !valid_name(name))
        {
            return Err(invalid("Invalid current branch name"));
        }
        let mut local = HashSet::new();
        for branch in &repository.local {
            if !valid_name(&branch.name) || !local.insert(branch.name.as_str()) {
                return Err(invalid("Invalid or repeated local branch name"));
            }
            if branch
                .upstream
                .as_deref()
                .is_some_and(|name| !valid_name(name))
            {
                return Err(invalid("Invalid upstream branch name"));
            }
        }
        let mut remote = HashSet::new();
        for name in &repository.remote {
            if !valid_name(name) || !remote.insert(name.as_str()) {
                return Err(invalid("Invalid or repeated remote branch name"));
            }
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_BYTES && !name.contains('\0')
}

/// Intersects the repositories' branches in the first repository's order.
fn common(repositories: &[RepositoryReferences]) -> Response {
    let Some((first, rest)) = repositories.split_first() else {
        return Response {
            current: None,
            local: Vec::new(),
            remote: Vec::new(),
        };
    };
    let current = first
        .current
        .as_ref()
        .filter(|name| {
            rest.iter()
                .all(|other| other.current.as_ref() == Some(*name))
        })
        .cloned();
    // Each other repository's local branches by name, with the upstream it tracks.
    let others_local: Vec<HashMap<&str, Option<&str>>> = rest
        .iter()
        .map(|repository| {
            repository
                .local
                .iter()
                .map(|branch| (branch.name.as_str(), branch.upstream.as_deref()))
                .collect()
        })
        .collect();
    let local = first
        .local
        .iter()
        .filter_map(|branch| {
            let mut upstream = branch.upstream.as_deref();
            for other in &others_local {
                let tracked = other.get(branch.name.as_str())?;
                if *tracked != upstream {
                    upstream = None;
                }
            }
            Some(CommonLocalBranch {
                name: branch.name.clone(),
                upstream: upstream.map(str::to_owned),
                current: current.as_deref() == Some(branch.name.as_str()),
            })
        })
        .collect();
    let others_remote: Vec<HashSet<&str>> = rest
        .iter()
        .map(|repository| repository.remote.iter().map(String::as_str).collect())
        .collect();
    let remote = first
        .remote
        .iter()
        .filter(|name| {
            others_remote
                .iter()
                .all(|other| other.contains(name.as_str()))
        })
        .cloned()
        .collect();
    Response {
        current,
        local,
        remote,
    }
}
