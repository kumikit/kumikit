//! Bounded conversation handoff across provider switches, forks and lost sessions
//! (ADR 0029).

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::model::{ProviderModel, TurnId, TurnPurpose};
use serde::{Deserialize, Serialize};

use crate::repo::StoredEvent;

/// Maximum messages carried in a handoff context (ADR 0029).
pub const MAX_HANDOFF_MESSAGES: usize = 200;

/// Maximum JSON-encoded bytes for `messages` in a handoff context (64 KiB, ADR 0029).
pub const MAX_HANDOFF_BYTES: usize = 64 * 1024;

/// Fixed prompt sent to the outgoing provider in `summary` handoff mode.
pub const SUMMARY_PROMPT: &str = "Write a concise handoff note for another coding agent taking over this conversation. Cover: (1) the user's goal, (2) key decisions made, (3) current state of the work, (4) open items or next steps, and (5) relevant files. Plain text or markdown only; do not run tools or edit files.";

/// Pending handoff metadata stored on a thread until the next provider completes a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingHandoff {
    /// Outgoing provider and model.
    pub from: ProviderModel,
    /// Optional summary written by the outgoing provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// True when `summary` mode was requested and the summary turn failed or was empty.
    #[serde(default, skip_serializing_if = "is_false")]
    pub summary_failed: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Speaker of a handoff message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffRole {
    /// The user.
    User,
    /// The agent.
    Assistant,
}

/// One user or assistant text message carried in a handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffMessage {
    /// Who wrote the message.
    pub role: HandoffRole,
    /// Plain text of the message.
    pub text: String,
    /// Turn it belonged to.
    pub turn: TurnId,
}

/// Bounded conversation history built from a thread's event log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffContext {
    /// Provider that ran the earlier conversation.
    pub from_provider: String,
    /// Model that ran the earlier conversation.
    pub from_model: String,
    /// Newest messages, bounded by [`MAX_HANDOFF_MESSAGES`] and [`MAX_HANDOFF_BYTES`].
    pub messages: Vec<HandoffMessage>,
    /// Workspace-relative file paths from `file.changed` events, deduplicated in order.
    pub files_changed: Vec<String>,
    /// True when earlier messages were dropped or trimmed to fit the cap.
    pub truncated: bool,
    /// Optional handoff note from the outgoing agent.
    pub summary: Option<String>,
    /// True when a summary turn was attempted and failed.
    pub summary_failed: bool,
}

/// Builds a bounded [`HandoffContext`] from all events strictly before `stop_before_turn`
/// (or all events when `stop_before_turn` is `None`).
pub fn build_context(
    events: &[StoredEvent],
    from: ProviderModel,
    summary: Option<String>,
    summary_failed: bool,
    stop_before_turn: Option<&TurnId>,
) -> HandoffContext {
    let mut messages: Vec<HandoffMessage> = Vec::new();
    let mut files_changed: Vec<String> = Vec::new();
    let mut current_turn: Option<TurnId> = None;
    let mut skip_turn = false;
    let mut assistant = String::new();

    let flush_assistant =
        |messages: &mut Vec<HandoffMessage>, turn: &Option<TurnId>, buf: &mut String| {
            if !buf.is_empty()
                && let Some(t) = turn
            {
                messages.push(HandoffMessage {
                    role: HandoffRole::Assistant,
                    text: std::mem::take(buf),
                    turn: t.clone(),
                });
            } else {
                buf.clear();
            }
        };

    for stored in events {
        if let Some(stop) = stop_before_turn
            && let ThreadEvent::TurnStarted(e) = &stored.event
            && &e.turn == stop
        {
            break;
        }
        match &stored.event {
            ThreadEvent::TurnStarted(e) => {
                if !skip_turn {
                    flush_assistant(&mut messages, &current_turn, &mut assistant);
                } else {
                    assistant.clear();
                }
                current_turn = Some(e.turn.clone());
                skip_turn = e.purpose == Some(TurnPurpose::HandoffSummary);
                if !skip_turn && !e.input.text.is_empty() {
                    messages.push(HandoffMessage {
                        role: HandoffRole::User,
                        text: e.input.text.clone(),
                        turn: e.turn.clone(),
                    });
                }
            }
            ThreadEvent::InputSteered(e) => {
                if !skip_turn {
                    flush_assistant(&mut messages, &current_turn, &mut assistant);
                    if !e.input.text.is_empty() {
                        messages.push(HandoffMessage {
                            role: HandoffRole::User,
                            text: e.input.text.clone(),
                            turn: e.turn.clone(),
                        });
                    }
                }
            }
            ThreadEvent::TextDelta(e) => {
                if !skip_turn {
                    assistant.push_str(&e.text);
                }
            }
            ThreadEvent::TextDone(e) => {
                if !skip_turn {
                    assistant = e.text.clone();
                }
            }
            ThreadEvent::TurnEnded(_) => {
                if !skip_turn {
                    flush_assistant(&mut messages, &current_turn, &mut assistant);
                } else {
                    assistant.clear();
                }
                skip_turn = false;
            }
            ThreadEvent::FileChanged(e) if !skip_turn && !files_changed.contains(&e.path) => {
                files_changed.push(e.path.clone());
            }
            _ => {}
        }
    }
    if !skip_turn {
        flush_assistant(&mut messages, &current_turn, &mut assistant);
    }

    let truncated = cap_messages(&mut messages);

    HandoffContext {
        from_provider: from.provider,
        from_model: from.model,
        messages,
        files_changed,
        truncated,
        summary,
        summary_failed,
    }
}

