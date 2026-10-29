//! Process lifetime is independent of visible windows. All UI and native tray
//! callbacks run on Slint's event loop; persistence stays on the backend.
use crate::{backend::Backend, ui::FastrockTray, ui_thread};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::cell::{Cell, RefCell};
thread_local! {
    static TRAY: RefCell<Option<FastrockTray>> = const { RefCell::new(None) };
    static LAST_WINDOW: Cell<ui_thread::WindowId> = const { Cell::new(ui_thread::MAIN) };
    static ICON_READY: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn start(backend: &Backend) -> Result<(), slint::PlatformError> {
    backend.spawn_blocking(|| {
        let icon = crate::app::decode_icon(crate::app::APP_ICON_PNG, 32);
        ui_thread::post_for(ui_thread::MAIN, move |app| {
            if let Some(icon) = icon {
                // Slint 1.18 creates the native handle on the first event-loop
                // evaluation. Construct only once the initial icon is ready.
                match FastrockTray::new() {
                    Ok(tray) => {
                        tray.set_tray_icon(slint::Image::from_rgba8(icon));
                        tray.on_restore(|| restore(false));
                        tray.on_restore_all(|| restore(true));
                        tray.on_quit(|| {
                            ui_thread::scope(LAST_WINDOW.get(), ui_thread::request_quit)
                        });
                        TRAY.with_borrow_mut(|slot| *slot = Some(tray));
                        ICON_READY.set(true);
                    }
                    Err(error) => {
                        app.push_warning(format!("Could not create the tray icon: {error}"))
                    }
                }
            } else {
                app.push_warning("Could not load the tray icon; keep a window open.".into());
            }
        });
    });
    Ok(())
}
pub(crate) fn focused(owner: ui_thread::WindowId) {
    LAST_WINDOW.set(owner);
}
pub(crate) fn can_hide_last_window() -> bool {
    // A close arriving before the asynchronously decoded icon is installed
    // keeps the window reachable. Windows also verifies native registration.
    if !ICON_READY.get() {
        return false;
    }
    #[cfg(windows)]
    {
        return crate::window_runtime::native_tray_registered();
    }
    #[cfg(not(windows))]
    {
        true
    }
}
pub(crate) fn restore(all: bool) {
    let last = LAST_WINDOW.get();
    let ids = ui_thread::ids();
    let selected = if ids.contains(&last) {
        last
    } else {
        ui_thread::MAIN
    };
    for id in ids.into_iter().filter(|id| all || *id == selected) {
        ui_thread::with_app_for(id, |app| {
            if let Err(error) = app.window.show() {
                app.push_warning(format!("Could not restore window: {error}"));
                return;
            }
            app.window.window().with_winit_window(|window| {
                window.set_minimized(false);
                window.focus_window();
            });
        });
    }
}
pub(crate) fn stop() {
    TRAY.with_borrow_mut(|slot| {
        slot.take();
    });
    ICON_READY.set(false);
}
