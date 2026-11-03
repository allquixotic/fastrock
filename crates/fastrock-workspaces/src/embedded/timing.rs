//! Fixed-size startup facts. Never include home paths or database contents.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[derive(Default)]
struct Timings {
    attempts: AtomicU64,
    clean: AtomicU64,
    recovered: AtomicU64,
    created: AtomicU64,
    last_ms: AtomicU64,
    peak_ms: AtomicU64,
}
static TIMINGS: Timings = Timings {
    attempts: AtomicU64::new(0),
    clean: AtomicU64::new(0),
    recovered: AtomicU64::new(0),
    created: AtomicU64::new(0),
    last_ms: AtomicU64::new(0),
    peak_ms: AtomicU64::new(0),
};
impl Timings {
    fn elapsed(&self, millis: u64) {
        self.last_ms.store(millis, Ordering::Relaxed);
        self.peak_ms.fetch_max(millis, Ordering::Relaxed);
    }
    fn display(&self) -> String {
        let read = |v: &AtomicU64| v.load(Ordering::Relaxed);
        format!(
            "opens {}; clean {}; recovered {}; new {}; last {} ms; peak {} ms this run",
            read(&self.attempts),
            read(&self.clean),
            read(&self.recovered),
            read(&self.created),
            read(&self.last_ms),
            read(&self.peak_ms)
        )
    }
}
pub(super) struct Opening(Instant);
impl Opening {
    pub fn start() -> Self {
        TIMINGS.attempts.fetch_add(1, Ordering::Relaxed);
        Self(Instant::now())
    }
}
impl Drop for Opening {
    fn drop(&mut self) {
        TIMINGS.elapsed(self.0.elapsed().as_millis().min(u64::MAX as u128) as u64);
    }
}
pub(super) fn opened(report: &serde_json::Value) {
    let counter = match report["state"].as_str() {
        Some("clean") => &TIMINGS.clean,
        Some("recovered") => &TIMINGS.recovered,
        _ => &TIMINGS.created,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}
pub fn diagnostic() -> String {
    TIMINGS.display()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v87_open_timings_retain_peak_without_paths() {
        let timings = Timings::default();
        timings.elapsed(90_000);
        timings.elapsed(3);
        assert_eq!(
            timings.display(),
            "opens 0; clean 0; recovered 0; new 0; last 3 ms; peak 90000 ms this run"
        );
    }
}
