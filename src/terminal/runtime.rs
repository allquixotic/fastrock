use super::{
    lease::{ContextInfo, Lease},
    process::Group,
    screen::{self, Frame},
};
use anyhow::{Context, Result, ensure};
use portable_pty::{CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
const INPUT_LIMIT: usize = 1024 * 1024;
const TICK: Duration = Duration::from_millis(16);
enum Event {
    Data(Vec<u8>),
    Input(Vec<u8>, bool),
    Scroll(i32),
    Wake,
    Eof,
}
struct State {
    events: SyncSender<Event>,
    stop: AtomicBool,
    done: AtomicBool,
    pending_input: AtomicUsize,
    requested_size: AtomicU32,
    import: Mutex<Vec<String>>,
    archive: Option<Arc<super::archive::Archive>>,
}
impl State {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.events.try_send(Event::Wake);
    }
}
static OWNERS: OnceLock<Mutex<Vec<Weak<State>>>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static STARTING: AtomicUsize = AtomicUsize::new(0);
struct Starting;
impl Drop for Starting {
    fn drop(&mut self) {
        STARTING.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct Handle {
    state: Arc<State>,
    pub frames: watch::Receiver<Frame>,
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.state.stop();
    }
}
impl Handle {
    pub fn input(&self, text: Vec<u8>, paste: bool) -> Result<()> {
        ensure!(
            !self.state.stop.load(Ordering::Acquire) && !self.state.done.load(Ordering::Acquire),
            "Terminal is closed"
        );
        let count = text.len();
        ensure!(count <= INPUT_LIMIT, "Terminal paste exceeds 1 MiB");
        self.state
            .pending_input
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(count).filter(|v| *v <= INPUT_LIMIT)
            })
            .map_err(|_| anyhow::anyhow!("Terminal input is busy; wait before pasting again"))?;
        if self
            .state
            .events
            .try_send(Event::Input(text, paste))
            .is_err()
        {
            self.state.pending_input.fetch_sub(count, Ordering::AcqRel);
            anyhow::bail!("Terminal input is busy; try again");
        }
        Ok(())
    }
    pub fn resize(&self, rows: u16, cols: u16) {
        if self.frames.borrow().finished {
            if let Some(archive) = &self.state.archive {
                archive.resize(rows, cols);
            }
        } else {
            let (rows, cols) = screen::size(rows, cols);
            self.state.requested_size.store(
                (u32::from(rows.get()) << 16) | u32::from(cols.get()),
                Ordering::Release,
            );
            let _ = self.state.events.try_send(Event::Wake);
        }
    }
    pub fn scroll(&self, rows: i32) {
        if self.frames.borrow().finished {
            if let Some(archive) = &self.state.archive {
                archive.scroll(rows);
            }
        } else {
            let _ = self.state.events.try_send(Event::Scroll(rows));
        }
    }
    pub fn stopping(&self) -> bool {
        self.state.stop.load(Ordering::Acquire)
    }
    pub fn close(&self, paths: Vec<String>) {
        let mut requested = self.state.import.lock().unwrap();
        if !self.stopping() {
            *requested = paths;
            self.state.stop();
        }
    }
}
pub(crate) async fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let owners = OWNERS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for owner in &owners {
            owner.stop();
        }
        if (STARTING.load(Ordering::Acquire) == 0
            && owners.iter().all(|o| o.done.load(Ordering::Acquire)))
            || tokio::time::Instant::now() >= deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
