//! Keeping a venv's name in its LabVIEW's window titles (ported from upstream
//! lvpm 6a7720f and d69a031).
//!
//! A venv LabVIEW is the same exe as the user's own and its window titles read the
//! same, so `lvpm launch` starts this watcher beside it, hidden, and it tags
//! every window title with the venv's name until that LabVIEW exits.
//!
//! What it relies on, measured on LabVIEW 2026 (Windows):
//!
//! - LabVIEW's own windows (Getting Started, Project Explorer, front panels,
//!   block diagrams, class and library windows) are class `LVDChild` with
//!   `WS_EX_APPWINDOW`; its dialogs and floating palettes lack that bit. The
//!   `LVFrame` window owns them all, and its title is the one Alt+Tab shows.
//!   Add-ons keep hidden app windows of their own: only visible ones are
//!   touched, so a tool that finds its own window by title keeps finding it.
//! - LabVIEW rebuilds a title from its own state on every edit, save or
//!   rename, which drops the tag, and never doubles it: putting the tag back
//!   after each change is all it takes.
//! - Where the tag goes matters. In front, or at the end but before LabVIEW's
//!   ` *` for unsaved changes, the `*` keeps working. Written after the `*`,
//!   LabVIEW lost track of its dirty marker and fell back to short titles.
//! - Reading another process's titles does not involve that process; only a
//!   rename reaches LabVIEW's UI thread, one per window per change.

#[cfg(any(windows, test))]
use crate::config::TitlePosition;

/// The title `title` should have, tagged with `tag` at `pos`, or `None` when
/// it already has it. The tag is first removed from wherever an earlier
/// position left it, so changing the setting moves it.
///
/// `None` for an unchanged title is what ends the loop of rename, title
/// change event, rename.
#[cfg(any(windows, test))]
pub fn retitle(title: &str, tag: &str, pos: TitlePosition) -> Option<String> {
    if title.is_empty() || tag.is_empty() || pos == TitlePosition::Off {
        return None;
    }
    let base = untagged(title, tag);
    if base.is_empty() {
        return None;
    }
    let want = match pos {
        TitlePosition::Start => format!("{tag} {base}"),
        TitlePosition::End => match base.strip_suffix(" *") {
            Some(clean) => format!("{clean} {tag} *"),
            None => format!("{base} {tag}"),
        },
        TitlePosition::Off => return None,
    };
    (want != title).then_some(want)
}

/// `title` without the tag in any of the places `retitle` puts it — in
/// front, at the end, or before a closing ` *` — and nowhere else, so a VI
/// whose own name contains the tag's text keeps it.
#[cfg(any(windows, test))]
fn untagged(title: &str, tag: &str) -> String {
    let mut t = title;
    let front = format!("{tag} ");
    while let Some(rest) = t.strip_prefix(front.as_str()) {
        t = rest;
    }
    let mut t = t.to_string();
    let end = format!(" {tag}");
    let before_star = format!(" {tag} *");
    loop {
        if let Some(rest) = t.strip_suffix(before_star.as_str()) {
            t = format!("{rest} *");
        } else if let Some(rest) = t.strip_suffix(end.as_str()) {
            t = rest.to_string();
        } else {
            break;
        }
    }
    if t == tag { String::new() } else { t }
}

/// What the taskbar gets told about each window: an app id of the venv's own,
/// so its windows group apart from the user's LabVIEW, and how to start it
/// again, which is what pinning that group records.
///
/// Measured on LabVIEW 2026: LabVIEW sets no app id on its windows, so the
/// taskbar groups them by exe path — every LabVIEW 2026, venv or not, in one
/// button. A window's own app id overrides that, and changing it on a shown
/// window moves the window to its new group.
#[cfg(windows)]
pub struct Taskbar {
    pub id: String,
    pub relaunch: String,
    pub icon: String,
    pub name: String,
}

/// Tag the windows of the LabVIEW with process id `pid` until it exits.
#[cfg(windows)]
pub fn watch(
    pid: u32,
    tag: &str,
    pos: TitlePosition,
    taskbar: Option<Taskbar>,
) -> anyhow::Result<()> {
    win::watch(pid, tag, pos, taskbar)
}

