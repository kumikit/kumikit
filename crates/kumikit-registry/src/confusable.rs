//! Confusable plugin ids (`docs/spec/10-distribution.md#registry`): registry CI refuses a
//! new id that reads like an existing one, so a typo-squatted plugin cannot be published
//! next to the real one.
//!
//! Ids are ASCII (`[a-z0-9-.]`), so there are no Unicode homoglyphs; the check compares
//! skeletons (lookalike letters folded, `-` dropped) and edit distance.

/// Publishers no one else may use or imitate.
pub const RESERVED_PUBLISHERS: &[&str] = &["kumikit", "test", "local"];

/// The id with lookalikes folded: lowercase, `-` removed, `rn`→`m`, `vv`→`w`, and
/// `0`→`o`, `1`→`l`, `i`→`l`, `5`→`s`.
pub fn skeleton(id: &str) -> String {
    let lowered: String = id
        .to_ascii_lowercase()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect();
    let folded = lowered.replace("rn", "m").replace("vv", "w");
    folded
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            '5' => 's',
            other => other,
        })
        .collect()
}

/// Damerau–Levenshtein distance (optimal string alignment).
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
}

/// Why a new id is refused, or `None` if it may be published next to `existing`.
pub fn conflict(new_id: &str, existing: &[String]) -> Option<String> {
    let publisher = new_id.split('.').next().unwrap_or_default();
    for reserved in RESERVED_PUBLISHERS {
        if publisher != *reserved
            && (skeleton(publisher) == skeleton(reserved) || distance(publisher, reserved) <= 1)
        {
            return Some(format!(
                "the publisher `{publisher}` looks like the reserved publisher `{reserved}`"
            ));
        }
    }
    let new_skeleton = skeleton(new_id);
    for old in existing.iter().filter(|o| o.as_str() != new_id) {
        if skeleton(old) == new_skeleton {
            return Some(format!("`{new_id}` reads like `{old}`"));
        }
        if old.len() >= 5 && new_id.len() >= 5 && distance(old, new_id) <= 1 {
            return Some(format!("`{new_id}` is one letter away from `{old}`"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn confusable_ids_refused() {
        let existing = ids(&["kumikit.sample-tool", "acme.git", "acme.modern-ui"]);
        for bad in [
            "kumikit.sample-toal",  // one letter
            "kumikit.samplee-tool", // one insertion
            "kumikit.sample-loot",  // not one edit away
            "acme.gjt",             // substitution
            "acme.glt",             // i/l lookalike via skeleton? (one edit anyway)
            "acme.rnodern-ui",      // rn/m
            "acme.modern_ui",       // separators dropped
            "acme.g1t",             // 1/i lookalike
            "kumikti.anything",     // looks like the reserved publisher
            "kumik1t.anything",
        ] {
            let refused = conflict(bad, &existing);
            if bad == "kumikit.sample-loot" {
                assert!(refused.is_none(), "{bad}: a reordered word is fine");
                continue;
            }
            assert!(refused.is_some(), "{bad} should be refused");
        }
    }

    #[test]
    fn distinct_ids_pass() {
        let existing = ids(&["acme.git", "acme.github"]);
        for ok in [
            "acme.gitlab",
            "other.tool",
            "acme.git-graph",
            "kumikit.theme-night",
        ] {
            assert_eq!(conflict(ok, &existing), None, "{ok}");
        }
        // An id never conflicts with itself (a new version of the same plugin).
        assert_eq!(conflict("acme.git", &existing), None);
    }

    #[test]
    fn distance_counts_transpositions_once() {
        assert_eq!(distance("abcd", "abdc"), 1);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "ab"), 2);
    }
}
