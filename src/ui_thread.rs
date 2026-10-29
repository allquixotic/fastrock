//! Window-scoped access to UI controllers. Native callbacks capture their owner
//! at binding time; backend tasks carry it across awaits in a Tokio task local.
//! No routing depends on focus, and server requests are delivered exactly once.
use crate::{app::AppController, transport::AppServerEvent};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    future::Future,
    rc::Rc,
};

pub(crate) type WindowId = u64;
pub(crate) const MAIN: WindowId = 1;
thread_local! {
    static APPS: RefCell<BTreeMap<WindowId, Rc<RefCell<AppController>>>> = const { RefCell::new(BTreeMap::new()) };
    static CURRENT: Cell<Option<WindowId>> = const { Cell::new(None) };
    static NEXT: Cell<WindowId> = const { Cell::new(MAIN + 1) };
    static WINDOWS: RefCell<BTreeMap<usize, WindowId>> = const { RefCell::new(BTreeMap::new()) };
    static ROUTES: RefCell<Routes> = const { RefCell::new(Routes { owners: BTreeMap::new(), parents: BTreeMap::new() }) };
}
tokio::task_local! { static ASYNC_WINDOW: WindowId; }

#[derive(Default)]
struct Routes {
    owners: BTreeMap<String, WindowId>,
    parents: BTreeMap<String, String>,
}
impl Routes {
    fn owner(&self, thread: &str) -> Option<WindowId> {
        let mut thread = thread;
        // Malformed lineage must never hang the UI.
        for _ in 0..64 {
            if let Some(owner) = self.owners.get(thread) {
                return Some(*owner);
            }
            thread = self.parents.get(thread)?;
        }
        None
    }
    fn request_owner(&self, request: &codex_app_server_protocol::ServerRequest) -> WindowId {
        use codex_app_server_protocol::ServerRequest as R;
        match request {
            R::DynamicToolCall { params, .. } => self.owner(&params.thread_id),
            R::ToolRequestUserInput { params, .. } => self.owner(&params.thread_id),
            _ => {
                let value = serde_json::to_value(request).unwrap_or_default();
                value["params"]["threadId"]
                    .as_str()
                    .and_then(|thread| self.owner(thread))
            }
        }
        .unwrap_or(MAIN)
    }
    fn sync(&mut self, window: WindowId, threads: Vec<String>) {
        self.owners.retain(|_, owner| *owner != window);
        for thread in threads {
            self.owners.entry(thread).or_insert(window);
        }
    }
}

pub(crate) fn current() -> WindowId {
    CURRENT
        .get()
        .or_else(|| ASYNC_WINDOW.try_with(|id| *id).ok())
        .unwrap_or(MAIN)
}
pub(crate) fn reserve() -> WindowId {
    NEXT.with(|next| {
        let id = next.get();
        next.set(id.checked_add(1).expect("window identity exhausted"));
        id
    })
}
pub(crate) fn scope<T>(id: WindowId, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<WindowId>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CURRENT.set(self.0);
        }
    }
    let _reset = Reset(CURRENT.replace(Some(id)));
    f()
}
pub(crate) async fn async_scope<T>(id: WindowId, future: impl Future<Output = T>) -> T {
    ASYNC_WINDOW.scope(id, future).await
}

/// Slint futures also need an owner while suspended on native dialogs.
pub(crate) fn spawn_local<F: Future + 'static>(
    future: F,
) -> Result<slint::JoinHandle<F::Output>, slint::EventLoopError> {
    slint::spawn_local(async_scope(current(), future))
}

// Keep argument types inferred by the generated Slint binding, including
// synchronous return values and popup menus that have their own native window.
macro_rules! scoped_callback {
    (move || $body:expr) => { $crate::ui_thread::scoped_callback!(|| $body) };
    (|| $body:expr) => {{ let owner=$crate::ui_thread::current(); move || $crate::ui_thread::scope(owner, || $body) }};
    (move |$($arg:pat_param),*| $body:expr) => { $crate::ui_thread::scoped_callback!(|$($arg),*| $body) };
    (|$($arg:pat_param),*| $body:expr) => {{ let owner=$crate::ui_thread::current(); move |$($arg),*| $crate::ui_thread::scope(owner, || $body) }};
}
pub(crate) use scoped_callback;

