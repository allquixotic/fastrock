//! Durable schedule decisions. Runtime dispatch never precedes a saved claim.
use super::super::{JSONRPCErrorError, error};
use chrono::{DateTime, Datelike, Days, Local, LocalResult, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

type Result<T> = std::result::Result<T, JSONRPCErrorError>;
const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Pattern {
    Webhook {
        hook: super::webhook_model::Hook,
    },
    Once {
        at: i64,
    },
    Interval {
        minutes: u32,
    },
    Daily {
        hour: u32,
        minute: u32,
        weekdays: Vec<u32>,
    },
}
impl Pattern {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Webhook { hook } => hook.validate().map_err(|e| error(e.to_string())),
            Self::Once { at } if DateTime::from_timestamp(*at, 0).is_some() => Ok(()),
            Self::Interval { minutes } if (1..=525600).contains(minutes) => Ok(()),
            Self::Daily {
                hour,
                minute,
                weekdays,
            } if *hour < 24
                && *minute < 60
                && !weekdays.is_empty()
                && weekdays.len() <= 7
                && weekdays.iter().all(|d| *d <= 6) =>
            {
                Ok(())
            }
            _ => Err(error(
                "Choose a valid time, weekdays, or interval from 1 minute to 1 year",
            )),
        }
    }
    pub fn next_after(&self, now: i64) -> Option<i64> {
        match self {
            Self::Webhook { .. } => None,
            Self::Once { at } => (*at > now).then_some(*at),
            Self::Interval { minutes } => now.checked_add(i64::from(*minutes) * 60),
            Self::Daily {
                hour,
                minute,
                weekdays,
            } => next_daily(&Local, now, *hour, *minute, weekdays),
        }
    }
}
fn first_instant<Tz: TimeZone>(value: LocalResult<DateTime<Tz>>) -> Option<i64> {
    match value {
        LocalResult::Single(at) => Some(at.timestamp()),
        LocalResult::Ambiguous(a, b) => Some(a.timestamp().min(b.timestamp())),
        LocalResult::None => None,
    }
}
fn next_daily<Tz: TimeZone>(
    zone: &Tz,
    now: i64,
    hour: u32,
    minute: u32,
    weekdays: &[u32],
) -> Option<i64> {
    let date = zone.timestamp_opt(now, 0).single()?.date_naive();
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    // A weekly occurrence can fall in a DST gap. Look through the following
    // week too, so skipping that missing time does not disable the schedule.
    for days in 0..=14 {
        let date = date.checked_add_days(Days::new(days))?;
        if weekdays.contains(&date.weekday().num_days_from_sunday()) {
            // A repeated local time runs at its first instant only. A missing
            // local time is skipped, not shifted into another occurrence.
            if let Some(at) = first_instant(zone.from_local_datetime(&date.and_time(time))) {
                if at > now {
                    return Some(at);
                }
            }
        }
    }
    None
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Run {
    pub id: String,
    pub started_at: i64,
    pub child: Option<String>,
    pub status: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub id: String,
    pub revision: u64,
    pub title: String,
    pub prompt: String,
    pub source: String,
    pub pattern: Pattern,
    pub paused: bool,
    pub next_at: Option<i64>,
    pub active: Option<Run>,
    pub last: Option<Run>,
    #[serde(default)]
    pub deliveries: Vec<super::webhook_model::Delivery>,
}
impl Task {
    pub fn stop_target(&self, run_id: &str) -> Result<&str> {
        self.active
            .as_ref()
            .filter(|r| r.id == run_id)
            .and_then(|r| r.child.as_deref())
            .ok_or_else(|| {
                error("That occurrence is no longer running; refresh before stopping another run")
            })
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 80
            || self.title.trim().is_empty()
            || self.title.len() > 256
            || self.prompt.trim().is_empty()
            || self.prompt.len() > 32768
            || self.source.is_empty()
            || self.source.len() > 256
        {
            return Err(error(
                "A schedule needs a title (1–256 bytes), task (1–32768 bytes), and source conversation",
            ));
        }
        self.pattern.validate()?;
        if let Pattern::Webhook { .. } = &self.pattern {
            super::webhook_model::template_parts(&self.prompt).map_err(|e| error(e.to_string()))?;
        }
        if self.deliveries.len() > 4096
            || self.deliveries.iter().any(|d| {
                uuid::Uuid::parse_str(&d.id).is_err()
                    || d.prompt.len() > 32768
                    || d.replay.len() != 64
                    || d.coalesce.as_ref().is_some_and(|k| k.len() != 64)
                    || !matches!(
                        d.status.as_str(),
                        "pending"
                            | "starting"
                            | "completed"
                            | "failed"
                            | "interrupted"
                            | "needs_review"
                            | "expired"
                            | "discarded"
                            | "filtered"
                            | "coalesced"
                    )
            })
        {
            return Err(error("Invalid webhook receipts; original file retained"));
        }
        Ok(())
    }
    pub fn claim(&mut self, now: i64, manual: bool) -> Result<Run> {
        if self.active.is_some() {
            return Err(error("This schedule already has an active run"));
        }
        if !manual && (self.paused || self.next_at.is_none_or(|at| at > now)) {
            return Err(error("This schedule is not due"));
        }
        let run = Run {
            id: uuid::Uuid::new_v4().simple().to_string(),
            started_at: now,
            child: None,
            status: "starting".into(),
            detail: String::new(),
        };
        self.active = Some(run.clone());
        self.next_at = self.pattern.next_after(now);
        self.revision += 1;
        Ok(run)
    }
    pub fn finish(&mut self, status: &str, detail: String, now: i64) {
        if let Some(mut run) = self.active.take() {
            for delivery in self
                .deliveries
                .iter_mut()
                .filter(|d| d.run.as_deref() == Some(&run.id))
            {
                delivery.status = if matches!(status, "paused" | "idle") {
                    "needs_review"
                } else {
                    status
                }
                .into();
                delivery.prompt.clear();
            }
            run.status = status.into();
            run.detail = detail;
            self.last = Some(run);
            if status != "completed" {
                self.paused = true;
            }
            if self.next_at.is_some_and(|at| at <= now) {
                self.next_at = self.pattern.next_after(now);
            }
            self.revision += 1;
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Data {
    pub version: u32,
    pub tasks: BTreeMap<String, Task>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: BTreeMap::new(),
        }
    }
}
impl Data {
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(error("Unknown schedule format; original file retained"));
        }
        for (id, task) in &self.tasks {
            if id != &task.id {
                return Err(error("Invalid schedule identity; original file retained"));
            }
            task.validate()?;
        }
        if serde_json::to_vec(self)
            .map_err(|_| error("Cannot encode schedules"))?
            .len() as u64
            > MAX_BYTES
        {
            return Err(error(
                "Schedule capacity reached; discard pending work or remove old schedules before adding more",
            ));
        }
        Ok(())
    }
    pub fn recover(&mut self) {
        for task in self.tasks.values_mut() {
            task.paused = true;
            task.finish("needs_review", "Fastrock stopped during this occurrence. Inspect its conversation and workspace; it will not be replayed.".into(), Utc::now().timestamp());
            task.revision += 1;
        }
    }
}
pub(crate) struct State {
    pub path: PathBuf,
    pub data: Data,
    pub error: Option<String>,
    pub stopping: bool,
}
impl State {
    pub fn open(path: PathBuf) -> Self {
        let loaded = (|| -> Result<Data> {
            match std::fs::metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Data::default()),
                Err(_) => return Err(error("Cannot read schedules; original file retained")),
                Ok(meta) if meta.len() > MAX_BYTES => {
                    return Err(error(
                        "Schedule file exceeds supported size; original file retained",
                    ));
                }
                Ok(_) => {}
            }
            let bytes = std::fs::read(&path)
                .map_err(|_| error("Cannot read schedules; original file retained"))?;
            let mut data: Data = serde_json::from_slice(&bytes)
                .map_err(|_| error("Invalid schedules; original file retained"))?;
            data.validate()?;
            data.recover();
            write(&path, &data)?;
            Ok(data)
        })();
        match loaded {
            Ok(data) => Self {
                path,
                data,
                error: None,
                stopping: false,
            },
            Err(e) => Self {
                path,
                data: Data::default(),
                error: Some(e.message),
                stopping: true,
            },
        }
    }
    pub fn available(&self) -> Result<()> {
        if let Some(message) = &self.error {
            return Err(error(message));
        }
        if self.stopping {
            return Err(error("Scheduler is shutting down"));
        }
        Ok(())
    }
    pub async fn commit(&mut self, data: Data) -> Result<()> {
        data.validate()?;
        let path = self.path.clone();
        let saved = data.clone();
        let result = tokio::task::spawn_blocking(move || write(&path, &saved))
            .await
            .unwrap_or_else(|_| Err(error("Schedule storage worker failed")));
        if let Err(e) = result {
            self.error = Some(e.message.clone());
            self.stopping = true;
            return Err(e);
        }
        self.data = data;
        Ok(())
    }
}
fn write(path: &std::path::Path, data: &Data) -> Result<()> {
    let bytes = serde_json::to_vec(data).map_err(|_| error("Cannot encode schedules"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("Schedule state exceeds supported size"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| error("Cannot create schedule storage"))?;
    }
    crate::rally::store::atomic_write(path, &bytes).map_err(|_| error("Cannot save schedules; automatic dispatch stopped. Preserve the file and restart after correcting storage."))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v58_capacity_rejection_preserves_dispatch_and_saved_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let mut state = State::open(path.clone());
        let mut saved = Data::default();
        saved.tasks.insert("task".into(), task());
        state.commit(saved.clone()).await.unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut large = saved;
        for i in 0..520 {
            let mut t = task();
            t.id = format!("task-{i}");
            t.prompt = "x".repeat(32768);
            large.tasks.insert(t.id.clone(), t);
        }
        assert!(state.commit(large).await.is_err());
        assert!(state.available().is_ok());
        assert_eq!(state.data.tasks.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn v58_receipt_recovery_retains_pending_and_marks_uncertain() {
        use super::super::webhook_model::{Hook, accept};
        let mut task = task();
        let hook = Hook::default();
        task.pattern = Pattern::Webhook { hook: hook.clone() };
        task.next_at = None;
        accept(
            &mut task.deliveries,
            &hook,
            &serde_json::json!({}),
            b"a",
            "First",
            100,
        )
        .unwrap();
        accept(
            &mut task.deliveries,
            &hook,
            &serde_json::json!({}),
            b"b",
            "Second",
            101,
        )
        .unwrap();
        let run = task.claim(102, true).unwrap();
        task.deliveries[0].status = "starting".into();
        task.deliveries[0].run = Some(run.id);
        let mut data = Data::default();
        data.tasks.insert(task.id.clone(), task);
        data.recover();
        data.validate().unwrap();
        let task = data.tasks.get_mut("task").unwrap();
        assert!(task.paused && task.active.is_none());
        assert_eq!(task.deliveries[0].status, "needs_review");
        assert_eq!(task.deliveries[1].status, "pending");
        assert_eq!(task.deliveries[1].prompt, "Second");
        let run = task.claim(104, true).unwrap();
        task.deliveries[1].run = Some(run.id);
        task.finish("paused", String::new(), 105);
        task.validate().unwrap();
        assert_eq!(task.deliveries[1].status, "needs_review");
    }
    fn task() -> Task {
        Task {
            id: "task".into(),
            revision: 1,
            title: "Task".into(),
            prompt: "Work".into(),
            source: "parent".into(),
            pattern: Pattern::Interval { minutes: 5 },
            paused: false,
            next_at: Some(100),
            active: None,
            last: None,
            deliveries: Vec::new(),
        }
    }
    #[test]
    fn v56_dates_skip_missed_and_repeated_local_times() {
        let zone = chrono::FixedOffset::east_opt(0).unwrap();
        let monday = zone
            .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(
            next_daily(&zone, monday, 9, 0, &[1]),
            Some(monday + 7 * 86400 - 3600)
        );
        assert_eq!(
            next_daily(&zone, monday, 11, 0, &[1, 2, 3, 4, 5]),
            Some(monday + 3600)
        );
        let early = Utc.timestamp_opt(100, 0).unwrap();
        let late = Utc.timestamp_opt(3700, 0).unwrap();
        assert_eq!(
            first_instant(LocalResult::Ambiguous(late, early)),
            Some(100)
        );
        assert_eq!(first_instant::<Utc>(LocalResult::None), None);
        #[derive(Clone)]
        struct MissingSunday;
        impl TimeZone for MissingSunday {
            type Offset = chrono::FixedOffset;
            fn from_offset(_: &Self::Offset) -> Self {
                Self
            }
            fn offset_from_local_date(
                &self,
                date: &chrono::NaiveDate,
            ) -> LocalResult<Self::Offset> {
                if date.to_string() == "2026-03-08" {
                    LocalResult::None
                } else {
                    LocalResult::Single(chrono::FixedOffset::east_opt(0).unwrap())
                }
            }
            fn offset_from_local_datetime(
                &self,
                date: &chrono::NaiveDateTime,
            ) -> LocalResult<Self::Offset> {
                self.offset_from_local_date(&date.date())
            }
            fn offset_from_utc_date(&self, _: &chrono::NaiveDate) -> Self::Offset {
                chrono::FixedOffset::east_opt(0).unwrap()
            }
            fn offset_from_utc_datetime(&self, _: &chrono::NaiveDateTime) -> Self::Offset {
                chrono::FixedOffset::east_opt(0).unwrap()
            }
        }
        assert_eq!(
            next_daily(
                &MissingSunday,
                zone.with_ymd_and_hms(2026, 3, 2, 10, 0, 0)
                    .unwrap()
                    .timestamp(),
                2,
                30,
                &[0]
            ),
            Some(
                zone.with_ymd_and_hms(2026, 3, 15, 2, 30, 0)
                    .unwrap()
                    .timestamp()
            )
        );
        assert_eq!(
            Pattern::Interval { minutes: 5 }.next_after(1000),
            Some(1300)
        );
        assert_eq!(Pattern::Once { at: 1000 }.next_after(1000), None);
        assert!(
            Pattern::Daily {
                hour: 25,
                minute: 0,
                weekdays: vec![1]
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn v56_stop_identity_survives_metadata_changes_but_not_a_new_run() {
        let mut task = task();
        let run = task.claim(1000, true).unwrap();
        task.active.as_mut().unwrap().child = Some("child".into());
        task.revision += 1;
        assert_eq!(task.stop_target(&run.id).unwrap(), "child");
        task.finish("completed", String::new(), 1001);
        task.claim(1002, true).unwrap();
        task.active.as_mut().unwrap().child = Some("other-child".into());
        assert!(task.stop_target(&run.id).is_err());
    }
    #[test]
    fn v56_claim_recovery_never_replays_or_overlaps() {
        let mut task = task();
        let first = task.claim(1000, false).unwrap();
        assert_eq!(task.next_at, Some(1300));
        assert!(task.claim(1001, true).is_err());
        task.active.as_mut().unwrap().child = Some("child".into());
        let mut data = Data::default();
        data.tasks.insert(task.id.clone(), task);
        data.recover();
        let task = data.tasks.get_mut("task").unwrap();
        assert!(task.paused && task.active.is_none());
        assert_eq!(task.last.as_ref().unwrap().id, first.id);
        assert_eq!(task.last.as_ref().unwrap().child.as_deref(), Some("child"));
        assert_eq!(task.last.as_ref().unwrap().status, "needs_review");
        assert!(task.claim(5000, false).is_err());
        assert_ne!(task.claim(5000, true).unwrap().id, first.id);
        task.finish("completed", String::new(), 9000);
        assert_eq!(task.next_at, Some(9300));
    }
    #[tokio::test]
    async fn v56_corrupt_and_unwritable_storage_preserves_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        std::fs::write(&path, "broken").unwrap();
        let state = State::open(path.clone());
        assert!(state.available().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
        let future = r#"{"version":1,"tasks":{},"new_field":"preserve"}"#;
        std::fs::write(&path, future).unwrap();
        assert!(State::open(path.clone()).available().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
        std::fs::remove_file(&path).unwrap();
        let mut state = State::open(path.clone());
        let mut data = Data::default();
        data.tasks.insert("task".into(), task());
        state.commit(data.clone()).await.unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        data.tasks
            .get_mut("task")
            .unwrap()
            .claim(1000, false)
            .unwrap();
        assert!(state.commit(data).await.is_err());
        assert!(state.available().is_err());
        assert!(state.data.tasks["task"].active.is_none());
    }
}
