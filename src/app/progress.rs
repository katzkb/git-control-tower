use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpStep {
    RunningWtRemove,
    RunningWtForceRemove,
    RunningBranchDelete,
    Done { success: bool },
}

#[derive(Debug, Clone)]
pub struct OpProgress {
    pub label: String,
    pub current_step: OpStep,
    pub op_started_at: Instant,
    /// Set when the op reaches `Done`; freezes the elapsed-time display.
    pub finished_at: Option<Instant>,
    pub last_command: Option<String>,
    pub error: Option<String>,
}

impl OpProgress {
    pub fn new(label: String) -> Self {
        Self {
            label,
            current_step: OpStep::RunningWtRemove, // overwritten by first OpStepBegin
            op_started_at: Instant::now(),
            finished_at: None,
            last_command: None,
            error: None,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self.current_step, OpStep::Done { .. })
    }
}

#[derive(Debug, Default)]
pub struct ProgressTracker {
    pub ops: BTreeMap<u64, OpProgress>,
    next_id: u64,
    pub started_at: Option<Instant>,
    /// `q`/`Esc` was pressed while ops were active; the next press quits.
    /// Reset explicitly by the main loop when the op batch finishes —
    /// deliberately not folded into `clear()`/`sweep_unfinished()` (issue #220).
    pub quit_pressed: bool,
}

impl ProgressTracker {
    pub fn is_active(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn total(&self) -> usize {
        self.ops.len()
    }

    pub fn done_count(&self) -> usize {
        self.ops.values().filter(|p| p.is_done()).count()
    }

    pub fn allocate_ids(&mut self, n: usize) -> std::ops::Range<u64> {
        let start = self.next_id;
        self.next_id += n as u64;
        if self.started_at.is_none() && n > 0 {
            self.started_at = Some(Instant::now());
        }
        start..self.next_id
    }

    pub fn insert(&mut self, op_id: u64, op: OpProgress) {
        self.ops.insert(op_id, op);
    }

    pub fn update_step(&mut self, op_id: u64, step: OpStep, command: String) {
        if let Some(op) = self.ops.get_mut(&op_id) {
            op.current_step = step;
            op.last_command = Some(command);
        }
    }

    pub fn finish(&mut self, op_id: u64, success: bool, error: Option<String>) {
        if let Some(op) = self.ops.get_mut(&op_id) {
            op.current_step = OpStep::Done { success };
            op.error = error;
            if op.finished_at.is_none() {
                op.finished_at = Some(Instant::now());
            }
        }
    }

    /// Force-finish any non-Done ops as failures. Used when OpAllDone arrives
    /// but some tasks panicked and never sent OpFinished.
    pub fn sweep_unfinished(&mut self) {
        for op in self.ops.values_mut() {
            if !op.is_done() {
                op.current_step = OpStep::Done { success: false };
                op.finished_at = Some(Instant::now());
                if op.error.is_none() {
                    op.error = Some("interrupted".to_string());
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.ops.clear();
        self.started_at = None;
    }
}

#[cfg(test)]
mod progress_tracker_tests {
    use super::*;

    #[test]
    fn progress_tracker_allocate_ids_advances() {
        let mut t = ProgressTracker::default();
        let r = t.allocate_ids(3);
        assert_eq!(r, 0..3);
        let r2 = t.allocate_ids(2);
        assert_eq!(r2, 3..5);
    }

    #[test]
    fn progress_tracker_allocate_ids_sets_started_at_once() {
        let mut t = ProgressTracker::default();
        assert!(t.started_at.is_none());
        let _ = t.allocate_ids(2);
        let first = t
            .started_at
            .expect("started_at set on first non-empty allocation");
        let _ = t.allocate_ids(1);
        assert_eq!(t.started_at, Some(first));
    }

    #[test]
    fn progress_tracker_state_transitions() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(2).collect();
        t.insert(ids[0], OpProgress::new("a".into()));
        t.insert(ids[1], OpProgress::new("b".into()));

        assert_eq!(t.total(), 2);
        assert_eq!(t.done_count(), 0);
        assert!(t.is_active());

        t.update_step(
            ids[0],
            OpStep::RunningWtForceRemove,
            "git worktree remove --force /wt/a".into(),
        );
        assert_eq!(t.ops[&ids[0]].current_step, OpStep::RunningWtForceRemove);
        assert_eq!(
            t.ops[&ids[0]].last_command.as_deref(),
            Some("git worktree remove --force /wt/a")
        );

        t.finish(ids[0], true, None);
        assert!(t.ops[&ids[0]].is_done());
        assert_eq!(t.done_count(), 1);

        t.finish(ids[1], false, Some("nope".into()));
        assert_eq!(t.done_count(), 2);
        assert_eq!(t.ops[&ids[1]].error.as_deref(), Some("nope"));
    }

    #[test]
    fn progress_tracker_sweep_unfinished_marks_remaining_as_failed() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(2).collect();
        t.insert(ids[0], OpProgress::new("a".into()));
        t.insert(ids[1], OpProgress::new("b".into()));
        t.finish(ids[0], true, None);

        t.sweep_unfinished();
        assert!(t.ops[&ids[1]].is_done());
        assert_eq!(t.ops[&ids[1]].current_step, OpStep::Done { success: false });
        assert_eq!(t.ops[&ids[1]].error.as_deref(), Some("interrupted"));
    }

    #[test]
    fn progress_tracker_clear_resets_state() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(1).collect();
        t.insert(ids[0], OpProgress::new("a".into()));
        t.clear();
        assert!(!t.is_active());
        assert!(t.started_at.is_none());
        assert_eq!(t.total(), 0);
    }
}
