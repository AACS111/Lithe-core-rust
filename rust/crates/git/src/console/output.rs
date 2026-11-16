//! IDEA-style progress and query folding; ordinary command output remains continuous text.
use super::types::{Display, OutputFragment, Record};
use regex::Regex;
use std::sync::OnceLock;

/// Output of at most this many lines stays visible: opening a fold would cost more than
/// reading the lines it hides.
const VISIBLE_LINES: usize = 10;

/// Folds nothing of a command that `deletes`, so every line of a deletion stays in sight.
pub(super) fn project(record: &Record, display: &Display, deletes: bool) -> Vec<OutputFragment> {
    let folds = !deletes && !record.failed();
    // A query's output answers the workflow, not the user: like IDEA's silent handlers it
    // stays out of sight, folded whole, once it is longer than a glance, or at any length
    // when the console's display choices ask.
    let visible = if display.fold_short_queries {
        0
    } else {
        VISIBLE_LINES
    };
    if folds && record.query && record.lines.len() > visible {
        return vec![fragment(0, record.lines.len(), "query")];
    }
    let mut result = Vec::new();
    let mut start = 0;
    while start < record.lines.len() {
        let mut end = start + 1;
        if looks_like_progress(&record.lines[start].text) {
            while end < record.lines.len() && looks_like_progress(&record.lines[end].text) {
                end += 1;
            }
        }
        // A failed command keeps its progress and diagnostics expanded for inspection.
        if folds && end - start > VISIBLE_LINES {
            result.push(fragment(start, end, "progress"));
        } else {
            result.extend((start..end).map(|line| fragment(line, line + 1, "text")));
        }
        start = end;
    }
    result
}

fn fragment(start: usize, end: usize, kind: &str) -> OutputFragment {
    OutputFragment {
        id: format!("output-{start}"),
        start,
        end,
        kind: kind.into(),
        count: end - start,
        added: 0,
        updated: 0,
        deleted: 0,
        matches: 0,
    }
}

/// Matches IDEA's GitImplBase progress grammar, including remote-side phases.
fn looks_like_progress(line: &str) -> bool {
    static PERCENT: OnceLock<Regex> = OnceLock::new();
    let percent = PERCENT.get_or_init(|| {
        Regex::new(r":\s*\d{1,3}% \(\d+/\d+\)").expect("constant Git progress pattern")
    });
    if percent.is_match(line) {
        return true;
    }
    let line = line.strip_prefix("remote: ").unwrap_or(line);
    [
        "Counting objects: ",
        "Enumerating objects: ",
        "Compressing objects: ",
        "Writing objects: ",
        "Receiving objects: ",
        "Resolving deltas: ",
        "Finding sources: ",
        "Updating files: ",
        "Checking out files: ",
        "Expanding reachable commits in commit graph: ",
        "Delta compression using up to ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}