/// Extracts the assistant text emitted during `turn` (for summary turns).
pub fn turn_assistant_text(events: &[StoredEvent], turn: &TurnId) -> Option<String> {
    let mut text = String::new();
    for stored in events {
        match &stored.event {
            ThreadEvent::TextDelta(e) if &e.turn == turn => text.push_str(&e.text),
            ThreadEvent::TextDone(e) if &e.turn == turn => text = e.text.clone(),
            _ => {}
        }
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Finds the last non-summary turn's `(provider, model)` in `events` strictly before
/// `stop_before_turn`, if any.
pub fn last_author(
    events: &[StoredEvent],
    stop_before_turn: Option<&TurnId>,
) -> Option<ProviderModel> {
    let mut last = None;
    for stored in events {
        if let ThreadEvent::TurnStarted(e) = &stored.event {
            if stop_before_turn == Some(&e.turn) {
                break;
            }
            if e.purpose != Some(TurnPurpose::HandoffSummary)
                && let (Some(provider), Some(model)) = (&e.provider, &e.model)
            {
                last = Some(ProviderModel {
                    provider: provider.clone(),
                    model: model.clone(),
                });
            }
        }
    }
    last
}

/// Drops oldest messages so at most [`MAX_HANDOFF_MESSAGES`] and [`MAX_HANDOFF_BYTES`]
/// remain. Returns true if anything was dropped or trimmed.
fn cap_messages(messages: &mut Vec<HandoffMessage>) -> bool {
    let mut truncated = false;
    if messages.len() > MAX_HANDOFF_MESSAGES {
        let drop_n = messages.len() - MAX_HANDOFF_MESSAGES;
        messages.drain(..drop_n);
        truncated = true;
    }
    while encoded_len(messages) > MAX_HANDOFF_BYTES {
        if messages.len() > 1 {
            messages.remove(0);
            truncated = true;
        } else if let Some(only) = messages.first_mut() {
            // Single message exceeds the byte cap: keep its newest UTF-8 suffix.
            let over = encoded_len(std::slice::from_ref(only)).saturating_sub(MAX_HANDOFF_BYTES);
            let cut = over.saturating_add(16).min(only.text.len());
            let mut start = cut;
            while start < only.text.len() && !only.text.is_char_boundary(start) {
                start += 1;
            }
            only.text.drain(..start);
            truncated = true;
            if encoded_len(std::slice::from_ref(only)) > MAX_HANDOFF_BYTES {
                only.text.clear();
                break;
            }
        } else {
            break;
        }
    }
    truncated
}

fn encoded_len(messages: &[HandoffMessage]) -> usize {
    serde_json::to_vec(messages).map_or(usize::MAX, |v| v.len())
}

/// Renders a [`HandoffContext`] into the shared prompt preamble (ADR 0029).
pub fn render(ctx: &HandoffContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "[Conversation handoff from {} ({})]\n",
        ctx.from_provider, ctx.from_model
    ));
    if ctx.summary_failed {
        out.push_str(
            "Note: an automatic handoff summary could not be generated; the conversation transcript is included below.\n",
        );
    }
    if let Some(summary) = &ctx.summary {
        out.push_str("\n## Handoff summary\n");
        out.push_str(summary.trim());
        out.push('\n');
    }
    if !ctx.files_changed.is_empty() {
        out.push_str("\n## Files changed earlier\n");
        for path in &ctx.files_changed {
            out.push_str("- ");
            out.push_str(path);
            out.push('\n');
        }
    }
    if ctx.truncated {
        out.push_str(
            "\n(Earlier messages were omitted to stay within the 200-message / 64 KiB handoff limit.)\n",
        );
    }
    if !ctx.messages.is_empty() {
        out.push_str("\n## Earlier conversation\n");
        for msg in &ctx.messages {
            let label = match msg.role {
                HandoffRole::User => "User",
                HandoffRole::Assistant => "Assistant",
            };
            out.push_str("\n### ");
            out.push_str(label);
            out.push('\n');
            out.push_str(&msg.text);
            if !msg.text.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out.push_str("\n[End of handoff — continue with the user's next message below]");
    out
}

/// Prepends a rendered handoff preamble to the user's message text.
pub fn prepend(rendered: &str, user_text: &str) -> String {
    if user_text.is_empty() {
        rendered.to_owned()
    } else {
        format!("{rendered}\n\n{user_text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kumikit_protocol::event::{FileChanged, InputSteered, TurnEnded, TurnStarted, TurnText};
    use kumikit_protocol::model::{Timestamp, TurnOutcome, UserInput};

    fn ev(seq: u64, event: ThreadEvent) -> StoredEvent {
        StoredEvent {
            seq,
            time: Timestamp(format!("t{seq}")),
            event,
        }
    }

    fn from() -> ProviderModel {
        ProviderModel {
            provider: "p.one/a".into(),
            model: "m1".into(),
        }
    }

    #[test]
    fn builds_and_renders_transcript_without_summary_turns() {
        let events = vec![
            ev(
                1,
                ThreadEvent::TurnStarted(TurnStarted {
                    turn: "u1".into(),
                    input: UserInput::text("fix the bug"),
                    from: None,
                    entry: None,
                    provider: Some("p.one/a".into()),
                    model: Some("m1".into()),
                    purpose: None,
                }),
            ),
            ev(
                2,
                ThreadEvent::TextDelta(TurnText {
                    turn: "u1".into(),
                    text: "looking".into(),
                }),
            ),
            ev(
                3,
                ThreadEvent::InputSteered(InputSteered {
                    turn: "u1".into(),
                    input: UserInput::text("also check tests"),
                }),
            ),
            ev(
                4,
                ThreadEvent::FileChanged(FileChanged {
                    turn: "u1".into(),
                    path: "src/lib.rs".into(),
                    diff: Some("+1".into()),
                }),
            ),
            ev(
                5,
                ThreadEvent::TextDone(TurnText {
                    turn: "u1".into(),
                    text: "fixed and tested".into(),
                }),
            ),
            ev(
                6,
                ThreadEvent::TurnEnded(TurnEnded {
                    turn: "u1".into(),
                    outcome: TurnOutcome::Done,
                    error: None,
                }),
            ),
            ev(
                7,
                ThreadEvent::TurnStarted(TurnStarted {
                    turn: "u2".into(),
                    input: UserInput::text(SUMMARY_PROMPT),
                    from: None,
                    entry: None,
                    provider: Some("p.one/a".into()),
                    model: Some("m1".into()),
                    purpose: Some(TurnPurpose::HandoffSummary),
                }),
            ),
            ev(
                8,
                ThreadEvent::TextDone(TurnText {
                    turn: "u2".into(),
                    text: "Bug in src/lib.rs is fixed.".into(),
                }),
            ),
            ev(
                9,
                ThreadEvent::TurnEnded(TurnEnded {
                    turn: "u2".into(),
                    outcome: TurnOutcome::Done,
                    error: None,
                }),
            ),
        ];

        let summary = turn_assistant_text(&events, &"u2".into());
        assert_eq!(summary.as_deref(), Some("Bug in src/lib.rs is fixed."));

        let ctx = build_context(&events, from(), summary, false, None);
        assert!(!ctx.truncated);
        assert_eq!(ctx.files_changed, vec!["src/lib.rs"]);
        assert_eq!(ctx.messages.len(), 4);
        assert_eq!(ctx.messages[0].role, HandoffRole::User);
        assert_eq!(ctx.messages[0].text, "fix the bug");
        assert_eq!(ctx.messages[1].role, HandoffRole::Assistant);
        assert_eq!(ctx.messages[1].text, "looking");
        assert_eq!(ctx.messages[2].role, HandoffRole::User);
        assert_eq!(ctx.messages[2].text, "also check tests");
        assert_eq!(ctx.messages[3].role, HandoffRole::Assistant);
        assert_eq!(ctx.messages[3].text, "fixed and tested");

        let rendered = render(&ctx);
        assert!(rendered.contains("p.one/a (m1)"));
        assert!(rendered.contains("## Handoff summary"));
        assert!(rendered.contains("Bug in src/lib.rs is fixed."));
        assert!(rendered.contains("- src/lib.rs"));
        assert!(!rendered.contains("Earlier messages were omitted"));
    }

    #[test]
    fn enforces_200_message_and_64_kib_caps_with_truncation_note() {
        let mut events = Vec::new();
        for i in 0..250u64 {
            let turn = TurnId(format!("u{i}"));
            events.push(ev(
                i * 2 + 1,
                ThreadEvent::TurnStarted(TurnStarted {
                    turn: turn.clone(),
                    input: UserInput::text(format!("msg-{i}")),
                    from: None,
                    entry: None,
                    provider: Some("p.one/a".into()),
                    model: Some("m1".into()),
                    purpose: None,
                }),
            ));
            events.push(ev(
                i * 2 + 2,
                ThreadEvent::TurnEnded(TurnEnded {
                    turn,
                    outcome: TurnOutcome::Done,
                    error: None,
                }),
            ));
        }
        let ctx = build_context(&events, from(), None, false, None);
        assert!(ctx.truncated);
        assert_eq!(ctx.messages.len(), MAX_HANDOFF_MESSAGES);
        assert_eq!(ctx.messages.first().unwrap().text, "msg-50");
        assert_eq!(ctx.messages.last().unwrap().text, "msg-249");
        assert!(render(&ctx).contains("Earlier messages were omitted"));

        // Byte cap: 20 messages of 5 KiB each -> exceeds 64 KiB, keeps newest.
        let mut big_events = Vec::new();
        for i in 0..20u64 {
            let turn = TurnId(format!("b{i}"));
            big_events.push(ev(
                i * 2 + 1,
                ThreadEvent::TurnStarted(TurnStarted {
                    turn: turn.clone(),
                    input: UserInput::text(format!("head-{i}-{}", "x".repeat(5 * 1024))),
                    from: None,
                    entry: None,
                    provider: None,
                    model: None,
                    purpose: None,
                }),
            ));
            big_events.push(ev(
                i * 2 + 2,
                ThreadEvent::TurnEnded(TurnEnded {
                    turn,
                    outcome: TurnOutcome::Done,
                    error: None,
                }),
            ));
        }
        let ctx = build_context(&big_events, from(), None, false, None);
        assert!(ctx.truncated);
        assert!(encoded_len(&ctx.messages) <= MAX_HANDOFF_BYTES);
        assert!(ctx.messages.last().unwrap().text.starts_with("head-19-"));
        assert!(ctx.messages.len() < 20);
    }
}