pub(crate) fn register_window(window: &slint::Window, owner: WindowId) {
    WINDOWS.with_borrow_mut(|windows| {
        windows.insert(crate::window_runtime::identity(window), owner);
    });
}
pub(crate) fn window_owner(window: &slint::Window) -> Option<WindowId> {
    WINDOWS.with_borrow(|windows| {
        windows
            .get(&crate::window_runtime::identity(window))
            .copied()
    })
}
pub(crate) fn sync_threads(window: WindowId, threads: Vec<String>) {
    ROUTES.with_borrow_mut(|routes| routes.sync(window, threads));
}
pub(crate) fn thread_owner(thread: &str) -> Option<WindowId> {
    ROUTES.with_borrow(|routes| routes.owner(thread))
}
pub(crate) fn install(app: AppController) {
    let id = app.backend.window_id();
    register_window(app.window.window(), id);
    APPS.with_borrow_mut(|apps| {
        assert!(apps.insert(id, Rc::new(RefCell::new(app))).is_none());
    });
}
pub(crate) fn ids() -> Vec<WindowId> {
    APPS.with_borrow(|apps| apps.keys().copied().collect())
}
pub(crate) fn remove(id: WindowId) {
    WINDOWS.with_borrow_mut(|windows| windows.retain(|_, owner| *owner != id));
    ROUTES.with_borrow_mut(|routes| routes.sync(id, vec![]));
    APPS.with_borrow_mut(|apps| apps.remove(&id));
}
/// Inspect every window after leaving the invoking callback's mutable borrow.
pub(crate) fn request_quit() {
    let caller = current();
    let _ = slint::invoke_from_event_loop(move || {
        let mut pending = None;
        let mut running = 0;
        for id in ids() {
            with_app_now_for(id, |app| {
                if app.rally_has_pending_write() {
                    pending = Some(id);
                }
                running += app.running_thread_count();
            });
        }
        if let Some(id) = pending {
            with_app_for(id, |app| {
                let _ = app.window.show();
                app.toast("Wait for Rally writes to finish before quitting");
            });
        } else if running > 0 {
            with_app_for(caller, move |app| {
                let _ = app.window.show();
                app.confirm_process_quit(running);
            });
        } else {
            finish_quit();
        }
    });
}
pub(crate) fn finish_quit() {
    // A dialog may have been open while another window began a Rally write.
    let _ = slint::invoke_from_event_loop(|| {
        let mut pending = false;
        for id in ids() {
            with_app_now_for(id, |app| pending |= app.rally_has_pending_write());
        }
        if pending {
            request_quit();
            return;
        }
        for id in ids() {
            with_app_now_for(id, AppController::save_before_quit);
        }
        let _ = slint::quit_event_loop();
    });
}
pub(crate) fn another_window_visible(owner: WindowId) -> bool {
    let mut visible = false;
    for id in ids().into_iter().filter(|id| *id != owner) {
        with_app_now_for(id, |app| visible |= app.window.window().is_visible());
    }
    visible
}