pub(super) async fn open(context: ContextInfo, rows: u16, cols: u16) -> Result<Handle> {
    STARTING.fetch_add(1, Ordering::AcqRel);
    let _starting = Starting;
    ensure!(
        !SHUTTING_DOWN.load(Ordering::Acquire),
        "Fastrock is quitting"
    );
    let lease = Lease::open(context).await?;
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        let _ = lease.finish(Vec::new()).await;
        anyhow::bail!("Fastrock is quitting");
    }
    let initial = Frame {
        cwd: lease.cwd.to_string_lossy().into_owned(),
        virtual_copy: lease.is_virtual(),
        status: "Starting terminal…".into(),
        ..Default::default()
    };
    let (frames, receiver) = watch::channel(initial);
    let (events, rx) = mpsc::sync_channel(64);
    let state = Arc::new(State {
        events,
        stop: AtomicBool::new(false),
        done: AtomicBool::new(false),
        pending_input: AtomicUsize::new(0),
        requested_size: AtomicU32::new(0),
        import: Mutex::new(Vec::new()),
        archive: Some(super::archive::Archive::new(frames.clone())),
    });
    {
        let mut owners = OWNERS.get_or_init(Default::default).lock().unwrap();
        owners.retain(|v| v.strong_count() > 0);
        owners.push(Arc::downgrade(&state));
    }
    let service = state.clone();
    tokio::spawn(async move {
        let worker_state = service.clone();
        let worker_frames = frames.clone();
        let cwd = lease.cwd.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            run(worker_state, rx, worker_frames, cwd, rows, cols)
        });
        let mut failure = None;
        loop {
            tokio::select! {
                result=&mut worker => {if let Err(e)=result {failure=Some(e.to_string());} else if let Ok(Err(e))=result {failure=Some(format!("{e:#}"));} break;},
                _=tokio::time::sleep(lease.renew_after), if !service.stop.load(Ordering::Acquire) => {
                    if let Err(e)=lease.renew().await {failure=Some(format!("Workspace lease renewal failed: {e}"));service.stop();}
                }
            }
        }
        let paths = std::mem::take(&mut *service.import.lock().unwrap());
        let status = lease.finish(paths).await;
        frames.send_modify(|f| {
            f.finished = true;
            f.status = match failure {
                Some(ref e) => format!("{e}. {status}"),
                None => format!("{} · {status}", f.status),
            };
        });
        service.done.store(true, Ordering::Release);
    });
    Ok(Handle {
        state,
        frames: receiver,
    })
}
#[derive(Default)]
struct Replies(Vec<Vec<u8>>);
impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if self.0.len() >= 32 {
            return;
        }
        let param = params.first().and_then(|v| v.first()).copied().unwrap_or(0);
        let reply = match (i1, c, param) {
            (None, 'n', 5) => Some(b"\x1b[0n".to_vec()),
            (None, 'n', 6) => {
                let (r, c) = screen.cursor_position();
                Some(format!("\x1b[{};{}R", r + 1, c + 1).into_bytes())
            }
            (None, 'c', 0) => Some(b"\x1b[?1;2c".to_vec()),
            (Some(b'>'), 'c', 0) => Some(b"\x1b[>0;1;0c".to_vec()),
            _ => None,
        };
        if let Some(reply) = reply {
            self.0.push(reply);
        }
    }
}
fn run(
    state: Arc<State>,
    rx: mpsc::Receiver<Event>,
    frames: watch::Sender<Frame>,
    cwd: std::path::PathBuf,
    rows: u16,
    cols: u16,
) -> Result<()> {
    let (rows, cols) = screen::size(rows, cols);
    let pair = portable_pty::native_pty_system().openpty(PtySize {
        rows: rows.get(),
        cols: cols.get(),
        pixel_width: 0,
        pixel_height: 0,
    })?;
    #[cfg(windows)]
    let mut command = {
        let mut c = CommandBuilder::new(
            std::env::var_os("COMSPEC").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into()),
        );
        c.args(["/D", "/Q"]);
        c
    };
    #[cfg(unix)]
    let mut command =
        CommandBuilder::new(std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()));
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .context("Launch terminal shell")?;
    let group = match Group::new(child.as_ref(), pair.master.as_ref()) {
        Ok(group) => group,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let pid = child.process_id().unwrap_or(0);
    let mut killer = child.clone_killer();
    drop(pair.slave);
    let (input_tx, input_rx) = mpsc::sync_channel::<(Vec<u8>, usize)>(64);
    let input_state = state.clone();
    let input_thread = std::thread::spawn(move || {
        while let Ok((data, count)) = input_rx.recv() {
            let result = writer.write_all(&data).and_then(|_| writer.flush());
            if count > 0 {
                input_state.pending_input.fetch_sub(count, Ordering::AcqRel);
            }
            if result.is_err() {
                input_state.stop();
                break;
            }
        }
    });
    let read_state = state.clone();
    let discard_output = Arc::new(AtomicBool::new(false));
    let discard_reader = discard_output.clone();
    let read_thread = std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            if discard_reader.load(Ordering::Acquire) {
                continue;
            }
            let mut event = Event::Data(buffer[..n].to_vec());
            loop {
                match read_state.events.try_send(event) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(e)) => {
                        event = e;
                        if discard_reader.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => {
                        discard_reader.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }
        let _ = read_state.events.send(Event::Eof);
    });
    let exit = Arc::new(Mutex::new(None));
    let exit_result = exit.clone();
    let wait_state = state.clone();
    let wait_thread = std::thread::spawn(move || {
        let result = child
            .wait()
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string());
        *exit_result.lock().unwrap() = Some(result);
        let _ = wait_state.events.try_send(Event::Wake);
    });
    let mut parser =
        vt100::Parser::new_with_callbacks(rows, cols, screen::HISTORY, Replies::default());
    let mut frame = frames.borrow().clone();
    frame.pid = pid;
    frame.status = "Running".into();
    let mut dirty = true;
    let mut rendered = Instant::now() - TICK;
    let mut result = Ok(());
    loop {
        if state.stop.load(Ordering::Acquire) || exit.lock().unwrap().is_some() {
            break;
        }
        let requested_size = state.requested_size.swap(0, Ordering::AcqRel);
        if requested_size != 0 {
            let (r, c) = screen::size((requested_size >> 16) as u16, requested_size as u16);
            if parser.screen().size() != (r, c) {
                if let Err(e) = pair.master.resize(PtySize {
                    rows: r.get(),
                    cols: c.get(),
                    pixel_width: 0,
                    pixel_height: 0,
                }) {
                    result = Err(e);
                    break;
                }
                parser.set_size(r, c);
                dirty = true;
            }
        }
        if dirty && rendered.elapsed() >= TICK {
            frame = screen::snapshot(parser.screen(), &frame);
            frames.send_replace(frame.clone());
            dirty = false;
            rendered = Instant::now();
        }
        let event = if dirty {
            match rx.recv_timeout(TICK.saturating_sub(rendered.elapsed())) {
                Ok(e) => e,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        } else {
            match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            }
        };
        match event {
            Event::Data(data) => {
                // vt100 advances a detached viewport with new history rows.
                parser.process(&data);
                for reply in std::mem::take(&mut parser.callbacks_mut().0) {
                    let _ = input_tx.try_send((reply, 0));
                }
                dirty = true;
            }
            Event::Input(mut data, paste) => {
                let count = data.len();
                if paste && parser.screen().bracketed_paste() {
                    let mut wrapped = b"\x1b[200~".to_vec();
                    wrapped.append(&mut data);
                    wrapped.extend_from_slice(b"\x1b[201~");
                    data = wrapped;
                }
                parser.screen_mut().set_scrollback(0);
                if input_tx.try_send((data, count)).is_err() {
                    state.pending_input.fetch_sub(count, Ordering::AcqRel);
                    frame.status = "Input backlog: retry your last input".into();
                }
                dirty = true;
            }
            Event::Scroll(delta) => {
                let offset = parser
                    .screen()
                    .scrollback()
                    .saturating_add_signed(delta as isize);
                parser.screen_mut().set_scrollback(offset);
                dirty = true;
            }
            Event::Eof => break,
            Event::Wake => {}
        }
    }
    state.stop.store(true, Ordering::Release);
    group.stop();
    let _ = killer.kill();
    drop(input_tx);
    // ClosePseudoConsole flushes asynchronously. Continue parsing the bounded
    // output channel until the reader sees EOF, retaining the final shell text.
    let close_thread = std::thread::spawn(move || drop(pair.master));
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Data(data)) => parser.process(&data),
            Ok(Event::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    discard_output.store(true, Ordering::Release);
    drop(rx);
    let _ = input_thread.join();
    let _ = read_thread.join();
    let _ = close_thread.join();
    let _ = wait_thread.join();
    if let Some(archive) = &state.archive {
        archive.capture(parser.screen());
    }
    frame = screen::snapshot(parser.screen(), &frame);
    frame.status = exit
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| "Stopped".into());
    frames.send_replace(frame);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v61_close_keeps_first_explicit_import_and_input_is_bounded() {
        let (events, _) = mpsc::sync_channel(1);
        let state = Arc::new(State {
            events,
            stop: AtomicBool::new(false),
            done: AtomicBool::new(false),
            pending_input: AtomicUsize::new(0),
            requested_size: AtomicU32::new(0),
            import: Mutex::new(Vec::new()),
            archive: None,
        });
        let (_, frames) = watch::channel(Frame::default());
        let handle = Handle {
            state: state.clone(),
            frames,
        };
        assert!(handle.input(vec![0; INPUT_LIMIT + 1], true).is_err());
        assert_eq!(state.pending_input.load(Ordering::Acquire), 0);
        handle.close(vec!["selected.rs".into()]);
        handle.close(Vec::new());
        assert_eq!(*state.import.lock().unwrap(), ["selected.rs"]);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn v61_pty_retains_final_output_after_shell_exit() {
        let directory = tempfile::tempdir().unwrap();
        let handle = open(
            ContextInfo {
                cwd: directory.path().to_owned(),
                workspace: None,
            },
            12,
            80,
        )
        .await
        .unwrap();
        let mut frames = handle.frames.clone();
        tokio::time::timeout(Duration::from_secs(15),async {
            while frames.borrow().pid==0 {frames.changed().await.unwrap();}
            handle.input(b"for /L %i in (1,1,100) do @echo line-%i\r\necho terminal-%OS%-tail\r\nexit\r\n".to_vec(),false).unwrap();
            while !frames.borrow().finished {frames.changed().await.unwrap();}
            { let frame=frames.borrow();assert!(frame.text.lines().any(|line|line.trim()=="terminal-Windows_NT-tail"),"{}: {}",frame.status,frame.text); }
            handle.scroll(30);
            while frames.borrow().scrollback == 0 { frames.changed().await.unwrap(); }
            assert!(frames.borrow().text.contains("line-"));
            handle.resize(8,62);
            while frames.borrow().cols != 62 { frames.changed().await.unwrap(); }
            assert_eq!(frames.borrow().rows,8);
        }).await.unwrap();
    }
}
