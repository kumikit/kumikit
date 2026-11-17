//! Test hooks, compiled only with the `test-hooks` feature (never in releases).
//!
//! WebDriver cannot drive native dialogs, so a stand-in `dialog` plugin answers the UI
//! kernel's `open()` calls from a queue file: `KUMIKIT_TEST_PICKS` names a file of paths,
//! one per line; each `open` takes the first line (an empty line means "cancelled").

use std::path::Path;

/// Takes the next answer from the queue file, or `None` if it is empty or missing.
pub fn next_pick(queue: &Path) -> Option<String> {
    let text = std::fs::read_to_string(queue).ok()?;
    let mut lines = text.lines();
    let first = lines.next()?.to_owned();
    let rest: Vec<&str> = lines.collect();
    let _ = std::fs::write(queue, rest.join("\n"));
    (!first.is_empty()).then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_answers_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let q = dir.path().join("picks");
        std::fs::write(&q, "/a\n\n/b").unwrap();
        assert_eq!(next_pick(&q).as_deref(), Some("/a"));
        assert_eq!(next_pick(&q), None, "an empty line is a cancel");
        assert_eq!(next_pick(&q).as_deref(), Some("/b"));
        assert_eq!(next_pick(&q), None);
        assert_eq!(next_pick(&dir.path().join("missing")), None);
    }
}