pub(crate) fn take_all() -> Vec<AppController> {
    WINDOWS.with_borrow_mut(BTreeMap::clear);
    ROUTES.with_borrow_mut(|routes| *routes = Routes::default());
    APPS.with_borrow_mut(std::mem::take)
        .into_values()
        .map(|app| {
            Rc::try_unwrap(app)
                .ok()
                .expect("controller still borrowed during shutdown")
                .into_inner()
        })
        .collect()
}
pub(crate) fn with_app(f: impl FnOnce(&mut AppController) + 'static) {
    with_app_for(current(), f);
}
pub(crate) fn with_app_for(id: WindowId, f: impl FnOnce(&mut AppController) + 'static) {
    let app = APPS.with_borrow(|apps| apps.get(&id).cloned());
    let Some(app) = app else {
        return;
    };
    match app.try_borrow_mut() {
        Ok(mut app) => scope(id, || f(&mut app)),
        Err(_) => slint::Timer::single_shot(std::time::Duration::ZERO, move || with_app_for(id, f)),
    }
}
pub(crate) fn post(f: impl FnOnce(&mut AppController) + Send + 'static) -> bool {
    post_for(current(), f)
}
pub(crate) fn post_for(id: WindowId, f: impl FnOnce(&mut AppController) + Send + 'static) -> bool {
    slint::invoke_from_event_loop(move || with_app_for(id, f)).is_ok()
}
pub(crate) fn post_all(f: impl Fn(&mut AppController) + Send + 'static) -> bool {
    slint::invoke_from_event_loop(move || {
        let f = Rc::new(f);
        for id in ids() {
            let f = f.clone();
            with_app_for(id, move |app| f(app));
        }
    })
    .is_ok()
}
pub(crate) fn post_events(events: Vec<AppServerEvent>) {
    let _ = slint::invoke_from_event_loop(move || {
        let mut batches: BTreeMap<WindowId, Vec<AppServerEvent>> = BTreeMap::new();
        for event in events {
            if let AppServerEvent::ServerNotification(notification) = &event {
                if let codex_app_server_protocol::ServerNotification::ThreadStarted(n) =
                    notification.as_ref()
                {
                    if let Some(parent) = &n.thread.parent_thread_id {
                        ROUTES.with_borrow_mut(|routes| {
                            routes.parents.insert(n.thread.id.clone(), parent.clone());
                        });
                    }
                }
            }
            if let AppServerEvent::ServerRequest(request) = &event {
                let owner = ROUTES.with_borrow(|routes| routes.request_owner(request));
                batches.entry(owner).or_default().push(event);
            } else {
                for id in ids() {
                    batches.entry(id).or_default().push(event.clone());
                }
            }
        }
        for (id, events) in batches {
            with_app_for(id, move |app| app.handle_server_events(events));
        }
    });
}
pub(crate) fn with_app_now(f: impl FnOnce(&mut AppController)) {
    with_app_now_for(current(), f);
}
pub(crate) fn with_app_now_for(id: WindowId, f: impl FnOnce(&mut AppController)) {
    let app = APPS.with_borrow(|apps| apps.get(&id).cloned());
    if let Some(app) = app {
        if let Ok(mut app) = app.try_borrow_mut() {
            scope(id, || f(&mut app));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v48_callbacks_keep_owner_and_restore_nested_scopes() {
        let callback = scope(7, || scoped_callback!(|value| (current(), value)));
        scope(9, || {
            assert_eq!(callback(42), (7, 42));
            assert_eq!(current(), 9);
        });
        assert_eq!(current(), MAIN);
        let timer = scope(8, || scoped_callback!(|| current()));
        assert_eq!(timer(), 8);
        let _ = std::panic::catch_unwind(|| scope(12, || panic!("fixture")));
        assert_eq!(current(), MAIN);
    }
    #[tokio::test]
    async fn v48_async_owner_survives_yields_and_concurrent_tasks() {
        let work = |id| {
            tokio::spawn(async_scope(id, async move {
                for _ in 0..20 {
                    tokio::task::yield_now().await;
                    assert_eq!(current(), id);
                }
                assert_eq!(scope(id + 30, current), id + 30);
                let nested = async_scope(id + 10, async { current() }).await;
                assert_eq!(nested, id + 10);
                assert_eq!(current(), id);
            }))
        };
        let (a, b) = tokio::join!(work(2), work(3));
        a.unwrap();
        b.unwrap();
        assert_eq!(current(), MAIN);
    }
    #[test]
    fn v48_tool_and_permission_requests_have_one_explicit_owner() {
        let mut routes = Routes::default();
        routes.sync(4, vec!["child-window-session".into()]);
        for request in [
            serde_json::json!({"method":"item/tool/call","id":"call-1","params":{"threadId":"child-window-session","turnId":"turn-1","callId":"call-1","tool":"rally_read","arguments":{}}}),
            serde_json::json!({"method":"item/tool/requestUserInput","id":"permission-1","params":{"threadId":"child-window-session","turnId":"turn-1","itemId":"tool-1","questions":[]}}),
        ] {
            let request = serde_json::from_value(request).unwrap();
            assert_eq!(routes.request_owner(&request), 4);
            let unknown = Routes::default();
            assert_eq!(unknown.request_owner(&request), MAIN);
        }
    }

    #[test]
    fn v48_requests_follow_lineage_and_closed_threads_release_ownership() {
        let mut routes = Routes::default();
        routes.sync(2, vec!["parent".into()]);
        routes.parents.insert("child".into(), "parent".into());
        assert_eq!(routes.owner("child"), Some(2));
        routes.sync(3, vec!["parent".into()]);
        assert_eq!(routes.owner("parent"), Some(2));
        routes.sync(2, vec![]);
        assert_eq!(routes.owner("child"), None);
        routes.sync(3, vec!["parent".into()]);
        assert_eq!(routes.owner("child"), Some(3));
        routes.parents.insert("loop".into(), "loop".into());
        assert_eq!(routes.owner("loop"), None);
    }
}
