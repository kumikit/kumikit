//! Folding a thread's event log into state. The same rules derive the stored thread
//! status on append, so a snapshot and `thread.list` cannot disagree.

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::model::{
    Approval, ApprovalStatus, ThreadId, ThreadStatus, Timestamp, Turn, TurnOutcome,
};

/// The thread status after `event`, given the status before it and the number of
/// approvals still pending after it. `archived` is not part of the log and never
/// produced here.
pub fn next_status(
    current: ThreadStatus,
    event: &ThreadEvent,
    pending_after: usize,
) -> ThreadStatus {
    match event {
        ThreadEvent::TurnStarted(_) => ThreadStatus::Running,
        ThreadEvent::ApprovalRequested(_) => ThreadStatus::AwaitingApproval,
        ThreadEvent::ApprovalResolved(_) if pending_after == 0 => ThreadStatus::Running,
        ThreadEvent::ApprovalResolved(_) => ThreadStatus::AwaitingApproval,
        ThreadEvent::TurnEnded(e) => match e.outcome {
            TurnOutcome::Failed => ThreadStatus::Failed,
            TurnOutcome::Done | TurnOutcome::Cancelled => ThreadStatus::Idle,
        },
        _ => current,
    }
}

/// State folded from a thread's events.
#[derive(Debug, Clone, PartialEq)]
pub struct Fold {
    /// The thread.
    pub thread: ThreadId,
    /// Status from the log (never `archived`).
    pub status: ThreadStatus,
    /// Turns, oldest first.
    pub turns: Vec<Turn>,
    /// Every approval, oldest first, with its current status.
    pub approvals: Vec<Approval>,
    /// The last applied `seq` (0 if none).
    pub seq: u64,
    /// Time of the last applied event.
    pub updated: Option<Timestamp>,
}

impl Fold {
    /// An empty fold for a new thread.
    pub fn new(thread: ThreadId) -> Self {
        Fold {
            thread,
            status: ThreadStatus::Idle,
            turns: Vec::new(),
            approvals: Vec::new(),
            seq: 0,
            updated: None,
        }
    }

    /// Approvals still waiting for an answer.
    pub fn pending(&self) -> impl Iterator<Item = &Approval> {
        self.approvals
            .iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
    }

    /// Applies one event.
    pub fn apply(&mut self, seq: u64, time: &Timestamp, event: &ThreadEvent) {
        match event {
            ThreadEvent::TurnStarted(e) => self.turns.push(Turn {
                id: e.turn.clone(),
                thread: self.thread.clone(),
                started: time.clone(),
                ended: None,
                outcome: None,
            }),
            ThreadEvent::TurnEnded(e) => {
                if let Some(t) = self.turns.iter_mut().find(|t| t.id == e.turn) {
                    t.ended = Some(time.clone());
                    t.outcome = Some(e.outcome);
                }
            }
            ThreadEvent::ApprovalRequested(e) => self.approvals.push(e.approval.clone()),
            ThreadEvent::ApprovalResolved(e) => {
                if let Some(a) = self.approvals.iter_mut().find(|a| a.id == e.approval) {
                    a.status = e.decision.status();
                }
            }
            _ => {}
        }
        let pending = self.pending().count();
        self.status = next_status(self.status, event, pending);
        self.seq = seq;
        self.updated = Some(time.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kumikit_protocol::event::{
        ApprovalRequested, ApprovalResolved, TurnEnded, TurnStarted, TurnText,
    };
    use kumikit_protocol::model::{Resolution, UserInput};

    fn ts(n: u32) -> Timestamp {
        Timestamp(format!("t{n}"))
    }

    #[test]
    fn folds_a_turn_with_an_approval() {
        let th = ThreadId("th".into());
        let mut f = Fold::new(th.clone());
        let approval = Approval {
            id: "a".into(),
            thread: th.clone(),
            turn: "u".into(),
            kind: "tool".into(),
            summary: "s".into(),
            detail: None,
            status: ApprovalStatus::Pending,
        };
        let events = [
            ThreadEvent::TurnStarted(TurnStarted {
                turn: "u".into(),
                input: UserInput { text: "hi".into() },
            }),
            ThreadEvent::TextDelta(TurnText {
                turn: "u".into(),
                text: "x".into(),
            }),
            ThreadEvent::ApprovalRequested(ApprovalRequested { approval }),
        ];
        for (i, e) in events.iter().enumerate() {
            f.apply(i as u64 + 1, &ts(i as u32), e);
        }
        assert_eq!(f.status, ThreadStatus::AwaitingApproval);
        assert_eq!(f.pending().count(), 1);

        f.apply(
            4,
            &ts(4),
            &ThreadEvent::ApprovalResolved(ApprovalResolved {
                approval: "a".into(),
                decision: Resolution::Allow,
                by: None,
            }),
        );
        assert_eq!(f.status, ThreadStatus::Running);
        assert_eq!(f.approvals[0].status, ApprovalStatus::Allowed);

        f.apply(
            5,
            &ts(5),
            &ThreadEvent::TurnEnded(TurnEnded {
                turn: "u".into(),
                outcome: TurnOutcome::Done,
                error: None,
            }),
        );
        assert_eq!(f.status, ThreadStatus::Idle);
        assert_eq!(f.turns[0].outcome, Some(TurnOutcome::Done));
        assert_eq!(f.turns[0].ended, Some(ts(5)));
        assert_eq!(f.seq, 5);
    }

    #[test]
    fn failed_turn_fails_the_thread() {
        let s = next_status(
            ThreadStatus::Running,
            &ThreadEvent::TurnEnded(TurnEnded {
                turn: "u".into(),
                outcome: TurnOutcome::Failed,
                error: None,
            }),
            0,
        );
        assert_eq!(s, ThreadStatus::Failed);
    }
}
