//! Bounded timing categories, never repository paths, arguments or tool results.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Create,
    Log,
    Temporary,
    Transfer,
    Task,
    Other,
    Native,
}
const LABELS: [&str; 7] = [
    "Workspace creation",
    "Workspace Git log",
    "Workspace temporary checkout",
    "Workspace repository transfer",
    "Workspace build or package task",
    "Other workspace commands",
    "Native workspace operations",
];
impl Operation {
    pub fn classify(op: &str) -> Self {
        match op {
            "workspace_create" | "workspace_fork" => Self::Create,
            "repo_log" => Self::Log,
            "workspace_temporary"
            | "workspace_export"
            | "workspace_import"
            | "temporary_release" => Self::Temporary,
            "repo_fetch" | "repo_register" | "repo_clone" => Self::Transfer,
            "workspace_run_task" | "package_run" => Self::Task,
            _ => Self::Other,
        }
    }
    pub fn bulk(self) -> bool {
        matches!(self, Self::Temporary | Self::Transfer | Self::Task)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Store,
    Admission,
    Worker,
    Execution,
}
const PHASES: [&str; 4] = ["store wait", "admission wait", "worker wait", "execution"];
struct Sample {
    active: AtomicU64,
    last: AtomicU64,
    peak: AtomicU64,
    count: AtomicU64,
}
impl Sample {
    const fn new() -> Self {
        Self {
            active: AtomicU64::new(0),
            last: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }
}
static SAMPLES: [[Sample; 4]; 7] = [const { [const { Sample::new() }; 4] }; 7];

pub(super) struct Timer(&'static Sample, Instant);
impl Timer {
    pub fn start(operation: Operation, phase: Phase) -> Self {
        let sample = &SAMPLES[operation as usize][phase as usize];
        sample.active.fetch_add(1, Ordering::Relaxed);
        Self(sample, Instant::now())
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        let millis = self.1.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.0.last.store(millis, Ordering::Relaxed);
        self.0.peak.fetch_max(millis, Ordering::Relaxed);
        self.0.count.fetch_add(1, Ordering::Relaxed);
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn diagnostics() -> Vec<(&'static str, String)> {
    LABELS
        .into_iter()
        .zip(&SAMPLES)
        .filter_map(|(label, phases)| {
            let facts: Vec<_> = PHASES
                .into_iter()
                .zip(phases)
                .filter_map(|(phase, sample)| {
                    let count = sample.count.load(Ordering::Relaxed);
                    let active = sample.active.load(Ordering::Relaxed);
                    (count > 0 || active > 0).then(|| {
                        format!(
                            "{phase}: {active} active, {count} samples, last {} ms, peak {} ms",
                            sample.last.load(Ordering::Relaxed),
                            sample.peak.load(Ordering::Relaxed),
                        )
                    })
                })
                .collect();
            (!facts.is_empty()).then(|| (label, facts.join("; ")))
        })
        .collect()
}
