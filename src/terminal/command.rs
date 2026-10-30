//! Explicit one-shot commands preserve shell syntax and own their process tree.
use super::{
    lease::{ContextInfo, Lease},
    process::Group,
};
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

pub const OUTPUT_LIMIT: usize = 256 * 1024;
#[derive(Default)]
pub(super) struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}
impl Capture {
    pub fn push(&mut self, bytes: &[u8]) {
        let take = bytes
            .len()
            .min(OUTPUT_LIMIT.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..take]);
        self.truncated |= take != bytes.len();
    }
    fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str("\n[Output truncated after 256 KiB]\n");
        }
        text
    }
}
pub(crate) struct Outcome {
    pub output: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub interrupted: bool,
    pub cwd: String,
}

pub(crate) async fn run(
    context: ContextInfo,
    command: String,
    timeout: Duration,
    mut cancel: watch::Receiver<bool>,
    mut progress: impl FnMut(String) + Send,
) -> Result<Outcome> {
    ensure!(
        !command.trim().is_empty() && command.len() <= 64 * 1024 && !command.contains('\0'),
        "Command must be nonempty, contain no NUL, and be at most 64 KiB"
    );
    if timeout.is_zero() || *cancel.borrow() {
        return Ok(Outcome {
            output: String::new(),
            status: "Command canceled before launch".into(),
            exit_code: None,
            interrupted: true,
            cwd: context.cwd.to_string_lossy().into_owned(),
        });
    }
    let lease = Lease::open(context).await?;
    let cwd = lease.cwd.clone();
    let capture = Arc::new(Mutex::new(Capture::default()));
    let stop = Arc::new(AtomicBool::new(false));
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _stop = Stop(stop.clone());
    let worker_stop = stop.clone();
    let worker_capture = capture.clone();
    let mut worker =
        tokio::task::spawn_blocking(move || execute(cwd, command, worker_stop, worker_capture));
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut interrupted = false;
    let mut reason = String::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut renew = tokio::time::interval(lease.renew_after);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut delivered = 0;
    let result = loop {
        tokio::select! {
            result = &mut worker => break result,
            _ = tick.tick() => {
                let text = capture.lock().unwrap().text();
                if text.len() != delivered { delivered = text.len(); progress(text); }
            },
            _ = renew.tick(), if !interrupted => {
                if let Err(error) = lease.renew().await { interrupted = true; reason = format!("Workspace lease renewal failed: {error}"); stop.store(true, Ordering::Release); }
            },
            _ = &mut deadline, if !interrupted => {
                interrupted = true; reason = "Command timed out".into(); stop.store(true, Ordering::Release);
            },
            _ = cancel.changed(), if !interrupted => {
                interrupted = true; reason = "Command stopped".into(); stop.store(true, Ordering::Release);
            },
        }
    };
    let cwd = lease.cwd.to_string_lossy().into_owned();
    let released = lease.finish(Vec::new()).await;
    let (status, code) = match result {
        Ok(Ok(exit)) => (exit.to_string(), exit.code()),
        error => {
            interrupted = true;
            (format!("Command failed: {error:?}"), None)
        }
    };
    let output = capture.lock().unwrap().text();
    Ok(Outcome {
        output,
        status: format!("{reason} {status}. {released}").trim().into(),
        exit_code: code,
        interrupted,
        cwd,
    })
}

fn execute(
    cwd: std::path::PathBuf,
    text: String,
    stop: Arc<AtomicBool>,
    capture: Arc<Mutex<Capture>>,
) -> Result<std::process::ExitStatus> {
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(
            std::env::var_os("COMSPEC").unwrap_or_else(|| "C:\\Windows\\System32\\cmd.exe".into()),
        );
        // /S strips only these outer quotes. Preserve the user's inner shell
        // quoting literally; CRT argument escaping would insert backslashes.
        command
            .args(["/D", "/S", "/C"])
            .raw_arg(format!("\"{text}\""));
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command =
            Command::new(std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()));
        command.arg("-c").arg(text);
        command
    };
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ensure!(
        !stop.load(Ordering::Acquire),
        "Command canceled before launch"
    );
    let (mut child, group) = Group::spawn(&mut command)?;
    fn read(
        mut pipe: impl Read + Send + 'static,
        capture: Arc<Mutex<Capture>>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n) = pipe.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                capture.lock().unwrap().push(&buffer[..n]);
            }
        })
    }
    let stdout = read(
        child.stdout.take().context("Command stdout unavailable")?,
        capture.clone(),
    );
    let stderr = read(
        child.stderr.take().context("Command stderr unavailable")?,
        capture,
    );
    let result = loop {
        if stop.load(Ordering::Acquire) {
            group.stop();
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(exit)) => break Ok(exit),
            Err(error) => break Err(error),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    // Descendants must stop even when the parent exits first, otherwise pipe
    // readers can remain blocked forever and a tool can outlive its owner.
    group.stop();
    let _ = child.kill();
    let _ = child.wait();
    let _ = stdout.join();
    let _ = stderr.join();
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v73_command_output_is_bounded_and_marks_truncation() {
        let mut capture = Capture::default();
        capture.push(&vec![b'a'; OUTPUT_LIMIT - 1]);
        capture.push("λ remainder".as_bytes());
        assert_eq!(capture.bytes.len(), OUTPUT_LIMIT);
        assert!(capture.text().contains("Output truncated"));
        for _ in 0..100 {
            capture.push(&[b'x'; 8192]);
        }
        assert_eq!(capture.bytes.len(), OUTPUT_LIMIT);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn v73_owned_commands_preserve_syntax_exit_code_timeout_and_stop() {
        let temp = tempfile::tempdir().unwrap();
        let context = ContextInfo {
            cwd: temp.path().to_owned(),
            workspace: None,
        };
        let (cancel, receiver) = watch::channel(false);
        let result = run(
            context.clone(),
            "echo first & echo second> \"named file.txt\" & type \"named file.txt\" & exit /b 7"
                .into(),
            Duration::from_secs(10),
            receiver.clone(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(7), "{}", result.status);
        assert!(
            result.output.contains("first") && result.output.contains("second"),
            "{}",
            result.output
        );
        assert!(temp.path().join("named file.txt").exists());
        let result = run(
            context.clone(),
            "echo forbidden> never.txt".into(),
            Duration::ZERO,
            receiver.clone(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(result.interrupted && !temp.path().join("never.txt").exists());
        let result = run(
            context.clone(),
            "ping -n 30 127.0.0.1 > NUL".into(),
            Duration::from_millis(150),
            receiver.clone(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(
            result.interrupted && result.status.contains("timed out"),
            "{}",
            result.status
        );
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            cancel.send_replace(true);
        });
        let result = run(
            context,
            "echo before-stop & ping -n 30 127.0.0.1 > NUL".into(),
            Duration::from_secs(10),
            receiver,
            |_| {},
        )
        .await
        .unwrap();
        assert!(
            result.interrupted && result.status.contains("stopped"),
            "{}",
            result.status
        );
    }
}
