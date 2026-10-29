//! Operating-system integration that Slint and winit do not provide.
//!
//! - macOS: Cmd+Q, Dock › Quit and logout send `terminate:` to the
//!   application, which would exit the process without the app's quit
//!   confirmation or shutdown ([`install_termination_handler`]).
//! - macOS: [`main_bundle_identifier`] lets notifications name the app.
//! - Windows: the GUI subsystem has no console for `--help` and argument
//!   errors ([`attach_parent_console`]).

/// Windows remote desktops can advertise OpenGL while losing the presentation
/// surface across reconnects. Use CPU rendering for Auto in those sessions.
pub(crate) fn remote_desktop_session() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: reads one system metric; no pointers or owned resources.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows_sys::Win32::UI::WindowsAndMessaging::SM_REMOTESESSION,
            ) != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Attaches to the console of the process that started us (cmd.exe or
/// PowerShell), so `--help` and argument errors print there. Returns false
/// when there is none, for example when started from a shortcut.
#[cfg(windows)]
pub(crate) fn attach_parent_console() -> bool {
    use windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS;
    use windows_sys::Win32::System::Console::AttachConsole;

    // SAFETY: plain Win32 call without pointers.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::install_termination_handler;
#[cfg(target_os = "macos")]
pub(crate) use macos::main_bundle_identifier;
#[cfg(target_os = "macos")]
pub(crate) use macos::request_app_termination;

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;

    use objc2::class;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::runtime::AnyObject;
    use objc2::runtime::Imp;
    use objc2::runtime::Sel;
    use objc2::sel;

    /// `NSApplicationTerminateReply`.
    const NS_TERMINATE_CANCEL: usize = 0;
    const NS_TERMINATE_NOW: usize = 1;
    /// `keyAEQuitReason` ('why?'), set on the quit Apple Event that logout,
    /// restart and shutdown send.
    const KEY_AE_QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

    thread_local! {
        static HOOKS: RefCell<Option<Hooks>> = const { RefCell::new(None) };
    }

    struct Hooks {
        /// Runs the normal, confirmable quit (Cmd+Q, Dock › Quit).
        request_quit: Box<dyn Fn()>,
        /// Shuts down synchronously when the session ends (logout).
        session_end: Option<Box<dyn FnOnce()>>,
    }

    /// Routes `applicationShouldTerminate:` to the app.
    ///
    /// winit's application delegate does not implement it, so AppKit would
    /// terminate right away. The handler cancels the termination and calls
    /// `request_quit`, which asks for confirmation when turns are running
    /// and then leaves the event loop so the normal shutdown runs. When the
    /// user logs out (or restarts or shuts down the Mac), cancelling would
    /// abort the logout, so `session_end` runs instead and the app quits.
    ///
    /// Must be called on the main thread after the event loop exists.
    pub(crate) fn install_termination_handler(
        request_quit: impl Fn() + 'static,
        session_end: impl FnOnce() + 'static,
    ) {
        HOOKS.with(|hooks| {
            *hooks.borrow_mut() = Some(Hooks {
                request_quit: Box::new(request_quit),
                session_end: Some(Box::new(session_end)),
            });
        });
        // SAFETY: standard AppKit messages on the main thread; the added
        // method's signature matches `applicationShouldTerminate:`
        // (`NSApplicationTerminateReply (NSApplication *)`).
        unsafe {
            let app: Option<Retained<AnyObject>> =
                msg_send![class!(NSApplication), sharedApplication];
            let Some(app) = app else {
                tracing::warn!("NSApp is missing; Cmd+Q will skip the quit confirmation");
                return;
            };
            let delegate: Option<Retained<AnyObject>> = msg_send![&*app, delegate];
            let Some(delegate) = delegate else {
                tracing::warn!("NSApp has no delegate; Cmd+Q will skip the quit confirmation");
                return;
            };
            let class: *const AnyClass = delegate.class();
            let imp: Imp = std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should_terminate);
            let added = objc2::ffi::class_addMethod(
                class.cast_mut(),
                sel!(applicationShouldTerminate:),
                imp,
                c"Q@:@".as_ptr(),
            );
            if !added.as_bool() {
                tracing::warn!("could not add applicationShouldTerminate: to the app delegate");
                return;
            }
            // AppKit may cache which optional methods a delegate implements
            // when it is assigned; assign it again so the new one is seen.
            let none: Option<&AnyObject> = None;
            let _: () = msg_send![&*app, setDelegate: none];
            let _: () = msg_send![&*app, setDelegate: Some(&*delegate)];
        }
    }

    /// `CFBundleIdentifier` of the running app bundle; `None` for a bare
    /// executable.
    pub(crate) fn main_bundle_identifier() -> Option<String> {
        // SAFETY: Foundation messages; every object may be nil and is
        // checked, and the C string is copied before the NSString can go.
        unsafe {
            let bundle: Option<Retained<AnyObject>> = msg_send![class!(NSBundle), mainBundle];
            let identifier: Option<Retained<AnyObject>> = msg_send![&*bundle?, bundleIdentifier];
            let identifier = identifier?;
            let utf8: *const std::ffi::c_char = msg_send![&*identifier, UTF8String];
            if utf8.is_null() {
                return None;
            }
            let identifier = std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned();
            (!identifier.is_empty()).then_some(identifier)
        }
    }

    /// Sends `terminate:` to the application, as Cmd+Q does (automation).
    pub(crate) fn request_app_termination() {
        // SAFETY: standard AppKit message on the main thread.
        unsafe {
            let app: Option<Retained<AnyObject>> =
                msg_send![class!(NSApplication), sharedApplication];
            if let Some(app) = app {
                let sender: Option<&AnyObject> = None;
                let _: () = msg_send![&*app, terminate: sender];
            }
        }
    }

    extern "C-unwind" fn should_terminate(
        _this: &AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        if session_is_ending() {
            let session_end = HOOKS.with(|hooks| {
                hooks
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut hooks| hooks.as_mut()?.session_end.take())
            });
            if let Some(session_end) = session_end {
                session_end();
            }
            return NS_TERMINATE_NOW;
        }
        let handled = HOOKS.with(|hooks| match hooks.try_borrow() {
            Ok(hooks) => match hooks.as_ref() {
                Some(hooks) => {
                    (hooks.request_quit)();
                    true
                }
                None => false,
            },
            Err(_) => false,
        });
        if handled {
            NS_TERMINATE_CANCEL
        } else {
            NS_TERMINATE_NOW
        }
    }

    /// Whether the quit request comes from logout, restart or shutdown.
    fn session_is_ending() -> bool {
        // SAFETY: Foundation messages on the main thread inside an AppKit
        // callback; every object may be nil and is checked.
        unsafe {
            let manager: Option<Retained<AnyObject>> =
                msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            let Some(manager) = manager else {
                return false;
            };
            let event: Option<Retained<AnyObject>> = msg_send![&*manager, currentAppleEvent];
            let Some(event) = event else {
                return false;
            };
            let reason: Option<Retained<AnyObject>> =
                msg_send![&*event, attributeDescriptorForKeyword: KEY_AE_QUIT_REASON];
            let Some(reason) = reason else {
                return false;
            };
            let code: u32 = msg_send![&*reason, enumCodeValue];
            code != 0
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn bare_executables_have_no_bundle_identifier() {
        // The test binary is not inside an app bundle.
        assert_eq!(main_bundle_identifier(), None);
    }
}
