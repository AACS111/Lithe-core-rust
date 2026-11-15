//! Records every Core panic as a catalog fact, and keeps a panic inside a
//! request from unwinding into the host.
//!
//! Core runs inside the host process. A panic that reaches the C ABI aborts
//! that process before the host's log writer drains any queued fact, so the
//! dispatcher runs each request inside [`request_boundary`], which turns a
//! panic into a failed response. Threads Core starts keep the standard
//! behavior: the panic unwinds that thread and its owner decides what happens
//! next. In both cases the hook installed here records one
//! `app.core.panic_interrupted` fact before unwinding starts, then runs the
//! hook that was installed before it, so the standard stderr report remains.

use lithe_diagnostics::core_events;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::sync::Once;

const EVENT: &str = "app.core.panic_interrupted";
static INSTALL: Once = Once::new();

thread_local! {
    /// Method of the request this thread runs inside [`request_boundary`].
    static REQUEST: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Installs the recording hook once per process, in front of the hook that
/// was installed before it. Later calls are cheap.
pub(crate) fn install() {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            record(info);
            previous(info);
        }));
    });
}

/// Runs one request so that a panic inside it returns `None` to the
/// dispatcher instead of unwinding across the C ABI.
pub(crate) fn request_boundary<T>(method: &str, run: impl FnOnce() -> T) -> Option<T> {
    let enclosing = REQUEST.with(|request| request.replace(Some(method.to_owned())));
    // Asserting unwind safety is sound here: the closure's captures are
    // consumed by the panicking request and never observed afterwards. State
    // shared with later requests sits behind locks; one the panic poisoned
    // either recovers or fails the next request the same way, and that panic
    // is caught and recorded too.
    let result = std::panic::catch_unwind(AssertUnwindSafe(run));
    REQUEST.with(|request| request.replace(enclosing));
    result.ok()
}

/// Runs inside the panic hook, so it must neither wait for a lock the
/// panicking thread may hold nor panic: a second panic aborts the process.
fn record(info: &PanicHookInfo<'_>) {
    let op = lithe_request_context::trace::try_current().and_then(|context| context.op);
    core_events::record_without_waiting(EVENT, fields(info, current_method()), op);
}

fn current_method() -> Option<String> {
    REQUEST
        .try_with(|request| request.try_borrow().ok().and_then(|method| method.clone()))
        .ok()
        .flatten()
}

fn fields(info: &PanicHookInfo<'_>, method: Option<String>) -> BTreeMap<String, Value> {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("Panic payload is not text");
    let boundary = if method.is_some() {
        "request"
    } else {
        "thread"
    };
    let mut fields = BTreeMap::from([
        ("boundary".to_string(), json!(boundary)),
        ("panic".to_string(), json!(message)),
    ]);
    if let Some(location) = info.location() {
        fields.insert(
            "location".into(),
            json!(format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )),
        );
    }
    if let Some(method) = method.as_deref().and_then(super::request_trace::token) {
        fields.insert("method".into(), json!(method));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use lithe_diagnostics::core_events::{capture, CoreEvent};

    fn panic_facts(events: Vec<CoreEvent>) -> Vec<CoreEvent> {
        events
            .into_iter()
            .filter(|event| event.event == EVENT)
            .collect()
    }

    #[test]
    fn a_panicking_request_is_recorded_and_returns_none() {
        install();
        let (result, events) = capture::run(false, || {
            request_boundary("git.write", || -> i32 { panic!("request broke") })
        });
        assert!(result.is_none());
        let facts = panic_facts(events);
        assert_eq!(facts.len(), 1);
        let fact = &facts[0];
        assert_eq!(fact.channel, "app");
        assert_eq!(fact.fields["boundary"], json!("request"));
        assert_eq!(fact.fields["method"], json!("git.write"));
        assert_eq!(fact.fields["panic"], json!("request broke"));
        assert!(fact.fields["location"]
            .as_str()
            .unwrap()
            .contains("panic_facts.rs:"));
        assert_eq!(current_method(), None);
    }

    #[test]
    fn a_request_that_returns_keeps_its_value_and_records_nothing() {
        install();
        let (result, events) = capture::run(false, || request_boundary("git.write", || 7));
        assert_eq!(result, Some(7));
        assert!(panic_facts(events).is_empty());
    }

    #[test]
    fn a_panic_outside_a_request_is_recorded_as_thread() {
        install();
        let (result, events) = capture::run(false, || {
            std::panic::catch_unwind(|| panic!("{} broke", String::from("worker")))
        });
        assert!(result.is_err());
        let facts = panic_facts(events);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fields["boundary"], json!("thread"));
        assert_eq!(facts[0].fields["panic"], json!("worker broke"));
        assert!(!facts[0].fields.contains_key("method"));
    }

    #[test]
    fn a_panic_payload_that_is_not_text_gets_a_fixed_message() {
        install();
        let (_, events) = capture::run(false, || {
            std::panic::catch_unwind(|| std::panic::panic_any(42_u8))
        });
        let facts = panic_facts(events);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fields["panic"], json!("Panic payload is not text"));
    }

    #[test]
    fn nested_requests_restore_the_enclosing_method() {
        install();
        let (_, events) = capture::run(false, || {
            request_boundary("workspace.repositories", || {
                assert!(request_boundary("git.write", || panic!("inner")).is_none());
                assert_eq!(current_method().as_deref(), Some("workspace.repositories"));
            })
        });
        let facts = panic_facts(events);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].fields["method"], json!("git.write"));
        assert_eq!(current_method(), None);
    }

    #[test]
    fn a_method_that_is_not_a_token_is_left_out() {
        install();
        let (_, events) = capture::run(false, || {
            request_boundary("not a token", || panic!("odd method"))
        });
        let facts = panic_facts(events);
        assert_eq!(facts[0].fields["boundary"], json!("request"));
        assert!(!facts[0].fields.contains_key("method"));
    }
}
