//! The window process's end of the line to the tray process (`ipc.rs`).
//!
//! A thread with a hidden window of its own receives what the tray process
//! sends: the devices after every poll, and the clicks on the tray icons,
//! which it turns into showing or hiding the main window exactly as the
//! tray thread used to when both lived in one process. Once the window has
//! been hidden for `LINGER` (and no box is half filled in), the process
//! closes itself: the window is not worth its memory while nobody looks.
//! It also closes when the tray process goes, so Quit leaves nothing behind.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, GetWindowThreadProcessId,
    KillTimer, MSG, PostMessageW, RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_COPYDATA, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::flyout::{self, Mode, TrayClick};
use crate::ipc::{self, Start, ToTray, ToWindow, hwnd};
use crate::monitor::{SharedState, lock};

const CLASS: PCWSTR = w!("BatteryTrayWindowLink");
/// Something arrived that needs handling.
const WM_INBOX: u32 = WM_APP + 1;
/// The window was hidden: the linger starts.
const WM_HIDDEN: u32 = WM_APP + 2;
/// The tray process has gone.
const WM_TRAY_GONE: u32 = WM_APP + 3;
const LINGER_TIMER: usize = 1;
const EXIT_TIMER: usize = 2;
const HIDE_TIMER: usize = 3;
/// How often a window started hidden checks whether eframe has shown it yet.
const HIDE_CHECK: Duration = Duration::from_millis(50);

/// How long a hidden window stays ready before its process ends. A click
/// within this reopens it at once; after it, the window starts afresh
/// (about a quarter of a second).
pub const LINGER: Duration = Duration::from_secs(20);
/// After asking eframe to close, how long before ending the process anyway.
const EXIT_GRACE: Duration = Duration::from_secs(3);

static QUITTING: AtomicBool = AtomicBool::new(false);
/// A box is open (feedback, unlock) whose text would be lost.
static BUSY: AtomicBool = AtomicBool::new(false);
/// Set once the first state from the tray has arrived.
static FIRST_STATE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

struct State {
    shared: SharedState,
    tray: isize,
    /// The main window and egui, once eframe has made them.
    main: Option<(isize, egui::Context)>,
    /// Commands that arrived before the main window existed.
    inbox: Vec<ToWindow>,
    /// The history.json write last read.
    history_rev: Option<u64>,
    /// Times a window started hidden was checked for eframe showing it.
    hide_checks: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Whether the process is on its way out, so a close request must not be
/// turned into hiding.
pub fn quitting() -> bool {
    QUITTING.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub struct Link {
    tray: isize,
    window: isize,
}

impl Link {
    /// Finds the tray process and says hello. `None` if there is none.
    pub fn connect(shared: SharedState, tray: isize) -> Option<Link> {
        let (sender, receiver) = channel();
        std::thread::spawn(move || unsafe {
            let window = create_window();
            let _ = sender.send(window.map(|w| w.0 as isize));
            if window.is_none() {
                return;
            }
            STATE.with_borrow_mut(|state| {
                *state = Some(State { shared, tray, main: None, inbox: Vec::new(), history_rev: None, hide_checks: 0 });
            });
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        let window = receiver.recv().ok()??;
        watch_tray(tray, window);
        ToTray::Hello(window).post(tray).then_some(Link { tray, window })
    }

    /// The main window and egui, once eframe has drawn the first frame;
    /// shows the flyout if that is how the process started, and handles
    /// whatever arrived before.
    pub fn attach(&self, main: isize, ctx: egui::Context, start: Start) {
        // The state lives on the link thread: hand them over there.
        let boxed = Box::new((main, ctx, start));
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.window)), WM_ATTACH, WPARAM(0), LPARAM(Box::into_raw(boxed) as isize));
        }
    }

    /// Waits a moment for the tray's first state, so the first frame
    /// already shows the devices.
    pub fn wait_for_state(&self, timeout: Duration) {
        let (lock, arrived) = &FIRST_STATE;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = arrived.wait_timeout_while(guard, timeout, |got| !*got);
    }

    /// settings.json changed; `refresh` also polls now.
    pub fn settings_changed(&self, refresh: bool) {
        ToTray::Settings { refresh }.post(self.tray);
    }

    /// Poll now.
    pub fn refresh(&self) {
        ToTray::Refresh.post(self.tray);
    }

