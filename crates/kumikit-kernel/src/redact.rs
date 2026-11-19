//! Redacting secret values from log output (`docs/spec/03-kernel.md#secrets`).
//!
//! The secret store records every value it loads or stores here; the core's log writer
//! passes each line through [`redact`] before it reaches stderr. The list is process-wide
//! because logging is. Values shorter than [`MIN_LEN`] are not redacted: replacing every
//! occurrence of a short string would mangle ordinary log lines.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::RwLock;

/// Shortest secret value that is redacted.
pub const MIN_LEN: usize = 8;

/// What a redacted value is replaced with.
pub const MARK: &str = "[secret]";

static VALUES: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// Adds values to redact (ones shorter than [`MIN_LEN`] are ignored). Values stay listed
/// for the life of the process, so a deleted secret that was logged earlier in the same
/// run is still hidden.
pub fn remember<'a>(values: impl IntoIterator<Item = &'a str>) {
    let fresh: Vec<&str> = values.into_iter().filter(|v| v.len() >= MIN_LEN).collect();
    if fresh.is_empty() {
        return;
    }
    let mut set = VALUES.write().unwrap_or_else(|e| e.into_inner());
    for v in fresh {
        if !set.contains(v) {
            set.insert(v.to_owned());
        }
    }
}

/// `text` with every remembered value replaced by [`MARK`].
pub fn redact(text: &str) -> Cow<'_, str> {
    let set = VALUES.read().unwrap_or_else(|e| e.into_inner());
    if set.is_empty() || !set.iter().any(|v| text.contains(v.as_str())) {
        return Cow::Borrowed(text);
    }
    // Longest first, so a value that contains another is replaced whole.
    let mut values: Vec<&String> = set.iter().collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    let mut out = text.to_owned();
    for v in values {
        if out.contains(v.as_str()) {
            out = out.replace(v.as_str(), MARK);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_values_are_replaced_and_short_ones_ignored() {
        remember(["redact-test-value-1234", "short"]);
        assert_eq!(
            redact("token=redact-test-value-1234 and short"),
            "token=[secret] and short"
        );
        assert!(matches!(redact("nothing here"), Cow::Borrowed(_)));
    }

    #[test]
    fn a_longer_value_containing_another_is_replaced_whole() {
        remember(["redact-inner-9876", "prefix-redact-inner-9876-suffix"]);
        assert_eq!(
            redact("x prefix-redact-inner-9876-suffix y"),
            "x [secret] y"
        );
    }
}
