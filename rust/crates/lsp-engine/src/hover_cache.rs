//! Hover answers a session already received, so resting on a symbol again answers at
//! once instead of asking the language server a second time.
//!
//! An answer is reused only for the same document version and a position the answer
//! covers. Anything that can change what the server knows clears every answer, not
//! only those of one document: an edit in one file changes the hover shown for its
//! symbols in every other file.

use super::*;

/// Most answers one session keeps; the least recently used one goes first.
pub(crate) const HOVER_CACHE_CAPACITY: usize = 32;

/// Longest hover text, in bytes, worth keeping. A longer answer is still delivered
/// but asked for again next time, which bounds a session's cache to
/// `HOVER_CACHE_CAPACITY * HOVER_CACHE_MAX_CONTENT_BYTES` of hover text.
pub(crate) const HOVER_CACHE_MAX_CONTENT_BYTES: usize = 64 * 1024;

/// Zero-based LSP line and UTF-16 column, ordered line first.
type Point = (i64, i64);

/// What a hover request in flight needs to keep its answer.
#[derive(Debug, Clone)]
pub(crate) struct HoverRequest {
    /// Document version the request was sent for; the answer describes that text.
    pub(crate) version: i64,
    pub(crate) position: Point,
    /// Cache generation when the request was sent. The answer is kept only if
    /// nothing cleared the cache before it arrived, because the server may have
    /// computed it from what it knew before the change.
    pub(crate) generation: u64,
}

/// One kept answer.
struct HoverEntry {
    uri: String,
    version: i64,
    position: Point,
    /// Same-line half-open column span `(line, start, end)` the server gave as the
    /// answer's range, when that range contains `position`. Any position inside it
    /// shows this answer, the way editors keep a hover open across its range.
    span: Option<(i64, i64, i64)>,
    /// The `requestCompleted` result exactly as the host received it.
    result: Value,
}

impl HoverEntry {
    fn covers(&self, uri: &str, version: i64, position: Point) -> bool {
        if self.uri != uri || self.version != version {
            return false;
        }
        position == self.position
            || self.span.is_some_and(|(line, start, end)| {
                position.0 == line && start <= position.1 && position.1 < end
            })
    }
}

/// Hover answers of one session, owned by its `SessionState`.
#[derive(Default)]
pub(crate) struct HoverCache {
    /// Advances on every clear; see `HoverRequest::generation`.
    generation: u64,
    /// Least recently used first.
    entries: VecDeque<HoverEntry>,
}

impl HoverCache {
    /// Forgets every answer. Called whenever the server may know something it did
    /// not know when it answered: a document opened, changed, saved or closed, files
    /// changed on disk, settings changed, a command ran, a plugin backend was
    /// notified or sent the server its own request, or the server asked the client
    /// for something or finished a piece of work.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// The answer kept for `position` in this version of the document, marked as
    /// the most recently used.
    pub(crate) fn lookup(&mut self, uri: &str, version: i64, position: Point) -> Option<Value> {
        let index = self
            .entries
            .iter()
            .rposition(|entry| entry.covers(uri, version, position))?;
        let entry = self.entries.remove(index)?;
        let result = entry.result.clone();
        self.entries.push_back(entry);
        Some(result)
    }

    /// Describes a hover request about to be sent for `position`.
    pub(crate) fn request(&self, version: i64, position: Point) -> HoverRequest {
        HoverRequest {
            version,
            position,
            generation: self.generation,
        }
    }

    /// Keeps the answer to `request`. An empty or oversized answer, or one the
    /// server computed before the cache was last cleared, is not kept: an empty
    /// answer can turn into a real one once the server has loaded the project.
    pub(crate) fn store(&mut self, uri: &str, request: &HoverRequest, result: &Value) {
        if request.generation != self.generation {
            return;
        }
        let Some(hover) = result.get("hover").filter(|hover| hover.is_object()) else {
            return;
        };
        let Some(contents) = hover.get("contents").and_then(Value::as_str) else {
            return;
        };
        if contents.trim().is_empty() || contents.len() > HOVER_CACHE_MAX_CONTENT_BYTES {
            return;
        }
        let span = same_line_span(hover.get("range")).filter(|(line, start, end)| {
            request.position.0 == *line && *start <= request.position.1 && request.position.1 < *end
        });
        self.entries.retain(|entry| {
            !(entry.uri == uri
                && entry.version == request.version
                && entry.position == request.position)
        });
        self.entries.push_back(HoverEntry {
            uri: uri.to_string(),
            version: request.version,
            position: request.position,
            span,
            result: result.clone(),
        });
        while self.entries.len() > HOVER_CACHE_CAPACITY {
            self.entries.pop_front();
        }
    }
}

