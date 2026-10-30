//! Own the shell's descendants, including when the shell exits before its tools.
use anyhow::{Context, Result};
#[cfg(windows)]
pub(super) struct Group(std::os::windows::io::OwnedHandle);
#[cfg(windows)]
impl Group {
    pub fn new(
        child: &dyn portable_pty::Child,
        _master: &dyn portable_pty::MasterPty,
    ) -> Result<Self> {
        Self::own(
            child
                .as_raw_handle()
                .context("Terminal shell has no process handle")?,
        )
    }
    fn own(process: windows_sys::Win32::Foundation::HANDLE) -> Result<Self> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            anyhow::ensure!(
                !raw.is_null(),
                "Cannot create terminal process job: {}",
                std::io::Error::last_os_error()
            );
            let handle = std::os::windows::io::OwnedHandle::from_raw_handle(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            anyhow::ensure!(
                SetInformationJobObject(
                    handle.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    std::mem::size_of_val(&limits) as u32
                ) != 0,
                "Cannot configure terminal process job: {}",
                std::io::Error::last_os_error()
            );
            anyhow::ensure!(
                AssignProcessToJobObject(handle.as_raw_handle(), process) != 0,
                "Cannot own terminal process tree: {}",
                std::io::Error::last_os_error()
            );
            Ok(Self(handle))
        }
    }
    pub fn spawn(command: &mut std::process::Command) -> Result<(std::process::Child, Self)> {
        use std::os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        };
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::{Diagnostics::ToolHelp::*, Threading::*},
        };
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let setup = (|| -> Result<Self> {
            let group = Self::own(child.as_raw_handle())?;
            unsafe {
                let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                anyhow::ensure!(
                    snapshot != INVALID_HANDLE_VALUE,
                    "Cannot enumerate suspended command thread"
                );
                let snapshot = OwnedHandle::from_raw_handle(snapshot);
                let mut entry = THREADENTRY32 {
                    dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                    ..Default::default()
                };
                let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry) != 0;
                while found {
                    if entry.th32OwnerProcessID == child.id() {
                        let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                        anyhow::ensure!(!thread.is_null(), "Cannot open suspended command thread");
                        let thread = OwnedHandle::from_raw_handle(thread);
                        anyhow::ensure!(
                            ResumeThread(thread.as_raw_handle()) != u32::MAX,
                            "Cannot resume owned command"
                        );
                        return Ok(group);
                    }
                    found = Thread32Next(snapshot.as_raw_handle(), &mut entry) != 0;
                }
                anyhow::bail!("Suspended command thread is unavailable")
            }
        })();
        match setup {
            Ok(group) => Ok((child, group)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
    pub fn stop(&self) {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}
#[cfg(unix)]
pub(super) struct Group(libc::pid_t);
#[cfg(unix)]
impl Group {
    pub fn new(
        child: &dyn portable_pty::Child,
        master: &dyn portable_pty::MasterPty,
    ) -> Result<Self> {
        Ok(Self(
            master
                .process_group_leader()
                .or_else(|| child.process_id().map(|id| id as libc::pid_t))
                .context("Terminal shell has no process group")?,
        ))
    }
    pub fn spawn(command: &mut std::process::Command) -> Result<(std::process::Child, Self)> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = command.spawn()?;
        let group = Self(child.id() as libc::pid_t);
        Ok((child, group))
    }
    pub fn stop(&self) {
        if self.0 > 0 {
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
        }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        self.stop();
    }
}
