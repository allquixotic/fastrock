#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_RTK_BINARY: &str = "rtk";
pub const DEFAULT_RTK_EVENT_CHANNEL_CAPACITY: usize = 256;
pub const DEFAULT_MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkCommandRequest {
    pub cwd: String,
    pub command: Vec<String>,
    pub shell: Option<String>,
    pub pty: bool,
    pub timeout_ms: Option<u64>,
    pub env: BTreeMap<String, String>,
    pub stdin: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkRunnerConfig {
    pub rtk_binary: String,
    pub event_channel_capacity: usize,
    pub max_capture_bytes: usize,
}

impl Default for RtkRunnerConfig {
    fn default() -> Self {
        Self {
            rtk_binary: DEFAULT_RTK_BINARY.to_owned(),
            event_channel_capacity: DEFAULT_RTK_EVENT_CHANNEL_CAPACITY,
            max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkCommandTranscript {
    pub cwd: String,
    pub argv: Vec<String>,
    pub pty_requested: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub rtk_savings: Option<RtkSavingsMetadata>,
    pub policy_labels: Vec<String>,
    pub events: Vec<RtkCommandEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkSavingsMetadata {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub saved_tokens: u64,
    pub savings_percent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtkCommandEvent {
    Started { cwd: String, argv: Vec<String> },
    Stdout { bytes: Vec<u8> },
    Stderr { bytes: Vec<u8> },
    TimedOut { timeout_ms: u64 },
    Cancelled,
    Exited { exit_code: Option<i32> },
}

#[derive(Debug, Error)]
pub enum RtkRunError {
    #[error("rtk command request cannot be empty")]
    EmptyCommand,
    #[error("rtk event channel capacity must be greater than zero")]
    ZeroEventCapacity,
    #[error("failed to spawn rtk: {0}")]
    Spawn(std::io::Error),
    #[error("failed to read rtk output: {0}")]
    Output(std::io::Error),
    #[error("failed to write rtk stdin: {0}")]
    Stdin(std::io::Error),
    #[error("rtk child process did not expose {0}")]
    MissingPipe(&'static str),
    #[error("command denied by policy: {0}")]
    PolicyDenied(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkBinaryDiagnostic {
    pub binary: String,
    pub status: RtkBinaryStatus,
    pub resolved_path: Option<PathBuf>,
    pub detail: String,
    pub install_guidance: Option<String>,
}

impl RtkBinaryDiagnostic {
    pub fn is_available(&self) -> bool {
        self.status == RtkBinaryStatus::Available
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtkBinaryStatus {
    Available,
    Missing,
}

pub fn diagnose_default_rtk_binary() -> RtkBinaryDiagnostic {
    diagnose_rtk_binary(DEFAULT_RTK_BINARY)
}

pub fn diagnose_rtk_binary(binary: &str) -> RtkBinaryDiagnostic {
    diagnose_rtk_binary_for(binary, std::env::var_os("PATH").as_deref(), Path::new("."))
}

pub fn diagnose_rtk_binary_for(
    binary: &str,
    path_env: Option<&OsStr>,
    cwd: &Path,
) -> RtkBinaryDiagnostic {
    let binary = binary.trim();
    if binary.is_empty() {
        return missing_rtk_diagnostic(binary, "configured rtk binary is empty".to_owned());
    }

    let binary_path = Path::new(binary);
    if binary_path.components().count() > 1 {
        let candidate = if binary_path.is_absolute() {
            binary_path.to_path_buf()
        } else {
            cwd.join(binary_path)
        };
        return diagnostic_for_candidate(binary, candidate);
    }

    if let Some(path_env) = path_env {
        for directory in std::env::split_paths(path_env) {
            for candidate in executable_candidates(&directory, binary) {
                if candidate.is_file() {
                    return RtkBinaryDiagnostic {
                        binary: binary.to_owned(),
                        status: RtkBinaryStatus::Available,
                        resolved_path: Some(candidate),
                        detail: "rtk found on PATH".to_owned(),
                        install_guidance: None,
                    };
                }
            }
        }
    }

    missing_rtk_diagnostic(
        binary,
        format!("rtk binary `{binary}` was not found on PATH"),
    )
}

fn diagnostic_for_candidate(binary: &str, candidate: PathBuf) -> RtkBinaryDiagnostic {
    if candidate.is_file() {
        RtkBinaryDiagnostic {
            binary: binary.to_owned(),
            status: RtkBinaryStatus::Available,
            resolved_path: Some(candidate),
            detail: "configured rtk binary exists".to_owned(),
            install_guidance: None,
        }
    } else {
        missing_rtk_diagnostic(
            binary,
            format!(
                "configured rtk binary does not exist: {}",
                candidate.display()
            ),
        )
    }
}

fn executable_candidates(directory: &Path, binary: &str) -> Vec<PathBuf> {
    let base = directory.join(binary);
    if cfg!(windows) && Path::new(binary).extension().is_none() {
        let pathext = std::env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.CMD;.BAT".into());
        std::env::split_paths(&pathext)
            .map(|extension| {
                let extension = extension.to_string_lossy();
                directory.join(format!("{binary}{extension}"))
            })
            .chain(std::iter::once(base))
            .collect()
    } else {
        vec![base]
    }
}

fn missing_rtk_diagnostic(binary: &str, detail: String) -> RtkBinaryDiagnostic {
    RtkBinaryDiagnostic {
        binary: binary.to_owned(),
        status: RtkBinaryStatus::Missing,
        resolved_path: None,
        detail,
        install_guidance: Some(
            "Install rtk and ensure the configured binary is on PATH.".to_owned(),
        ),
    }
}

pub fn local_rtk_argv(command: &[String]) -> Vec<String> {
    local_rtk_argv_with_binary(DEFAULT_RTK_BINARY, command)
}

pub fn local_rtk_argv_with_binary(rtk_binary: &str, command: &[String]) -> Vec<String> {
    let mut argv = Vec::with_capacity(command.len() + 1);
    argv.push(rtk_binary.to_owned());
    argv.extend(command.iter().cloned());
    argv
}

pub fn local_rtk_request_argv_with_binary(
    rtk_binary: &str,
    request: &RtkCommandRequest,
) -> Vec<String> {
    let args = rtk_request_args(request);
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(rtk_binary.to_owned());
    argv.extend(args);
    argv
}

pub fn rtk_request_args(request: &RtkCommandRequest) -> Vec<String> {
    if let Some(shell) = request
        .shell
        .as_deref()
        .map(str::trim)
        .filter(|shell| !shell.is_empty())
    {
        vec![
            shell.to_owned(),
            "-lc".to_owned(),
            shell_command_line(&request.command),
        ]
    } else {
        request.command.clone()
    }
}

pub async fn run_local_rtk_command(
    request: RtkCommandRequest,
    config: RtkRunnerConfig,
) -> Result<RtkCommandTranscript, RtkRunError> {
    run_local_rtk_command_with_cancellation(request, config, CancellationToken::new()).await
}

pub async fn run_local_rtk_command_with_cancellation(
    request: RtkCommandRequest,
    config: RtkRunnerConfig,
    cancellation_token: CancellationToken,
) -> Result<RtkCommandTranscript, RtkRunError> {
    if request.command.is_empty() {
        return Err(RtkRunError::EmptyCommand);
    }
    if config.event_channel_capacity == 0 {
        return Err(RtkRunError::ZeroEventCapacity);
    }

    let argv = local_rtk_request_argv_with_binary(&config.rtk_binary, &request);
    let command_args = rtk_request_args(&request);
    let cwd = request.cwd.clone();
    let (event_tx, mut event_rx) = mpsc::channel(config.event_channel_capacity);
    let transcript = Arc::new(Mutex::new(RtkCommandTranscript {
        cwd: cwd.clone(),
        argv: argv.clone(),
        pty_requested: request.pty,
        exit_code: None,
        timed_out: false,
        cancelled: false,
        stdout_bytes: 0,
        stderr_bytes: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        rtk_savings: None,
        policy_labels: Vec::new(),
        events: Vec::new(),
    }));
    let collector_transcript = transcript.clone();
    let max_capture_bytes = config.max_capture_bytes;
    let collector = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let mut transcript = collector_transcript.lock().await;
            capture_event(&mut transcript, &event, max_capture_bytes);
            transcript.events.push(event);
        }
    });

    event_tx
        .send(RtkCommandEvent::Started {
            cwd: cwd.clone(),
            argv: argv.clone(),
        })
        .await
        .ok();

    let mut command = Command::new(&config.rtk_binary);
    command
        .args(&command_args)
        .current_dir(PathBuf::from(&request.cwd))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    command.envs(&request.env);

    let mut child = command.spawn().map_err(RtkRunError::Spawn)?;
    if let Some(stdin) = request.stdin {
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or(RtkRunError::MissingPipe("stdin"))?;
        tokio::spawn(async move { child_stdin.write_all(&stdin).await });
    }
    let stdout = child
        .stdout
        .take()
        .ok_or(RtkRunError::MissingPipe("stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(RtkRunError::MissingPipe("stderr"))?;

    let stdout_tx = event_tx.clone();
    let stderr_tx = event_tx.clone();
    tokio::spawn(async move { read_pipe(stdout, OutputPipe::Stdout, stdout_tx).await });
    tokio::spawn(async move { read_pipe(stderr, OutputPipe::Stderr, stderr_tx).await });

    let mut timed_out = false;
    let mut cancelled = false;
    let status_code = if let Some(timeout_ms) = request.timeout_ms {
        tokio::select! {
            status = child.wait() => status.map_err(RtkRunError::Output)?.code(),
            _ = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)) => {
                timed_out = true;
                let _ = event_tx.send(RtkCommandEvent::TimedOut { timeout_ms }).await;
                child.kill().await.map_err(RtkRunError::Output)?;
                None
            }
            _ = cancellation_token.cancelled() => {
                cancelled = true;
                let _ = event_tx.send(RtkCommandEvent::Cancelled).await;
                child.kill().await.map_err(RtkRunError::Output)?;
                None
            }
        }
    } else {
        tokio::select! {
            status = child.wait() => status.map_err(RtkRunError::Output)?.code(),
            _ = cancellation_token.cancelled() => {
                cancelled = true;
                let _ = event_tx.send(RtkCommandEvent::Cancelled).await;
                child.kill().await.map_err(RtkRunError::Output)?;
                None
            }
        }
    };
    event_tx
        .send(RtkCommandEvent::Exited {
            exit_code: status_code,
        })
        .await
        .ok();
    drop(event_tx);
    collector.await.map_err(|error| {
        RtkRunError::Output(std::io::Error::other(format!(
            "rtk event collector failed: {error}"
        )))
    })?;

    let mut transcript = transcript.lock().await.clone();
    transcript.exit_code = status_code;
    transcript.timed_out = timed_out;
    transcript.cancelled = cancelled;
    transcript.rtk_savings = parse_rtk_savings_metadata(&transcript.stdout)
        .or_else(|| parse_rtk_savings_metadata(&transcript.stderr));
    Ok(transcript)
}

fn shell_command_line(command: &[String]) -> String {
    if command.len() == 1 {
        return command[0].clone();
    }
    command
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(argument: &str) -> String {
    if argument
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:=+@%".contains(&byte))
    {
        argument.to_owned()
    } else {
        format!("'{}'", argument.replace('\'', "'\"'\"'"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputPipe {
    Stdout,
    Stderr,
}

async fn read_pipe<R>(
    pipe: R,
    output_pipe: OutputPipe,
    event_tx: mpsc::Sender<RtkCommandEvent>,
) -> Result<(), std::io::Error>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(pipe);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).await?;
        if read == 0 {
            break;
        }
        let redacted = redact_command_output_bytes(&buffer);
        let event = match output_pipe {
            OutputPipe::Stdout => RtkCommandEvent::Stdout { bytes: redacted },
            OutputPipe::Stderr => RtkCommandEvent::Stderr { bytes: redacted },
        };
        if event_tx.send(event).await.is_err() {
            break;
        }
    }
    Ok(())
}

pub fn redact_command_output_bytes(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    redact_secret_markers(text).into_bytes()
}

fn redact_secret_markers(text: &str) -> String {
    [
        "aws_secret_access_key=",
        "secret_access_key=",
        "api_key=",
        "token=",
        "Authorization: Bearer ",
        "authorization: bearer ",
    ]
    .into_iter()
    .fold(text.to_owned(), redact_marker_value)
}

fn redact_marker_value(mut text: String, marker: &str) -> String {
    let mut search_start = 0;
    while let Some(relative_start) = text[search_start..].find(marker) {
        let value_start = search_start + relative_start + marker.len();
        let value_end = text[value_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (character.is_whitespace() || [',', ';', '&', '"', '\''].contains(&character))
                    .then_some(value_start + offset)
            })
            .unwrap_or(text.len());
        text.replace_range(value_start..value_end, "[REDACTED]");
        search_start = value_start + "[REDACTED]".len();
    }
    text
}

fn capture_event(
    transcript: &mut RtkCommandTranscript,
    event: &RtkCommandEvent,
    max_capture_bytes: usize,
) {
    match event {
        RtkCommandEvent::Stdout { bytes } => {
            transcript.stdout_bytes += bytes.len();
            append_capture(
                &mut transcript.stdout,
                bytes,
                max_capture_bytes,
                &mut transcript.stdout_truncated,
            );
        }
        RtkCommandEvent::Stderr { bytes } => {
            transcript.stderr_bytes += bytes.len();
            append_capture(
                &mut transcript.stderr,
                bytes,
                max_capture_bytes,
                &mut transcript.stderr_truncated,
            );
        }
        RtkCommandEvent::TimedOut { .. } => transcript.timed_out = true,
        RtkCommandEvent::Cancelled => transcript.cancelled = true,
        RtkCommandEvent::Started { .. } | RtkCommandEvent::Exited { .. } => {}
    }
}

fn append_capture(
    destination: &mut Vec<u8>,
    bytes: &[u8],
    max_capture_bytes: usize,
    truncated: &mut bool,
) {
    if destination.len() >= max_capture_bytes {
        *truncated = true;
        return;
    }
    let available = max_capture_bytes - destination.len();
    let to_take = available.min(bytes.len());
    destination.extend_from_slice(&bytes[..to_take]);
    if to_take < bytes.len() {
        *truncated = true;
    }
}

pub fn parse_rtk_savings_metadata(bytes: &[u8]) -> Option<RtkSavingsMetadata> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.lines().find_map(parse_rtk_savings_line)
}

fn parse_rtk_savings_line(line: &str) -> Option<RtkSavingsMetadata> {
    let candidate = line
        .strip_prefix("RTK_STATS=")
        .or_else(|| line.strip_prefix("rtk_stats="))
        .unwrap_or(line)
        .trim();
    if !candidate.contains("total_saved") && !candidate.contains("saved_tokens") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(candidate).ok()?;
    let input_tokens = first_json_u64(&value, &["total_input", "input_tokens"])?;
    let output_tokens = first_json_u64(&value, &["total_output", "output_tokens"])?;
    let saved_tokens = first_json_u64(&value, &["total_saved", "saved_tokens"])?;
    let savings_percent = value
        .get("avg_savings_pct")
        .or_else(|| value.get("savings_percent"))
        .and_then(|value| match value {
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::String(text) => Some(text.clone()),
            _ => None,
        });
    Some(RtkSavingsMetadata {
        input_tokens,
        output_tokens,
        saved_tokens,
        savings_percent,
    })
}

fn first_json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_rtk_diagnostic_finds_binary_on_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let binary = temp_dir.path().join("rtk-test");
        std::fs::write(&binary, "").unwrap();

        let diagnostic = diagnose_rtk_binary_for(
            "rtk-test",
            Some(temp_dir.path().as_os_str()),
            Path::new("."),
        );

        assert!(diagnostic.is_available());
        assert_eq!(diagnostic.status, RtkBinaryStatus::Available);
        assert_eq!(diagnostic.resolved_path.as_deref(), Some(binary.as_path()));
        assert_eq!(diagnostic.install_guidance, None);
    }

    #[test]
    fn v2_rtk_diagnostic_reports_missing_binary_with_install_guidance() {
        let temp_dir = tempfile::tempdir().unwrap();

        let diagnostic = diagnose_rtk_binary_for(
            "missing-rtk",
            Some(temp_dir.path().as_os_str()),
            Path::new("."),
        );

        assert!(!diagnostic.is_available());
        assert_eq!(diagnostic.status, RtkBinaryStatus::Missing);
        assert!(diagnostic.resolved_path.is_none());
        assert!(diagnostic.detail.contains("missing-rtk"));
        assert!(
            diagnostic
                .install_guidance
                .as_deref()
                .is_some_and(|guidance| guidance.contains("Install rtk"))
        );
    }

    #[test]
    fn v2_local_rtk_argv_always_prefixes_external_rtk_binary() {
        assert_eq!(
            local_rtk_argv(&["cargo".to_owned(), "test".to_owned()]),
            vec!["rtk".to_owned(), "cargo".to_owned(), "test".to_owned()]
        );
        assert_eq!(
            local_rtk_argv_with_binary("fake-rtk", &["status".to_owned()]),
            vec!["fake-rtk".to_owned(), "status".to_owned()]
        );
    }

    #[test]
    fn v2_local_rtk_request_argv_preserves_shell_semantics() {
        let request = RtkCommandRequest {
            cwd: ".".to_owned(),
            command: vec![
                "cargo".to_owned(),
                "test".to_owned(),
                "name with spaces".to_owned(),
            ],
            shell: Some("zsh".to_owned()),
            pty: false,
            timeout_ms: None,
            env: BTreeMap::new(),
            stdin: None,
        };

        assert_eq!(
            rtk_request_args(&request),
            vec![
                "zsh".to_owned(),
                "-lc".to_owned(),
                "cargo test 'name with spaces'".to_owned(),
            ]
        );
        assert_eq!(
            local_rtk_request_argv_with_binary("rtk", &request),
            vec![
                "rtk".to_owned(),
                "zsh".to_owned(),
                "-lc".to_owned(),
                "cargo test 'name with spaces'".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn t14_runner_executes_through_configured_rtk_binary() {
        let fake = fake_rtk_command(
            "printf 'out\\n'; printf 'err\\n' >&2",
            "cmd /C \"echo out & echo err 1>&2\"",
        );
        let transcript = run_local_rtk_command(
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: fake.args,
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake.binary,
                event_channel_capacity: 4,
                max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            },
        )
        .await
        .unwrap();

        assert_eq!(transcript.exit_code, Some(0));
        assert!(String::from_utf8_lossy(&transcript.stdout).contains("out"));
        assert!(String::from_utf8_lossy(&transcript.stderr).contains("err"));
        assert!(matches!(
            transcript.events.first(),
            Some(RtkCommandEvent::Started { .. })
        ));
    }

    #[tokio::test]
    async fn v12_runner_truncates_large_output_capture() {
        let fake = fake_rtk_command("printf 'abcdef'", "cmd /C \"<nul set /p dummy=abcdef\"");
        let transcript = run_local_rtk_command(
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: fake.args,
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake.binary,
                event_channel_capacity: 1,
                max_capture_bytes: 3,
            },
        )
        .await
        .unwrap();

        assert_eq!(transcript.stdout, b"abc");
        assert_eq!(transcript.stdout_bytes, 6);
        assert_eq!(transcript.stderr_bytes, 0);
        assert!(transcript.stdout_truncated);
    }

    #[test]
    fn t14_runner_parses_rtk_savings_metadata_when_reported() {
        let bytes = br#"command output
RTK_STATS={"total_input":1200,"total_output":300,"total_saved":900,"avg_savings_pct":75.0}
"#;

        assert_eq!(
            parse_rtk_savings_metadata(bytes),
            Some(RtkSavingsMetadata {
                input_tokens: 1200,
                output_tokens: 300,
                saved_tokens: 900,
                savings_percent: Some("75.0".to_owned()),
            })
        );
    }

    #[test]
    fn v9_command_output_redacts_detectable_secret_markers() {
        let redacted = redact_command_output_bytes(
            b"api_key=sk-test token=abc123 Authorization: Bearer bearer-secret\n",
        );
        let text = String::from_utf8(redacted).unwrap();

        assert_eq!(
            text,
            "api_key=[REDACTED] token=[REDACTED] Authorization: Bearer [REDACTED]\n"
        );
    }

    #[tokio::test]
    async fn v9_runner_redacts_command_output_before_transcript_capture() {
        let fake = fake_rtk_command(
            "printf 'api_key=super-secret\\n'; printf 'token=stderr-secret\\n' >&2",
            "cmd /C \"echo api_key=super-secret & echo token=stderr-secret 1>&2\"",
        );
        let transcript = run_local_rtk_command(
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: fake.args,
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake.binary,
                event_channel_capacity: 8,
                max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            },
        )
        .await
        .unwrap();
        let stdout = String::from_utf8_lossy(&transcript.stdout);
        let stderr = String::from_utf8_lossy(&transcript.stderr);

        assert!(stdout.contains("api_key=[REDACTED]"));
        assert!(stderr.contains("token=[REDACTED]"));
        assert!(!stdout.contains("super-secret"));
        assert!(!stderr.contains("stderr-secret"));
        assert!(transcript.events.iter().all(|event| match event {
            RtkCommandEvent::Stdout { bytes } | RtkCommandEvent::Stderr { bytes } => {
                let text = String::from_utf8_lossy(bytes);
                !text.contains("super-secret") && !text.contains("stderr-secret")
            }
            RtkCommandEvent::Started { .. }
            | RtkCommandEvent::TimedOut { .. }
            | RtkCommandEvent::Cancelled
            | RtkCommandEvent::Exited { .. } => true,
        }));
    }

    #[tokio::test]
    async fn t14_runner_timeout_kills_command_and_records_event() {
        let fake = fake_rtk_command("exec sleep 5", "ping -n 6 127.0.0.1 >nul");
        let transcript = run_local_rtk_command(
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: fake.args,
                shell: None,
                pty: true,
                timeout_ms: Some(50),
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake.binary,
                event_channel_capacity: 4,
                max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            },
        )
        .await
        .unwrap();

        assert!(transcript.pty_requested);
        assert!(transcript.timed_out);
        assert_eq!(transcript.exit_code, None);
        assert!(
            transcript
                .events
                .iter()
                .any(|event| { matches!(event, RtkCommandEvent::TimedOut { timeout_ms: 50 }) })
        );
    }

    #[tokio::test]
    async fn t14_runner_cancellation_kills_command_and_records_event() {
        let fake = fake_rtk_command("exec sleep 5", "ping -n 6 127.0.0.1 >nul");
        let cancellation = CancellationToken::new();
        let cancel_task = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_task.cancel();
        });

        let transcript = run_local_rtk_command_with_cancellation(
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: fake.args,
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake.binary,
                event_channel_capacity: 4,
                max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            },
            cancellation,
        )
        .await
        .unwrap();

        assert!(transcript.cancelled);
        assert_eq!(transcript.exit_code, None);
        assert!(
            transcript
                .events
                .iter()
                .any(|event| matches!(event, RtkCommandEvent::Cancelled))
        );
    }

    struct FakeRtkCommand {
        binary: String,
        args: Vec<String>,
    }

    fn fake_rtk_command(unix_script: &str, windows_script: &str) -> FakeRtkCommand {
        if cfg!(windows) {
            FakeRtkCommand {
                binary: "cmd".to_owned(),
                args: vec!["/C".to_owned(), windows_script.to_owned()],
            }
        } else {
            FakeRtkCommand {
                binary: "sh".to_owned(),
                args: vec!["-c".to_owned(), unix_script.to_owned()],
            }
        }
    }
}
