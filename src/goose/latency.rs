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
}
const LABELS: [&str; 5] = [
    "Session creation",
    "Session loading",
    "Model configuration",
    "Turn setup",
    "Send to first response",
];
#[derive(Default)]
struct Measurement {
    last: AtomicU64,
    peak: AtomicU64,
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
static MEASUREMENTS: [Measurement; 5] = [const {
    Measurement {
        last: AtomicU64::new(0),
        peak: AtomicU64::new(0),
    }
}; 5];

pub(super) fn record(phase: Phase, start: Instant) {
    let milliseconds = start.elapsed().as_millis().min(u64::MAX as u128 - 1) as u64;
    MEASUREMENTS[phase as usize].record(milliseconds);
    tracing::info!(
        phase = LABELS[phase as usize],
        elapsed_ms = milliseconds,
        "Goose latency"
    );
}

pub(crate) fn facts() -> Vec<(&'static str, String)> {
    LABELS
        .iter()
        .zip(&MEASUREMENTS)
        .filter_map(|(label, value)| value.display().map(|value| (*label, value)))
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
