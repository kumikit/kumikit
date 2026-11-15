//! Process trees on Linux and macOS: each child leads its own process group.

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process::Command;

/// The process group of one supervised child.
#[derive(Debug)]
pub(super) struct Tree {
    pgid: Option<Pid>,
}

impl Tree {
    /// Makes the child the leader of a new process group.
    pub(super) fn prepare(cmd: &mut Command) {
        cmd.process_group(0);
    }

    pub(super) fn attach(child: &tokio::process::Child) -> std::io::Result<Tree> {
        let pgid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw);
        Ok(Tree { pgid })
    }

    /// Asks the group to stop (SIGTERM).
    pub(super) fn stop(&self) {
        self.signal(Signal::TERM);
    }

    /// Kills the group (SIGKILL). The group id stays reserved while any member lives,
    /// so this cannot hit an unrelated process.
    pub(super) fn kill(&self) {
        self.signal(Signal::KILL);
    }

    fn signal(&self, signal: Signal) {
        if let Some(pgid) = self.pgid {
            // ESRCH: the group is already gone.
            let _ = kill_process_group(pgid, signal);
        }
    }
}
