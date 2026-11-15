//! Event topics: `thread:<id>` is defined; `workspace:<id>`, `core` and `approvals` are
//! reserved (`docs/spec/02-protocol.md#events`).

use std::fmt;

use crate::model::ThreadId;

/// A parsed topic string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    /// `thread:<id>`: a thread's event log.
    Thread(ThreadId),
    /// A topic the spec reserves but does not define yet.
    Reserved(String),
}

impl Topic {
    /// Parses a topic string. Returns `None` for strings that are not a known or reserved
    /// topic.
    pub fn parse(value: &str) -> Option<Topic> {
        if let Some(id) = value.strip_prefix("thread:") {
            return (!id.is_empty()).then(|| Topic::Thread(ThreadId(id.to_owned())));
        }
        let reserved = value == "core"
            || value == "approvals"
            || value
                .strip_prefix("workspace:")
                .is_some_and(|id| !id.is_empty());
        reserved.then(|| Topic::Reserved(value.to_owned()))
    }

    /// The topic for a thread.
    pub fn thread(id: &ThreadId) -> Topic {
        Topic::Thread(id.clone())
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Topic::Thread(id) => write!(f, "thread:{id}"),
            Topic::Reserved(s) => f.write_str(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_topics() {
        assert_eq!(
            Topic::parse("thread:01J"),
            Some(Topic::Thread(ThreadId("01J".into())))
        );
        assert_eq!(Topic::parse("core"), Some(Topic::Reserved("core".into())));
        assert_eq!(
            Topic::parse("workspace:w"),
            Some(Topic::Reserved("workspace:w".into()))
        );
        assert_eq!(Topic::parse("thread:"), None);
        assert_eq!(Topic::parse("nope"), None);
    }

    #[test]
    fn displays_round_trip() {
        let t = Topic::thread(&ThreadId("abc".into()));
        assert_eq!(Topic::parse(&t.to_string()), Some(t));
    }
}
