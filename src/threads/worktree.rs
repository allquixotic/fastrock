//! The former physical-worktree action now forks the bound virtual workspace.
use crate::app::{AppController, ThreadPhase};
use crate::transcript::NoticeKind;

impl AppController {
    pub(crate) fn worktree_continue(&mut self, index: usize) {
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        if thread.extras.side_parent.is_some()
            || thread.is_busy()
            || thread.phase == ThreadPhase::Starting
        {
            self.transcript_push_notice(index, NoticeKind::Warning,
                "Finish the current turn in a persistent conversation before forking its workspace.".into());
            return;
        }
        self.fork_tab(index, None);
    }
}
