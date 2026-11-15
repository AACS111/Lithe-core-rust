//! Trace correlation of the running request, inherited by the process traces
//! started inside it.

use crate::Source;
use std::cell::RefCell;

/// Correlation inherited by Git process traces started inside the request.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Operation shared by the user action, its requests and their Git processes.
    pub op: Option<String>,
    /// Provenance resolved once from the request envelope.
    pub source: Source,
}

thread_local! {
    static CURRENT: RefCell<Context> = RefCell::new(Context::default());
}

/// Snapshot for traces that finish on another thread, such as streamed history.
pub fn current() -> Context {
    CURRENT.with(|current| current.borrow().clone())
}

/// [`current`] for a panic hook: `None` instead of a second panic when the
/// thread is being torn down or the panic struck while the context was borrowed.
pub fn try_current() -> Option<Context> {
    CURRENT
        .try_with(|current| current.try_borrow().ok().map(|context| context.clone()))
        .ok()
        .flatten()
}

/// Makes `context` the running request's correlation and returns the enclosing
/// one, which the caller restores with another `replace` when its request ends.
pub fn replace(context: Context) -> Context {
    CURRENT.with(|current| current.replace(context))
}
