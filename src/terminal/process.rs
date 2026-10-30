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
            let process = child
                .as_raw_handle()
                .context("Terminal shell has no process handle")?;
            anyhow::ensure!(
                AssignProcessToJobObject(handle.as_raw_handle(), process) != 0,
                "Cannot own terminal process tree: {}",
                std::io::Error::last_os_error()
            );
            Ok(Self(handle))
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
