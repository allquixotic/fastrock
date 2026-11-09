//! Bounded, process-local timing facts; no prompts, IDs, URLs or credentials.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) enum Phase {
    NewSession,
    LoadSession,
    Configure,
    TurnSetup,
    FirstResponse,
    ThreadSetup,
    ProviderSelection,
    WorkspaceSelection,
    ToolSetup,
    RestoreGate,
    RestoreHistory,
    RestoreSettings,
    RestoreWorkspace,
    RestoreTools,
    RestoreQueue,
    WorkspacePersistence,
}
const LABELS: [&str; 16] = [
    "Session creation",
    "Session loading",
    "Model configuration",
    "Turn setup",
    "Send to first response",
    "Complete thread setup",
    "Provider selection",
    "Workspace selection",
    "Workspace tools and hooks",
    "Waiting for an earlier thread operation",
    "Reading saved conversation",
    "Restoring saved model and tool settings",
    "Resolving saved workspace",
    "Registering saved workspace tools",
    "Reading saved message queue",
    "Saving new workspace recovery",
];
#[derive(Default)]
struct Measurement {
    last: AtomicU64,
    peak: AtomicU64,
    active: AtomicU64,
}
impl Measurement {
    fn record(&self, milliseconds: u64) {
        // Zero denotes no observation; preserve real sub-millisecond samples.
        let value = milliseconds.saturating_add(1);
        self.peak.fetch_max(value, Ordering::Relaxed);
        self.last.store(value, Ordering::Relaxed);
    }
    fn display(&self) -> Option<String> {
        let last = self.last.load(Ordering::Relaxed).checked_sub(1)?;
        let peak = self.peak.load(Ordering::Relaxed).saturating_sub(1);
        Some(format!("last {last} ms; peak {peak} ms this run"))
    }
}
static MEASUREMENTS: [Measurement; 16] = [const {
    Measurement {
        last: AtomicU64::new(0),
        peak: AtomicU64::new(0),
        active: AtomicU64::new(0),
    }
}; 16];

pub(super) struct Timer(Phase, Instant);
impl Timer {
    pub fn start(phase: Phase) -> Self {
        MEASUREMENTS[phase as usize]
            .active
            .fetch_add(1, Ordering::Relaxed);
        Self(phase, Instant::now())
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        record(self.0, self.1);
        MEASUREMENTS[self.0 as usize]
            .active
            .fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) async fn step<T>(phase: Phase, future: impl std::future::Future<Output = T>) -> T {
    let _timer = Timer::start(phase);
    future.await
}

pub(crate) fn active() -> String {
    LABELS
        .iter()
        .zip(&MEASUREMENTS)
        .filter(|(_, value)| value.active.load(Ordering::Relaxed) > 0)
        .map(|(label, _)| *label)
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn record(phase: Phase, start: Instant) {
    let milliseconds = start.elapsed().as_millis().min(u64::MAX as u128 - 1) as u64;
    MEASUREMENTS[phase as usize].record(milliseconds);
    tracing::info!(
        phase = LABELS[phase as usize],
        elapsed_ms = milliseconds,
        "Response latency"
    );
}

pub(crate) fn facts() -> Vec<(&'static str, String)> {
    LABELS
        .iter()
        .zip(&MEASUREMENTS)
        .filter_map(|(label, value)| {
            let count = value.active.load(Ordering::Relaxed);
            let finished = value.display();
            if count > 0 {
                Some((
                    *label,
                    format!(
                        "{count} in progress; {}",
                        finished.unwrap_or_else(|| "no completed sample".into())
                    ),
                ))
            } else {
                finished.map(|value| (*label, value))
            }
        })
        .chain(std::iter::once((
            "Workspace store initialization",
            fastrock_workspaces::embedded::diagnostic(),
        )))
        .chain(fastrock_workspaces::embedded::operation_diagnostics())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v82_timing_facts_keep_slow_peak_after_later_fast_response() {
        let sample = Measurement::default();
        assert!(sample.display().is_none());
        sample.record(180_000);
        sample.record(0);
        assert_eq!(
            sample.display().unwrap(),
            "last 0 ms; peak 180000 ms this run"
        );
    }
}
