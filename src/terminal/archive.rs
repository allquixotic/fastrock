//! Keep a closed shell's bounded scrollback available without retaining its PTY.
use super::screen::{self, Frame};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
};
use tokio::sync::watch;
pub(super) struct Archive {
    screen: Mutex<Option<vt100::Screen>>,
    pending: AtomicI32,
    size: AtomicU32,
    scheduled: AtomicBool,
    runtime: tokio::runtime::Handle,
    frames: watch::Sender<Frame>,
}
impl Archive {
    pub fn new(frames: watch::Sender<Frame>) -> Arc<Self> {
        Arc::new(Self {
            screen: Mutex::new(None),
            pending: AtomicI32::new(0),
            size: AtomicU32::new(0),
            scheduled: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
            frames,
        })
    }
    pub fn capture(&self, screen: &vt100::Screen) {
        *self.screen.lock().unwrap() = Some(screen.clone());
    }
    pub fn scroll(self: &Arc<Self>, rows: i32) {
        let _ = self
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                Some(
                    v.saturating_add(rows)
                        .clamp(-(screen::HISTORY as i32), screen::HISTORY as i32),
                )
            });
        self.schedule();
    }
    pub fn resize(self: &Arc<Self>, rows: u16, cols: u16) {
        let (rows, cols) = screen::size(rows, cols);
        self.size.store(
            (u32::from(rows.get()) << 16) | u32::from(cols.get()),
            Ordering::Release,
        );
        self.schedule();
    }
    fn schedule(self: &Arc<Self>) {
        if self.scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let archive = self.clone();
        self.runtime.spawn_blocking(move || {
            loop {
                let rows = archive.pending.swap(0, Ordering::AcqRel);
                let size = archive.size.swap(0, Ordering::AcqRel);
                if rows != 0 || size != 0 {
                    let mut stored = archive.screen.lock().unwrap();
                    if let Some(screen) = stored.as_mut() {
                        if size != 0 {
                            let (r, c) = screen::size((size >> 16) as u16, size as u16);
                            screen.set_size(r, c);
                        }
                        screen.set_scrollback(
                            screen.scrollback().saturating_add_signed(rows as isize),
                        );
                        let previous = archive.frames.borrow().clone();
                        archive
                            .frames
                            .send_replace(screen::snapshot(screen, &previous));
                    }
                }
                archive.scheduled.store(false, Ordering::Release);
                if (archive.pending.load(Ordering::Acquire) == 0
                    && archive.size.load(Ordering::Acquire) == 0)
                    || archive.scheduled.swap(true, Ordering::AcqRel)
                {
                    break;
                }
            }
        });
    }
}
impl Drop for Archive {
    fn drop(&mut self) {
        if let Some(screen) = self.screen.get_mut().unwrap().take() {
            self.runtime.spawn_blocking(move || drop(screen));
        }
    }
}