    /// The window was hidden: the process ends after `LINGER` unless it is
    /// shown again.
    pub fn hidden(&self) {
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.window)), WM_HIDDEN, WPARAM(0), LPARAM(0));
        }
    }

    /// Whether a box with unsent text is open; such a process stays.
    pub fn set_busy(&self, busy: bool) {
        BUSY.store(busy, Ordering::Relaxed);
    }
}

/// Hands the main window and egui to the link thread (`lparam`: a box).
const WM_ATTACH: u32 = WM_APP + 4;

/// Ends this process when the tray process ends: a thread that waits on it
/// and wakes for nothing else.
fn watch_tray(tray: isize, window: isize) {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd(tray), Some(&mut pid)) };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else { return };
    let process = process.0 as isize;
    std::thread::spawn(move || unsafe {
        let handle = HANDLE(process as *mut core::ffi::c_void);
        if WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
            let _ = PostMessageW(Some(hwnd(window)), WM_TRAY_GONE, WPARAM(0), LPARAM(0));
        }
        let _ = CloseHandle(handle);
    });
}

/// The tray's hidden window, if the tray process is running.
pub fn find_tray() -> Option<isize> {
    unsafe {
        FindWindowW(crate::tray::CLASS, PCWSTR::null()).ok().filter(|w| !w.is_invalid()).map(|w| w.0 as isize)
    }
}

unsafe fn create_window() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&class);
        // Never shown; a top-level window rather than a message-only one,
        // so the light/dark broadcast reaches it.
        CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(instance.into()), None).ok()
    }
}

/// A new state from the tray: the devices, and the history if it was
/// written since it was last read.
fn take_state(snapshot: ipc::Snapshot) {
    let Some((shared, rev)) = STATE.with_borrow(|s| s.as_ref().map(|s| (s.shared.clone(), s.history_rev))) else { return };
    let new_rev = snapshot.history;
    let history = (rev != Some(new_rev)).then(crate::history::load);
    {
        let mut shared = lock(&shared);
        shared.polled = snapshot.polled;
        shared.devices = snapshot.into_devices();
        if let Some(history) = history {
            shared.history = history;
        }
    }
    let ctx = STATE.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        s.history_rev = Some(new_rev);
        s.main.as_ref().map(|(_, ctx)| ctx.clone())
    });
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
    let (lock, arrived) = &FIRST_STATE;
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
    arrived.notify_all();
}

/// Handles the commands waiting, once there is a window to act on.
fn handle_inbox(link: HWND) {
    let Some((shared, main, ctx, inbox)) = STATE.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        let (main, ctx) = s.main.clone()?;
        Some((s.shared.clone(), main, ctx, std::mem::take(&mut s.inbox)))
    }) else {
        return;
    };
    for message in inbox {
        match message {
            ToWindow::Click => tray_click(&shared, main),
            ToWindow::Open => open_pinned(&shared, main),
            ToWindow::Quit => quit(link),
            ToWindow::State(_) => {}
        }
    }
    ctx.request_repaint();
    linger(link, &shared);
}

/// A left click on a tray icon: open or close the flyout.
fn tray_click(shared: &SharedState, main: isize) {
    let (click, height) = {
        let mut shared = lock(shared);
        let click = shared.window.on_tray_click(Instant::now());
        match click {
            TrayClick::Flyout => {
                shared.window.mode = Mode::Flyout;
                shared.window.shown += 1;
            }
            TrayClick::Hide => shared.window.mode = Mode::Hidden,
            _ => {}
        }
        (click, shared.window.height)
    };
    // The lock is free again before Windows calls into the main window.
    match click {
        TrayClick::Flyout => flyout::show_flyout(main, height),
        TrayClick::Hide => flyout::hide(main),
        TrayClick::Raise => flyout::raise(main),
        TrayClick::Nothing => {}
    }
}

/// The tray menu's Open, or the app started again: the window, pinned.
fn open_pinned(shared: &SharedState, main: isize) {
    let was = {
        let mut shared = lock(shared);
        let was = shared.window.mode;
        shared.window.mode = Mode::Pinned;
        shared.window.shown += 1;
        was
    };
    match was {
        Mode::Pinned if flyout::visible(main) => flyout::raise(main),
        Mode::Flyout => flyout::show_pinned(main, false),
        _ => flyout::show_pinned(main, true),
    }
}

/// Starts or stops the countdown to closing, by whether the window is
/// hidden.
fn linger(link: HWND, shared: &SharedState) {
    let hidden = lock(shared).window.mode == Mode::Hidden;
    unsafe {
        if hidden {
            SetTimer(Some(link), LINGER_TIMER, LINGER.as_millis() as u32, None);
        } else {
            let _ = KillTimer(Some(link), LINGER_TIMER);
        }
    }
}

