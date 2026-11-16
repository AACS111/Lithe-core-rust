//! Chooses the repository a multi-repository view acts on.
//!
//! The branch popup and the Git Log each act on one repository of the workspace
//! and list the others as groups the user can switch to. A view keeps the
//! repository the user picked while discovery still lists it; otherwise it falls
//! back to the repository that contains the opened folder, and in a folder that
//! is no repository itself, to the first nested repository.

use super::{invalid, validate_discovered, DiscoveredRepository};
use crate::contracts::WorkspaceRepositoryRole;
use lithe_protocol::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    /// Discovery entries in discovery order.
    repositories: Vec<DiscoveredRepository>,
    /// ID the view acted on before, absent when it has not picked one.
    #[serde(default)]
    selected: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    /// ID the view acts on, `null` when the workspace has no repository.
    active: Option<String>,
    /// True when the picked repository is no longer listed, so the view drops
    /// state that belonged to it, such as an open history walk or a reference filter.
    fell_back: bool,
    /// Every other ID in discovery order: the groups the view lists for switching.
    others: Vec<String>,
}

/// Handles `git.workspaceRepositorySelection`.
pub fn command(payload: Value) -> Result<Value, CoreError> {
    let request: Request = serde_json::from_value(payload).map_err(|error| {
        invalid("Invalid workspace repository selection request").with_details(error.to_string())
    })?;
    validate_discovered(&request.repositories)?;
    if request.selected.as_deref().is_some_and(str::is_empty) {
        return Err(invalid(
            "A selected workspace repository ID cannot be empty",
        ));
    }
    let listed = |id: &str| {
        request
            .repositories
            .iter()
            .any(|repository| repository.id == id)
    };
    let kept = request.selected.as_deref().filter(|id| listed(id));
    let fell_back = request.selected.is_some() && kept.is_none();
    let active = kept
        .or_else(|| {
            request
                .repositories
                .iter()
                .find(|repository| repository.role == WorkspaceRepositoryRole::Workspace)
                .map(|repository| repository.id.as_str())
        })
        .or_else(|| {
            request
                .repositories
                .first()
                .map(|repository| repository.id.as_str())
        })
        .map(str::to_owned);
    let others = request
        .repositories
        .iter()
        .filter(|repository| Some(&repository.id) != active.as_ref())
        .map(|repository| repository.id.clone())
        .collect();
    Ok(serde_json::to_value(Response {
        active,
        fell_back,
        others,
    })
    .expect("workspace repository selection should encode"))
}