#[cfg(windows)]
mod win {
    use super::{Taskbar, retitle};
    use crate::config::TitlePosition;
    use anyhow::{Result, bail};
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type Handle = *mut c_void;
    type Hook = *mut c_void;
    type WinEventProc = unsafe extern "system" fn(Hook, u32, Hwnd, i32, i32, u32, u32);
    type EnumProc = unsafe extern "system" fn(Hwnd, isize) -> i32;

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
        private: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWinEventHook(
            min: u32,
            max: u32,
            module: *mut c_void,
            proc_: WinEventProc,
            pid: u32,
            tid: u32,
            flags: u32,
        ) -> Hook;
        fn UnhookWinEvent(hook: Hook) -> i32;
        fn EnumWindows(proc_: EnumProc, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetClassNameW(hwnd: Hwnd, buf: *mut u16, len: i32) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, len: i32) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        fn GetWindowLongW(hwnd: Hwnd, index: i32) -> i32;
        fn SendMessageTimeoutW(
            hwnd: Hwnd,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            ms: u32,
            result: *mut usize,
        ) -> isize;
        fn MsgWaitForMultipleObjects(
            count: u32,
            handles: *const Handle,
            wait_all: i32,
            ms: u32,
            wake: u32,
        ) -> u32;
        fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn LoadImageW(
            instance: Handle,
            name: *const u16,
            kind: u32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> Handle;
        fn GetDpiForWindow(hwnd: Hwnd) -> u32;
        fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    }

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }

    const EVENT_OBJECT_SHOW: u32 = 0x8002;
    const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
    const WINEVENT_OUTOFCONTEXT: u32 = 0;
    const OBJID_WINDOW: i32 = 0;
    const CHILDID_SELF: i32 = 0;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_APPWINDOW: u32 = 0x0004_0000;
    const WM_SETTEXT: u32 = 0x000C;
    const WM_GETICON: u32 = 0x007F;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const SM_CXICON: i32 = 11;
    const SM_CXSMICON: i32 = 49;
    const IMAGE_ICON: u32 = 1;
    const LR_LOADFROMFILE: u32 = 0x10;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    const SET_TIMEOUT_MS: u32 = 500;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const QS_ALLINPUT: u32 = 0x04FF;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_FAILED: u32 = 0xFFFF_FFFF;
    const INFINITE: u32 = 0xFFFF_FFFF;
    const PM_REMOVE: u32 = 0x0001;

    /// What the callbacks need. Out-of-context hooks call back on the thread
    /// that set them, while it pumps messages, so a thread-local is enough.
    struct State {
        pid: u32,
        tag: String,
        pos: TitlePosition,
        taskbar: Option<Taskbar>,
        /// Windows that already carry the taskbar properties: set once each,
        /// not on every title change. A cell of its own, borrowed only
        /// briefly: a cross-process send can let the next event in before it
        /// returns, and the state must stay shareable across that.
        grouped: RefCell<HashSet<usize>>,
        /// The venv's icon for the windows' title bars, taskbar buttons and
        /// Alt+Tab.
        icons: Option<Icons>,
    }

    /// The venv's `.ico`, loaded at each size a window asks for and kept for
    /// the watcher's life. Icon handles are valid in every process of the
    /// session, so LabVIEW's windows can show ours — for as long as this
    /// process lives, which is as long as that LabVIEW does.
    struct Icons {
        file: Vec<u16>,
        loaded: RefCell<HashMap<i32, Handle>>,
    }

    impl Icons {
        fn new(path: &str) -> Icons {
            Icons {
                file: path.encode_utf16().chain(std::iter::once(0)).collect(),
                loaded: RefCell::new(HashMap::new()),
            }
        }

        fn at(&self, size: i32) -> Handle {
            *self.loaded.borrow_mut().entry(size).or_insert_with(|| {
                // SAFETY: a NUL-terminated path; a null result just means no icon.
                unsafe {
                    LoadImageW(
                        std::ptr::null_mut(),
                        self.file.as_ptr(),
                        IMAGE_ICON,
                        size,
                        size,
                        LR_LOADFROMFILE,
                    )
                }
            })
        }

        /// Give `hwnd` the venv's small and big icon at its DPI, unless it has
        /// them already: LabVIEW may put its own back, and this runs on every
        /// title change, which is when it would.
        fn apply(&self, hwnd: Hwnd) {
            // SAFETY: no pointers.
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            for (which, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
                // SAFETY: no pointers.
                let size = unsafe { GetSystemMetricsForDpi(metric, dpi) };
                let icon = self.at(size);
                if icon.is_null() {
                    continue;
                }
                let mut current = 0usize;
                // SAFETY: `current` is a valid usize; the call returns before
                // its timeout ends.
                let asked = unsafe {
                    SendMessageTimeoutW(
                        hwnd,
                        WM_GETICON,
                        which,
                        0,
                        SMTO_ABORTIFHUNG,
                        SET_TIMEOUT_MS,
                        &mut current,
                    )
                };
                if asked != 0 && current == icon as usize {
                    continue;
                }
                let mut ignored = 0usize;
                // SAFETY: as above; the icon handle outlives the window's use
                // of it (see the type's comment).
                unsafe {
                    SendMessageTimeoutW(
                        hwnd,
                        WM_SETICON,
                        which,
                        icon as isize,
                        SMTO_ABORTIFHUNG,
                        SET_TIMEOUT_MS,
                        &mut ignored,
                    )
                };
            }
        }
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub fn watch(pid: u32, tag: &str, pos: TitlePosition, taskbar: Option<Taskbar>) -> Result<()> {
        if pos == TitlePosition::Off && taskbar.is_none() {
            return Ok(());
        }
        let Some(process) = Process::open(pid) else {
            bail!("no process {pid} to watch")
        };
        if taskbar.is_some() {
            taskbar::init();
        }
        let icons = taskbar
            .as_ref()
            .filter(|t| t.icon.to_ascii_lowercase().ends_with(".ico"))
            .map(|t| Icons::new(&t.icon));
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                pid,
                tag: tag.to_string(),
                pos,
                taskbar,
                grouped: RefCell::new(HashSet::new()),
                icons,
            })
        });
        let (Some(_shown), Some(_renamed)) = (
            EventHook::new(EVENT_OBJECT_SHOW, pid),
            EventHook::new(EVENT_OBJECT_NAMECHANGE, pid),
        ) else {
            bail!("could not hook window events of process {pid}");
        };
        // Windows from before the hooks; later ones arrive as events.
        enum_windows();
        // The hooks call back from inside `pump`; it returns when LabVIEW exits.
        process.pump_until_exit();
        Ok(())
    }

    /// The watched LabVIEW's process, closed when dropped.
    struct Process(Handle);

    impl Process {
        fn open(pid: u32) -> Option<Process> {
            // SAFETY: no pointers in; the handle is checked and owned from here.
            let h = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            (!h.is_null()).then_some(Process(h))
        }

        /// Dispatch this thread's messages (and with them the hook
        /// callbacks) until the process exits.
        fn pump_until_exit(&self) {
            // SAFETY: an all-zero MSG is a valid value of a plain C struct.
            let mut msg: Msg = unsafe { std::mem::zeroed() };
            loop {
                // SAFETY: one handle, owned by `self`, passed with count 1.
                let woke =
                    unsafe { MsgWaitForMultipleObjects(1, &self.0, 0, INFINITE, QS_ALLINPUT) };
                if woke == WAIT_OBJECT_0 || woke == WAIT_FAILED {
                    return;
                }
                // SAFETY: `msg` is a valid, writable MSG for each call.
                while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
                {
                    unsafe {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: a handle this value owns, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// An out-of-context hook for one event of one process, removed when dropped.
    struct EventHook(Hook);

    impl EventHook {
        fn new(event: u32, pid: u32) -> Option<EventHook> {
            // SAFETY: `on_event` matches WINEVENTPROC; no module for an
            // out-of-context hook.
            let h = unsafe {
                SetWinEventHook(
                    event,
                    event,
                    std::ptr::null_mut(),
                    on_event,
                    pid,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            (!h.is_null()).then_some(EventHook(h))
        }
    }

    impl Drop for EventHook {
        fn drop(&mut self) {
            // SAFETY: a hook this value owns, removed once.
            unsafe { UnhookWinEvent(self.0) };
        }
    }

    fn enum_windows() {
        // SAFETY: `on_window` matches WNDENUMPROC and uses no lparam.
        unsafe { EnumWindows(on_window, 0) };
    }

    unsafe extern "system" fn on_event(
        _: Hook,
        _event: u32,
        hwnd: Hwnd,
        object: i32,
        child: i32,
        _: u32,
        _: u32,
    ) {
        if object == OBJID_WINDOW && child == CHILDID_SELF && !hwnd.is_null() {
            tag_window(hwnd);
        }
    }

    unsafe extern "system" fn on_window(hwnd: Hwnd, _: isize) -> i32 {
        tag_window(hwnd);
        1
    }

    /// Tag `hwnd` if it is one of the watched LabVIEW's own visible windows.
    fn tag_window(hwnd: Hwnd) {
        STATE.with(|s| {
            let s = s.borrow();
            let Some(state) = s.as_ref() else { return };
            if window_pid(hwnd) != state.pid || !is_visible(hwnd) {
                return;
            }
            let class = class_name(hwnd);
            let app = ex_style(hwnd) & WS_EX_APPWINDOW != 0;
            if !(class == "LVFrame" || (class == "LVDChild" && app)) {
                return;
            }
            if let Some(t) = &state.taskbar
                && state.grouped.borrow_mut().insert(hwnd as usize)
            {
                // Best effort: a window that refuses keeps LabVIEW's group.
                let _ = taskbar::apply(hwnd, t);
            }
            if let Some(icons) = state.icons.as_ref() {
                icons.apply(hwnd);
            }
            let title = window_text(hwnd);
            let Some(want) = retitle(&title, &state.tag, state.pos) else {
                return;
            };
            // Busy or hung: the next title change brings it back here.
            let _ = set_window_text(hwnd, &want);
        });
    }

    // Window queries. A window handle is an opaque value that Windows checks
    // on every call: a stale or foreign one makes the call fail, never touch
    // memory, so these are safe to call with any `hwnd`.

    fn window_pid(hwnd: Hwnd) -> u32 {
        let mut pid = 0u32;
        // SAFETY: `pid` is a valid u32 to write to.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn is_visible(hwnd: Hwnd) -> bool {
        // SAFETY: no pointers.
        unsafe { IsWindowVisible(hwnd) != 0 }
    }

    fn ex_style(hwnd: Hwnd) -> u32 {
        // SAFETY: no pointers.
        unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 }
    }

    fn class_name(hwnd: Hwnd) -> String {
        let mut buf = [0u16; 64];
        // SAFETY: the buffer's length travels with it; the result is at most that.
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.clamp(0, buf.len() as i32) as usize])
    }

    /// The whole title, however long: a cut-off read would hide a tag at the
    /// end, and retitling that would cut LabVIEW's title and loop.
    fn window_text(hwnd: Hwnd) -> String {
        // SAFETY: no pointers.
        let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        // SAFETY: as in `class_name`.
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.clamp(0, buf.len() as i32) as usize])
    }

    /// Retitle `hwnd`, waiting at most `SET_TIMEOUT_MS` for its owner, so a
    /// busy LabVIEW never blocks the watcher. False when it did not answer.
    fn set_window_text(hwnd: Hwnd, text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut result = 0usize;
        // SAFETY: `wide` is NUL-terminated and outlives the call, which
        // returns before the timeout ends; `result` is a valid usize.
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_SETTEXT,
                0,
                wide.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                SET_TIMEOUT_MS,
                &mut result,
            )
        };
        ok != 0
    }

    /// The taskbar's per-window properties, through the window's property
    /// store. The store lives in window properties, which another process
    /// may set; nothing of it runs inside LabVIEW.
    mod taskbar {
        use super::{Hwnd, Taskbar};
        use std::ffi::c_void;

        #[repr(C)]
        struct Guid(u32, u16, u16, [u8; 8]);

        #[repr(C)]
        struct PropertyKey {
            fmtid: Guid,
            pid: u32,
        }

        /// PROPVARIANT holding a VT_LPWSTR: the type, three reserved words,
        /// then the pointer, 24 bytes in all on x64.
        #[repr(C)]
        struct PropVariant {
            vt: u16,
            reserved: [u16; 3],
            ptr: *const u16,
            pad: usize,
        }

        #[repr(C)]
        struct PropertyStoreVtbl {
            query_interface: usize,
            add_ref: usize,
            release: unsafe extern "system" fn(*mut PropertyStore) -> u32,
            get_count: usize,
            get_at: usize,
            get_value: usize,
            set_value: unsafe extern "system" fn(
                *mut PropertyStore,
                *const PropertyKey,
                *const PropVariant,
            ) -> i32,
            commit: unsafe extern "system" fn(*mut PropertyStore) -> i32,
        }

        #[repr(C)]
        struct PropertyStore {
            vtbl: *const PropertyStoreVtbl,
        }

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHGetPropertyStoreForWindow(
                hwnd: Hwnd,
                iid: *const Guid,
                out: *mut *mut c_void,
            ) -> i32;
        }

        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
        }

        /// COM on this thread, which the property store is: once, before
        /// the first `apply`. Left initialised until the process exits.
        pub fn init() {
            const COINIT_APARTMENTTHREADED: u32 = 0x2;
            // SAFETY: no pointers; a failure only means COM was set up already.
            unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
        }

        const IID_IPROPERTYSTORE: Guid = Guid(
            0x886D_8EEB,
            0x8CF2,
            0x4446,
            [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99],
        );
        /// PKEY_AppUserModel_*: one format id, these property ids.
        const APP_USER_MODEL: (u32, u16, u16, [u8; 8]) = (
            0x9F4C_2855,
            0x9F79,
            0x4B39,
            [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
        );
        const RELAUNCH_COMMAND: u32 = 2;
        const RELAUNCH_ICON: u32 = 3;
        const RELAUNCH_NAME: u32 = 4;
        const ID: u32 = 5;
        const VT_LPWSTR: u16 = 31;

        fn key(pid: u32) -> PropertyKey {
            let (a, b, c, d) = APP_USER_MODEL;
            PropertyKey {
                fmtid: Guid(a, b, c, d),
                pid,
            }
        }

        /// Set the relaunch properties, then the id: the taskbar reads the
        /// relaunch ones when the id arrives.
        pub fn apply(hwnd: Hwnd, t: &Taskbar) -> Result<(), String> {
            let mut store: *mut c_void = std::ptr::null_mut();
            // SAFETY: `store` receives an IPropertyStore pointer on success,
            // released below; the IID is the one asked for.
            let hr = unsafe { SHGetPropertyStoreForWindow(hwnd, &IID_IPROPERTYSTORE, &mut store) };
            if hr < 0 || store.is_null() {
                return Err(format!("SHGetPropertyStoreForWindow: {hr:#010x}"));
            }
            let store = store as *mut PropertyStore;
            let props = [
                (RELAUNCH_COMMAND, t.relaunch.as_str()),
                (RELAUNCH_ICON, &format!("{},0", t.icon)),
                (RELAUNCH_NAME, t.name.as_str()),
                (ID, t.id.as_str()),
            ];
            let mut result = Ok(());
            for (pid, value) in props {
                let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
                let pv = PropVariant {
                    vt: VT_LPWSTR,
                    reserved: [0; 3],
                    ptr: wide.as_ptr(),
                    pad: 0,
                };
                // SAFETY: a live IPropertyStore; the store copies the value
                // before returning, while `wide` is still alive.
                let hr = unsafe { ((*(*store).vtbl).set_value)(store, &key(pid), &pv) };
                if hr < 0 {
                    result = Err(format!("SetValue({pid}): {hr:#010x}"));
                    break;
                }
            }
            // SAFETY: the store obtained above, committed and released once.
            unsafe {
                if result.is_ok() {
                    ((*(*store).vtbl).commit)(store);
                }
                ((*(*store).vtbl).release)(store);
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TitlePosition::{End, Off, Start};

    const TAG: &str = "[venv-test4]";

    #[test]
    fn end_goes_after_the_title_and_before_labviews_star() {
        assert_eq!(
            retitle("x.vi Front Panel on p.lvproj/My Computer", TAG, End).as_deref(),
            Some("x.vi Front Panel on p.lvproj/My Computer [venv-test4]")
        );
        assert_eq!(
            retitle("x.vi Front Panel on p.lvproj/My Computer *", TAG, End).as_deref(),
            Some("x.vi Front Panel on p.lvproj/My Computer [venv-test4] *")
        );
        // The project's `*` sits mid-title: nothing special about the end.
        assert_eq!(
            retitle("p.lvproj * - Project Explorer", TAG, End).as_deref(),
            Some("p.lvproj * - Project Explorer [venv-test4]")
        );
    }

    #[test]
    fn start_goes_in_front() {
        assert_eq!(
            retitle("LabVIEW", TAG, Start).as_deref(),
            Some("[venv-test4] LabVIEW")
        );
        assert_eq!(
            retitle("x.vi Block Diagram *", TAG, Start).as_deref(),
            Some("[venv-test4] x.vi Block Diagram *")
        );
    }

    #[test]
    fn a_tagged_title_is_left_alone_which_ends_the_rename_loop() {
        assert_eq!(retitle("LabVIEW [venv-test4]", TAG, End), None);
        assert_eq!(retitle("x.vi [venv-test4] *", TAG, End), None);
        assert_eq!(retitle("[venv-test4] LabVIEW", TAG, Start), None);
    }

    #[test]
    fn a_tag_in_the_wrong_place_moves() {
        assert_eq!(
            retitle("[venv-test4] x.vi *", TAG, End).as_deref(),
            Some("x.vi [venv-test4] *")
        );
        assert_eq!(
            retitle("x.vi [venv-test4] *", TAG, Start).as_deref(),
            Some("[venv-test4] x.vi *")
        );
        // The form that breaks LabVIEW's `*` is repaired, not kept.
        assert_eq!(
            retitle("x.vi * [venv-test4]", TAG, End).as_deref(),
            Some("x.vi [venv-test4] *")
        );
    }

    #[test]
    fn the_tags_text_inside_labviews_own_title_is_kept() {
        assert_eq!(
            retitle("[venv-test4] notes.vi Front Panel", TAG, End).as_deref(),
            Some("notes.vi Front Panel [venv-test4]"),
            "a leading tag is taken for ours, the rest stays"
        );
        assert_eq!(
            retitle("a [venv-test4] b.vi Front Panel *", TAG, End).as_deref(),
            Some("a [venv-test4] b.vi Front Panel [venv-test4] *")
        );
        assert_eq!(retitle("a [venv-test4] b.vi [venv-test4]", TAG, End), None);
    }

    #[test]
    fn long_titles_are_tagged_whole() {
        let long = format!(
            "{}.vi Front Panel on p.lvproj/My Computer *",
            "x".repeat(2000)
        );
        let want = retitle(&long, TAG, End).unwrap();
        assert!(want.ends_with(" [venv-test4] *") && want.len() == long.len() + TAG.len() + 1);
        assert_eq!(retitle(&want, TAG, End), None);
    }

    #[test]
    fn nothing_to_do_for_empty_titles_or_off() {
        assert_eq!(retitle("", TAG, End), None);
        assert_eq!(retitle(TAG, TAG, End), None);
        assert_eq!(retitle("LabVIEW", TAG, Off), None);
    }
}