/// Closes the window process: eframe closes the window and ends it; if it
/// does not within `EXIT_GRACE`, the process ends anyway.
fn quit(link: HWND) {
    if QUITTING.swap(true, Ordering::Relaxed) {
        return;
    }
    let found = STATE.with_borrow(|s| s.as_ref().map(|s| (s.tray, s.main.as_ref().map(|(_, ctx)| ctx.clone()))));
    if let Some((tray, ctx)) = found {
        ToTray::Bye.post(tray);
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
            unsafe { SetTimer(Some(link), EXIT_TIMER, EXIT_GRACE.as_millis() as u32, None) };
            return;
        }
    }
    std::process::exit(0);
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COPYDATA => {
            // Only copied here; handled once the sender has its answer.
            let Some((tag, bytes)) = (unsafe { ipc::received(lparam) }) else { return LRESULT(0) };
            let Some(decoded) = ipc::decode(tag, &bytes) else { return LRESULT(0) };
            match decoded {
                ToWindow::State(snapshot) => take_state(snapshot),
                command => {
                    STATE.with_borrow_mut(|s| {
                        if let Some(s) = s.as_mut() {
                            s.inbox.push(command);
                        }
                    });
                    unsafe {
                        let _ = PostMessageW(Some(window), WM_INBOX, WPARAM(0), LPARAM(0));
                    }
                }
            }
            LRESULT(1)
        }
        WM_INBOX => {
            handle_inbox(window);
            LRESULT(0)
        }
        WM_ATTACH => {
            let (main, ctx, start) = *unsafe { Box::from_raw(lparam.0 as *mut (isize, egui::Context, Start)) };
            STATE.with_borrow_mut(|s| {
                if let Some(s) = s.as_mut() {
                    s.main = Some((main, ctx));
                    // Started by a click: that click, before any that came since.
                    if start == Start::Flyout {
                        s.inbox.insert(0, ToWindow::Click);
                    }
                }
            });
            if start == Start::Hidden {
                // eframe shows the window (off-screen) once it is drawn;
                // out of sight it goes, until the click.
                unsafe { SetTimer(Some(window), HIDE_TIMER, HIDE_CHECK.as_millis() as u32, None) };
            }
            handle_inbox(window);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HIDE_TIMER => {
            let found = STATE.with_borrow_mut(|s| {
                let s = s.as_mut()?;
                s.hide_checks += 1;
                Some((s.shared.clone(), s.main.as_ref()?.0, s.hide_checks))
            });
            let Some((shared, main, checks)) = found else { return LRESULT(0) };
            let hidden = lock(&shared).window.mode == Mode::Hidden;
            if !hidden || flyout::visible(main) || checks > 50 {
                unsafe {
                    let _ = KillTimer(Some(window), HIDE_TIMER);
                }
                if hidden {
                    flyout::hide(main);
                    linger(window, &shared);
                }
            }
            LRESULT(0)
        }
        WM_HIDDEN => {
            if let Some(shared) = STATE.with_borrow(|s| s.as_ref().map(|s| s.shared.clone())) {
                linger(window, &shared);
            }
            LRESULT(0)
        }
        WM_TRAY_GONE => {
            quit(window);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == LINGER_TIMER => {
            unsafe {
                let _ = KillTimer(Some(window), LINGER_TIMER);
            }
            let Some(shared) = STATE.with_borrow(|s| s.as_ref().map(|s| s.shared.clone())) else { return LRESULT(0) };
            if lock(&shared).window.mode != Mode::Hidden {
                return LRESULT(0);
            }
            if BUSY.load(Ordering::Relaxed) {
                // A box is half filled in: ask again later.
                linger(window, &shared);
            } else {
                quit(window);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == EXIT_TIMER => std::process::exit(0),
        WM_SETTINGCHANGE => {
            // "ImmersiveColorSet": the light/dark choice.
            let area = PCWSTR(lparam.0 as *const u16);
            if !area.is_null() && unsafe { area.to_string() }.is_ok_and(|a| a == "ImmersiveColorSet")
                && let Some(ctx) = STATE.with_borrow(|s| s.as_ref().and_then(|s| s.main.as_ref().map(|(_, ctx)| ctx.clone())))
            {
                crate::theme::follow_windows(&ctx);
                ctx.request_repaint();
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
