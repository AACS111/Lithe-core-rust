//! Notification read transitions; hosts retain text, toasts and native presentation.

use crate::{RuleError, RuleErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// The Core command for notification read transitions.
pub const REDUCE_NOTIFICATION_READ_STATE: &str = "workbench.notifications.reduceReadState";

// A wire safety bound, independent of the host's smaller visible-history limit.
const MAX_UNREAD_IDS: usize = 1024;

/// Window-owned read metadata, with no notification content or display-order dependency.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadState {
    /// Unique positive host identities of unread history records, newest arrival first.
    #[serde(default)]
    pub unread_ids: Vec<u64>,
}

/// One host event; a repeated record retains its original identity.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ReadAction {
    /// A new arrival or repeat, after the host updates its bounded display history.
    Arrived {
        /// Stable positive history-record identity, not the replacing toast identity.
        id: u64,
        /// Identity evicted from the display history, whether previously read or unread.
        forgotten_id: Option<u64>,
        /// The host is currently presenting either history entry point to the user.
        history_visible: bool,
    },
    /// Either history entry point was viewed, or its history was explicitly cleared.
    MarkAllRead,
}

/// The next state and its ready-to-render count; hosts do not recalculate the count.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReduction {
    /// Metadata to supply with the next event, scoped to the same native window.
    pub state: ReadState,
    /// Number of distinct unread records, independent of notification display order.
    pub unread_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    #[serde(default)]
    state: ReadState,
    action: ReadAction,
}

fn invalid() -> RuleError {
    RuleError::new(
        RuleErrorCode::InvalidRequest,
        "Invalid notification read request",
    )
}

/// Applies an event without clocks or retained global state.
///
/// Rejects zero or duplicate identities and metadata exceeding the wire bound.
/// Repeated arrivals count once; eviction can remove any unread identity, so no
/// prefix invariant in the host's display list is required.
pub fn reduce_read_state(
    mut state: ReadState,
    action: ReadAction,
) -> Result<ReadReduction, RuleError> {
    let mut seen = HashSet::with_capacity(state.unread_ids.len().min(MAX_UNREAD_IDS));
    if state.unread_ids.len() > MAX_UNREAD_IDS
        || state
            .unread_ids
            .iter()
            .any(|&id| id == 0 || !seen.insert(id))
    {
        return Err(invalid());
    }
    match action {
        ReadAction::MarkAllRead => state.unread_ids.clear(),
        ReadAction::Arrived {
            id,
            forgotten_id,
            history_visible,
        } => {
            if id == 0 || forgotten_id == Some(0) {
                return Err(invalid());
            }
            if history_visible {
                state.unread_ids.clear();
            } else {
                state
                    .unread_ids
                    .retain(|&old| old != id && Some(old) != forgotten_id);
                if state.unread_ids.len() == MAX_UNREAD_IDS {
                    return Err(invalid());
                }
                state.unread_ids.insert(0, id);
            }
        }
    }
    let unread_count = state.unread_ids.len();
    Ok(ReadReduction {
        state,
        unread_count,
    })
}

/// Decodes the command payload and returns the deterministic read projection.
pub fn reduce_read_state_json(payload: Value) -> Result<Value, RuleError> {
    let request: Request = serde_json::from_value(payload)
        .map_err(|error| invalid().with_details(error.to_string()))?;
    let reduction = reduce_read_state(request.state, request.action)?;
    serde_json::to_value(reduction).map_err(|error| invalid().with_details(error.to_string()))
}