/// A hover range from `lithe_lsp_client` (`{start, end}` of `{line, utf16Column}`)
/// as `(line, start, end)`, when it is a non-empty span on one line. A range over
/// several lines is not used to answer other positions: it can enclose other
/// symbols with hovers of their own.
fn same_line_span(range: Option<&Value>) -> Option<(i64, i64, i64)> {
    let range = range?;
    let point = |key: &str| -> Option<Point> {
        let position = range.get(key)?;
        Some((
            position.get("line")?.as_i64()?,
            position.get("utf16Column")?.as_i64()?,
        ))
    };
    let (start, end) = (point("start")?, point("end")?);
    (start.0 == end.0 && start.1 < end.1).then_some((start.0, start.1, end.1))
}

/// Whether a message the server sent may mean it knows something new: any request
/// it makes of the client (refreshes, edits, capability changes, configuration
/// pulls, progress tokens) and the end of a piece of work such as loading the
/// workspace.
pub(crate) fn server_message_clears_hovers(value: &Value) -> bool {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return false;
    };
    value.get("id").is_some()
        || (method == "$/progress"
            && value.pointer("/params/value/kind").and_then(Value::as_str) == Some("end"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(contents: &str) -> Value {
        json!({ "hover": { "contents": contents, "isMarkdown": false, "range": null } })
    }

    fn keep(cache: &mut HoverCache, line: i64) {
        let request = cache.request(1, (line, 0));
        cache.store("file:///a.go", &request, &answer(&format!("line {line}")));
    }

    #[test]
    fn the_least_recently_used_answer_goes_first_at_capacity() {
        let mut cache = HoverCache::default();
        for line in 0..HOVER_CACHE_CAPACITY as i64 {
            keep(&mut cache, line);
        }
        // Reading line 0 makes line 1 the oldest.
        assert!(cache.lookup("file:///a.go", 1, (0, 0)).is_some());
        keep(&mut cache, HOVER_CACHE_CAPACITY as i64);
        assert_eq!(cache.entries.len(), HOVER_CACHE_CAPACITY);
        assert!(cache.lookup("file:///a.go", 1, (1, 0)).is_none());
        assert!(cache.lookup("file:///a.go", 1, (0, 0)).is_some());
        assert!(cache
            .lookup("file:///a.go", 1, (HOVER_CACHE_CAPACITY as i64, 0))
            .is_some());
    }

    #[test]
    fn an_answer_is_kept_once_per_position_and_only_for_its_document_version() {
        let mut cache = HoverCache::default();
        keep(&mut cache, 3);
        keep(&mut cache, 3);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.lookup("file:///a.go", 2, (3, 0)).is_none());
        assert!(cache.lookup("file:///b.go", 1, (3, 0)).is_none());
        assert_eq!(
            cache.lookup("file:///a.go", 1, (3, 0)),
            Some(answer("line 3"))
        );
    }

    #[test]
    fn a_clear_forgets_answers_and_refuses_ones_requested_before_it() {
        let mut cache = HoverCache::default();
        keep(&mut cache, 0);
        let in_flight = cache.request(1, (5, 0));
        cache.clear();
        assert!(cache.lookup("file:///a.go", 1, (0, 0)).is_none());
        cache.store("file:///a.go", &in_flight, &answer("stale"));
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn only_server_requests_and_finished_work_clear_answers() {
        let clears = |message: Value| server_message_clears_hovers(&message);
        assert!(clears(
            json!({ "id": 1, "method": "workspace/codeLens/refresh" })
        ));
        assert!(clears(json!({
            "method": "$/progress",
            "params": { "token": "t", "value": { "kind": "end" } }
        })));
        assert!(!clears(json!({
            "method": "$/progress",
            "params": { "token": "t", "value": { "kind": "report" } }
        })));
        assert!(!clears(
            json!({ "method": "textDocument/publishDiagnostics", "params": {} })
        ));
        assert!(!clears(json!({ "id": 1, "result": null })));
    }
}
