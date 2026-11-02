//! Give unmodified Goose hooks a console they can inherit without opening a window.
//!
//! Goose hides Developer shell processes, but its command-hook launcher uses an
//! ordinary `sh -c` spawn. From a GUI process with no console, Windows gives that
//! child a visible console. Allocate a windowless session before starting threads.
//! Unlike AllocConsole followed by ShowWindow, this cannot flash or steal focus.
use std::io;
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{Console::*, LibraryLoader::*},
};

#[repr(C)]
struct AllocConsoleOptions {
    mode: i32,
    use_show_window: i32,
    show_window: u16,
}

type AllocConsoleWithOptions =
    unsafe extern "system" fn(*const AllocConsoleOptions, *mut i32) -> i32;

/// Call only at application startup, before threads; helpers and CLI output skip
/// this path. Existing consoles belong to the caller and are never hidden/freed.
/// Returns false on older Windows without the documented 24H2 API.
pub(super) fn prepare() -> io::Result<bool> {
    // SAFETY: valid Win32 output buffers, documented C layouts and ABI. Kernel32
    // stays loaded for process lifetime. Dynamic resolution preserves loading on
    // older Windows; we never modify Goose or intercept its process launches.
    unsafe {
        let mut pid = 0;
        if GetConsoleProcessList(&mut pid, 1) != 0 {
            return Ok(true);
        }
        let kernel = GetModuleHandleW(windows_sys::w!("kernel32.dll"));
        let Some(function) = GetProcAddress(kernel, c"AllocConsoleWithOptions".as_ptr().cast())
        else {
            return Ok(false);
        };
        let allocate: AllocConsoleWithOptions = std::mem::transmute(function);
        let streams = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
        let saved = streams.map(|stream| GetStdHandle(stream));
        let options = AllocConsoleOptions {
            mode: 2, // ALLOC_CONSOLE_MODE_NO_WINDOW
            use_show_window: 0,
            show_window: 0,
        };
        let status = allocate(&options, std::ptr::null_mut());
        if status < 0 {
            return Err(io::Error::other(format!(
                "Windowless console allocation failed: HRESULT {status:#010x}"
            )));
        }
        // Allocation initializes the standard handles. Preserve redirected logs,
        // pipes and absent handles, rather than redirecting them into the console.
        let allocated = streams.map(|stream| GetStdHandle(stream));
        let mut restore_error = None;
        for (stream, handle) in streams.into_iter().zip(saved) {
            if SetStdHandle(stream, handle) == 0 {
                restore_error = Some(io::Error::last_os_error());
            }
        }
        if let Some(error) = restore_error {
            return Err(error);
        }
        for (index, handle) in allocated.into_iter().enumerate() {
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && !saved.contains(&handle)
                && !allocated[..index].contains(&handle)
            {
                CloseHandle(handle);
            }
        }
        // Keep the console attached: descendants share it. Windows releases the
        // session when its last process exits; no launcher/helper process exists.
        Ok(true)
    }
}
