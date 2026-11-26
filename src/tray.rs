//! The resident process: what runs all day. Tray icons, their tooltips, the
//! menu and notifications, the monitor thread that polls, and starting the
//! window process when the window is wanted. It has no window of its own
//! beyond a hidden one for messages, and no egui: the window (`app.rs`) is
//! the same exe started with `--ui`, which ends itself soon after it is
//! hidden (`link.rs`, `ipc.rs`).
//!
//! Plain Win32 through the `windows` crate: `Shell_NotifyIconW` for the
//! icons and notifications (balloons, which Windows 10 and 11 show as
//! toasts without the app needing an AppUserModelID or a Start-menu
//! shortcut), a popup menu on right-click.
//!
//! Each device with a tray icon has its own icon id, so Windows remembers
//! whether the user dragged it onto the taskbar. The first icon is always
//! id 1, which is the one icon v0.1.4 had, so an icon the user pinned then
//! stays pinned; the others take an id made from the device's id.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIIF_WARNING, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowThreadProcessId,
    HICON, IsWindow, MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_LBUTTONUP, WM_NULL, WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE, WNDCLASSW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::devices::{self, Device, Notice, NoticeKind, Rules, State as DeviceState};
use crate::i18n::{Strings, fill};
use crate::icon::{self, Look, Spec, Tone};
use crate::ipc::{self, Start, ToTray, ToWindow};
use crate::monitor::{Monitor, Shared, SharedState, Wake, lock};
use crate::signals::Event;
use crate::theme;

/// Also how a second copy of the app, and the window process, find it.
pub const CLASS: PCWSTR = w!("BatteryTrayHiddenWindow");
/// From a tray icon; `wparam` is the icon's id, `lparam` the mouse message.
const WM_TRAY: u32 = WM_APP + 1;
/// Show the main window, pinned.
const WM_OPEN: u32 = WM_APP + 2;
/// The shared state changed: redraw the icons, show waiting notifications.
const WM_REFRESH: u32 = WM_APP + 3;
/// The first icon's id, the same as v0.1.4's only icon.
const FIRST_ICON: u32 = 1;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;
/// Set to anything to keep notifications off the screen (for screenshots).
const QUIET_ENV: &str = "BT_QUIET";

/// An icon in the tray now.
struct Shown {
    planned: Planned,
    size: usize,
    icon: HICON,
}

struct State {
    shared: SharedState,
    monitor: Monitor,
    icons: Vec<Shown>,
    taskbar_created: u32,
    light_taskbar: bool,
    window: WindowProcess,
    signals: crate::signals::Signals,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

/// What to do with a message for the window process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Send,
    /// It is starting: hand it over once it says hello.
    Wait,
    Start(Start),
    Drop,
}

/// Where a message for the window process goes, by whether the process is
/// running and has said hello. A state is only worth sending to a window
/// that is there (one that is starting gets the state with its hello); a
/// click or Open starts one.
fn route(running: bool, linked: bool, message: &ToWindow) -> Route {
    match (running, linked, message) {
        (true, true, _) => Route::Send,
        (true, false, ToWindow::State(_)) => Route::Drop,
        (true, false, _) => Route::Wait,
        (false, _, ToWindow::Click) => Route::Start(Start::Flyout),
        (false, _, ToWindow::Open) => Route::Start(Start::Pinned),
        (false, _, _) => Route::Drop,
    }
}

/// The window process, while there is one.
#[derive(Default)]
struct WindowProcess {
    child: Option<std::process::Child>,
    /// Its link window, once it has said hello.
    link: Option<isize>,
    waiting: Vec<ToWindow>,
}

impl WindowProcess {
    fn running(&mut self) -> bool {
        let alive = match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            // One this process did not start (`battery_tray.exe --ui`).
            None => self.link.is_some_and(|l| unsafe { IsWindow(Some(hwnd(l))).as_bool() }),
        };
        if !alive {
            *self = WindowProcess::default();
        }
        alive
    }

    /// The process id, to let it take the foreground.
    fn pid(&self) -> Option<u32> {
        if let Some(child) = &self.child {
            return Some(child.id());
        }
        let mut pid = 0u32;
        self.link.map(|l| unsafe { GetWindowThreadProcessId(hwnd(l), Some(&mut pid)) });
        (pid != 0).then_some(pid)
    }

    fn start(&mut self, start: Start) {
        let Ok(exe) = std::env::current_exe() else { return };
        if let Ok(child) = std::process::Command::new(exe).args([ipc::UI_ARG, start.arg()]).spawn() {
            // The click gave this process the right to bring a window to the
            // front; pass it on.
            unsafe {
                let _ = AllowSetForegroundWindow(child.id());
            }
            self.child = Some(child);
        }
    }

    fn deliver(&mut self, message: ToWindow) {
        let route = route(self.running(), self.link.is_some(), &message);
        match route {
            Route::Send => {
                if matches!(message, ToWindow::Click | ToWindow::Open)
                    && let Some(pid) = self.pid()
                {
                    unsafe {
                        let _ = AllowSetForegroundWindow(pid);
                    }
                }
                if let Some(link) = self.link
                    && !ipc::send(link, &message)
                {
                    // Gone or hung: forget the line; it is started afresh
                    // once the process has ended.
                    self.link = None;
                }
            }
            Route::Wait => self.waiting.push(message),
            Route::Start(start) => self.start(start),
            Route::Drop => {}
        }
    }

    /// The window process said hello: its state, then what was waiting.
    fn hello(&mut self, link: isize, state: ToWindow) {
        self.link = Some(link);
        if !ipc::send(link, &state) {
            self.link = None;
            return;
        }
        for message in std::mem::take(&mut self.waiting) {
            self.deliver(message);
        }
    }
}

/// Whether the app is already running; `open` also asks it to show its window.
pub fn running_instance(open: bool) -> bool {
    unsafe {
        match FindWindowW(CLASS, PCWSTR::null()) {
            Ok(window) if !window.is_invalid() => {
                if open {
                    let _ = PostMessageW(Some(window), WM_OPEN, WPARAM(0), LPARAM(0));
                }
                true
            }
            _ => false,
        }
    }
}

/// Runs the resident process on this thread until Quit. `open`: show the
/// pinned window straight away (started from the Start menu). False if
/// Windows would not make the hidden window, in which case the caller runs
/// the window in this process instead, as an ordinary window.
pub fn run(open: bool) -> bool {
    // Without eframe in this process nothing else says so, and the tray
    // icons must be drawn at the real size, not scaled up blurred.
    unsafe {
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let settings = crate::settings::load();
    let licensed = crate::license::verify(&settings.license_key).is_some();
    let shared: SharedState = Arc::new(Mutex::new(Shared::new(
        devices::load(),
        crate::history::load(),
        settings.rules(licensed),
        settings.lang(),
    )));
    let Some(window) = (unsafe { create_window() }) else { return false };
    let raw = window.0 as isize;
    let monitor = Monitor::start(crate::source::pick(), shared.clone(), move || unsafe {
        let _ = PostMessageW(Some(hwnd(raw)), WM_REFRESH, WPARAM(0), LPARAM(0));
    });
    STATE.with_borrow_mut(|state| {
        *state = Some(State {
            shared,
            monitor,
            icons: Vec::new(),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            light_taskbar: theme::taskbar_light(),
            window: WindowProcess::default(),
            signals: crate::signals::Signals::default(),
        });
    });
    // Polling waits for devices to change and backs off while nobody looks;
    // what is left runs in efficiency mode.
    crate::signals::register(window);
    crate::power::efficiency_mode(true);
    update(window, true);
    if open {
        to_window(ToWindow::Open);
    }
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}

/// Hands a message to the window process, starting it if need be.
fn to_window(message: ToWindow) {
    // Taken out of the state while it is in use: Windows may call
    // `window_proc` again while a message is on its way.
    let Some(mut process) = STATE.with_borrow_mut(|s| s.as_mut().map(|s| std::mem::take(&mut s.window))) else { return };
    process.deliver(message);
    put_back(process);
}

fn put_back(process: WindowProcess) {
    STATE.with_borrow_mut(|s| {
        if let Some(s) = s.as_mut() {
            s.window = process;
        }
    });
}

/// What the window process needs to show: the devices as they are now.
fn state_message() -> Option<ToWindow> {
    let shared = STATE.with_borrow(|s| s.as_ref().map(|s| s.shared.clone()))?;
    let shared = lock(&shared);
    Some(ToWindow::State(ipc::Snapshot::new(&shared.devices, shared.polled, shared.history_rev)))
}

/// A message from the window process.
fn from_window(window: HWND, message: ToTray) {
    match message {
        ToTray::Hello(link) => {
            // What the window shows should not be older than a regular poll.
            STATE.with_borrow(|s| s.as_ref().map(|s| s.monitor.wake(Wake::Looked)));
            let Some(state) = state_message() else { return };
            let Some(mut process) = STATE.with_borrow_mut(|s| s.as_mut().map(|s| std::mem::take(&mut s.window))) else { return };
            process.hello(link, state);
            put_back(process);
        }
        ToTray::Settings { refresh } => {
            // The window process saved them; the rules and language follow.
            let settings = crate::settings::load();
            let licensed = crate::license::verify(&settings.license_key).is_some();
            STATE.with_borrow(|s| {
                if let Some(s) = s.as_ref() {
                    let mut shared = lock(&s.shared);
                    shared.rules = settings.rules(licensed);
                    shared.lang = settings.lang();
                }
            });
            update(window, false);
            if refresh {
                STATE.with_borrow(|s| s.as_ref().map(|s| s.monitor.refresh()));
            }
        }
        ToTray::Refresh => {
            STATE.with_borrow(|s| s.as_ref().map(|s| s.monitor.refresh()));
        }
        ToTray::Bye => {
            // Forgotten at once: a click while it closes starts a new one.
            STATE.with_borrow_mut(|s| s.as_mut().map(|s| s.window = WindowProcess::default()));
        }
    }
}

unsafe fn create_window() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return None;
        }
        // Never shown. Not a message-only window: those do not get the
        // "TaskbarCreated" or settings broadcasts and cannot be found by
        // another copy.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("Battery Tray"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

/// Copies `text` into a fixed, NUL-terminated UTF-16 field, cutting it at a
/// character boundary if it is too long.
fn copy_text(field: &mut [u16], text: &str) {
    let mut at = 0;
    for c in text.chars() {
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units);
        if at + units.len() >= field.len() {
            break;
        }
        field[at..at + units.len()].copy_from_slice(units);
        at += units.len();
    }
    field[at] = 0;
}

fn icon_data(window: HWND, id: u32) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: id,
        ..Default::default()
    }
}

fn make_icon(spec: &Spec, size: usize) -> HICON {
    let mut pixels = icon::render(spec, size);
    // Windows wants blue first.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // With 32-bit colour the alpha channel decides transparency; the mask
    // is still required (one bit per pixel, rows padded to 16 bits).
    let mask = vec![0u8; size.div_ceil(16) * 2 * size];
    let side = size as i32;
    unsafe { CreateIcon(None, side, side, 1, 32, mask.as_ptr(), pixels.as_ptr()).unwrap_or_default() }
}

/// One tray icon as it should be.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub id: u32,
    /// The device it shows; `None` for the app's own icon.
    pub device: Option<String>,
    pub spec: Spec,
    pub tip: String,
}

/// FNV-1a: a stable number from a device id.
fn hash(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// A device's own icon id: from its id, never 1, distinct from `used`.
fn icon_id(device: &str, used: &[u32]) -> u32 {
    let mut id = 0x100 + hash(device) % 0x7fff_0000;
    while used.contains(&id) {
        id = id.wrapping_add(1).max(0x100);
    }
    id
}

/// How a device's icon looks right now.
fn look(device: &Device, rules: &Rules, light_taskbar: bool) -> Spec {
    let tone = |level: u8| {
        if device.is_charging() {
            Tone::Charging
        } else if rules.is_low(device, level) {
            Tone::Low
        } else {
            Tone::Normal
        }
    };
    let (look, tone) = match device.state() {
        DeviceState::Live(level) => (Look::Level(level, device.precision), tone(level)),
        // Picked automatically and never had a level: nothing to say yet.
        _ if !rules.tray.contains(&device.id) && device.last_level.is_none() => (Look::App, Tone::Normal),
        _ => (Look::NoLevel, Tone::Normal),
    };
    Spec { look, tone, light_taskbar }
}

/// The icons the tray should have: one per tray device, the first with id
/// 1; the app's own icon when there is no device at all. The first icon's
/// tooltip also lists the devices without an icon of their own.
pub fn plan(devices: &[Device], rules: &Rules, light_taskbar: bool, t: &Strings) -> Vec<Planned> {
    let shown = rules.tray_devices(devices);
    if shown.is_empty() {
        return vec![Planned {
            id: FIRST_ICON,
            device: None,
            spec: Spec { look: Look::App, tone: Tone::Normal, light_taskbar },
            tip: t.tray_none.to_owned(),
        }];
    }
    let mut planned: Vec<Planned> = Vec::new();
    for (index, device) in shown.iter().enumerate() {
        let used: Vec<u32> = planned.iter().map(|p| p.id).collect();
        let id = if index == 0 { FIRST_ICON } else { icon_id(&device.id, &used) };
        let mut lines = vec![devices::summary_line(device, t)];
        if index == 0 {
            lines.extend(
                devices
                    .iter()
                    .filter(|d| !shown.iter().any(|s| s.id == d.id))
                    .map(|d| devices::summary_line(d, t)),
            );
        }
        planned.push(Planned {
            id,
            device: Some(device.id.clone()),
            spec: look(device, rules, light_taskbar),
            tip: devices::cut_tooltip(&lines.join("\n")),
        });
    }
    planned
}

/// A notification balloon.
#[derive(Clone, Debug, PartialEq)]
pub struct Balloon {
    /// The icon it comes from.
    pub id: u32,
    pub title: String,
    pub body: String,
    pub warning: bool,
}

/// Notices as balloons: from the device's own icon if it has one, else the
/// first; several of one kind on one icon in one balloon.
pub fn balloons(notices: &[Notice], planned: &[Planned], t: &Strings) -> Vec<Balloon> {
    let mut groups: Vec<(u32, NoticeKind, Vec<&Notice>)> = Vec::new();
    for notice in notices {
        let id = planned
            .iter()
            .find(|p| p.device.as_deref() == Some(notice.id.as_str()))
            .map_or(FIRST_ICON, |p| p.id);
        match groups.iter_mut().find(|(i, k, _)| *i == id && *k == notice.kind) {
            Some((_, _, list)) => list.push(notice),
            None => groups.push((id, notice.kind, vec![notice])),
        }
    }
    groups
        .into_iter()
        .map(|(id, kind, list)| {
            let level = |n: &Notice| {
                n.level_text(t).unwrap_or_else(|| devices::transport_text(n.transport, t).to_owned())
            };
            let (title, body) = match kind {
                NoticeKind::Low => (
                    t.low_title.to_owned(),
                    list.iter()
                        .map(|n| devices::fill_level(t.low_body, n.level.unwrap_or(0), n.precision, t, &[("name", &n.name)]))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                NoticeKind::Full => (
                    t.full_title.to_owned(),
                    list.iter().map(|n| fill(t.full_body, &[("name", &n.name)])).collect::<Vec<_>>().join("\n"),
                ),
                NoticeKind::Connected if list.len() == 1 => (fill(t.connected_title, &[("name", &list[0].name)]), level(list[0])),
                NoticeKind::Connected => (
                    t.connected.to_owned(),
                    list.iter().map(|n| format!("{}: {}", n.name, level(n))).collect::<Vec<_>>().join("\n"),
                ),
            };
            Balloon { id, title, body, warning: kind == NoticeKind::Low }
        })
        .collect()
}

/// Brings the icons, tooltips and notifications in line with the shared
/// state. `add` puts every icon in the tray again (at start, and when
/// Explorer restarts).
fn update(window: HWND, add: bool) {
    // The state is only borrowed for a moment at a time: Windows may call
    // `window_proc` again while a tray call below is waiting for Explorer.
    let Some((shared, light)) = STATE.with_borrow(|state| state.as_ref().map(|s| (s.shared.clone(), s.light_taskbar))) else {
        return;
    };
    let (planned, notices, t) = {
        let mut shared = lock(&shared);
        let t = shared.lang.strings();
        let planned = plan(&shared.devices, &shared.rules, light, t);
        (planned, std::mem::take(&mut shared.pending), t)
    };
    let size = icon::tray_size();
    let old = STATE.with_borrow_mut(|state| state.as_mut().map(|s| std::mem::take(&mut s.icons))).unwrap_or_default();

    for gone in old.iter().filter(|o| !planned.iter().any(|p| p.id == o.planned.id)) {
        send(NIM_DELETE, &icon_data(window, gone.planned.id));
        unsafe {
            let _ = DestroyIcon(gone.icon);
        }
    }
    let mut now: Vec<Shown> = Vec::with_capacity(planned.len());
    for p in planned {
        let before = old.iter().find(|o| o.planned.id == p.id);
        let mut data = icon_data(window, p.id);
        data.uFlags = NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        copy_text(&mut data.szTip, &p.tip);
        let redraw = before.is_none_or(|b| b.planned.spec != p.spec || b.size != size);
        let icon = if redraw { make_icon(&p.spec, size) } else { before.map(|b| b.icon).unwrap_or_default() };
        if redraw || add {
            data.uFlags |= NIF_ICON;
            data.hIcon = icon;
        }
        let tip_changed = before.is_none_or(|b| b.planned.tip != p.tip);
        if add || before.is_none() {
            send(NIM_ADD, &data);
        } else if redraw || tip_changed {
            send(NIM_MODIFY, &data);
        }
        if redraw && let Some(before) = before {
            unsafe {
                let _ = DestroyIcon(before.icon);
            }
        }
        now.push(Shown { planned: p, size, icon });
    }

    if !notices.is_empty() && std::env::var_os(QUIET_ENV).is_none() {
        let planned: Vec<Planned> = now.iter().map(|s| s.planned.clone()).collect();
        for balloon in balloons(&notices, &planned, t) {
            let mut data = icon_data(window, balloon.id);
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = (if balloon.warning { NIIF_WARNING } else { NIIF_INFO }) | NIIF_RESPECT_QUIET_TIME;
            copy_text(&mut data.szInfoTitle, &balloon.title);
            copy_text(&mut data.szInfo, &balloon.body);
            send(NIM_MODIFY, &data);
        }
    }
    STATE.with_borrow_mut(|state| {
        if let Some(state) = state.as_mut() {
            state.icons = now;
        }
    });
}

fn send(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) {
    unsafe {
        let _ = Shell_NotifyIconW(message, data);
    }
}

/// The right-click menu: every device, then Open and Quit.
fn menu(window: HWND) {
    // Collected first: the menu runs its own message loop, during which
    // `update` may need the state again.
    let Some((lines, open, quit_text)) = STATE.with_borrow(|state| {
        let state = state.as_ref()?;
        let shared = lock(&state.shared);
        let t = shared.lang.strings();
        let lines: Vec<String> = shared.devices.iter().map(|d| devices::summary_line(d, t)).collect();
        Some((lines, t.tray_open, t.tray_quit))
    }) else {
        return;
    };
    unsafe {
        let Ok(popup) = CreatePopupMenu() else { return };
        for line in &lines {
            // "&" would otherwise underline the next letter.
            let text = HSTRING::from(line.replace('&', "&&"));
            let _ = AppendMenuW(popup, MF_STRING | MF_DISABLED, 0, &text);
        }
        if !lines.is_empty() {
            let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
        }
        let _ = AppendMenuW(popup, MF_STRING, MENU_OPEN, &HSTRING::from(open));
        let _ = AppendMenuW(popup, MF_STRING, MENU_QUIT, &HSTRING::from(quit_text));
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // Without this the menu does not close when the user clicks away.
        let _ = SetForegroundWindow(window);
        let chosen = TrackPopupMenu(popup, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, cursor.x, cursor.y, None, window, None);
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(popup);
        match chosen.0 as usize {
            MENU_OPEN => to_window(ToWindow::Open),
            MENU_QUIT => quit(window),
            _ => {}
        }
    }
}

/// Quit: the window process goes too (it also notices this process end).
fn quit(window: HWND) {
    to_window(ToWindow::Quit);
    let ids: Vec<u32> = STATE
        .with_borrow(|state| state.as_ref().map(|s| s.icons.iter().map(|i| i.planned.id).collect()))
        .unwrap_or_default();
    for id in ids {
        send(NIM_DELETE, &icon_data(window, id));
    }
    // Settings, devices and history are saved as they change.
    std::process::exit(0);
}

/// Something Windows said that changes when to poll.
fn on_signal(event: Event) {
    STATE.with_borrow_mut(|s| {
        let Some(s) = s.as_mut() else { return };
        match event {
            Event::Wake(wake) => s.monitor.wake(wake),
            Event::Suspending => {}
            Event::Changed(signal) => {
                if let Some(conditions) = s.signals.apply(signal) {
                    s.monitor.wake(Wake::Conditions(conditions));
                }
            }
        }
    });
}

/// Windows' light/dark setting changed: recolour the icons for the
/// taskbar. (The window process follows it by itself.)
fn theme_changed(window: HWND) {
    let light = theme::taskbar_light();
    STATE.with_borrow_mut(|state| state.as_mut().map(|s| s.light_taskbar = light));
    update(window, false);
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(from) = ToTray::from_message(message, wparam.0, lparam.0) {
        from_window(window, from);
        return LRESULT(0);
    }
    if let Some(event) = crate::signals::event(message, wparam, lparam) {
        on_signal(event);
        if message == WM_POWERBROADCAST {
            return LRESULT(1);
        }
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    match message {
        WM_TRAY => {
            match lparam.0 as u32 & 0xffff {
                WM_LBUTTONUP => to_window(ToWindow::Click),
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(window),
                _ => {}
            }
            LRESULT(0)
        }
        WM_OPEN => {
            to_window(ToWindow::Open);
            LRESULT(0)
        }
        WM_REFRESH => {
            update(window, false);
            // Only to a window process that is there; none is started.
            let linked = STATE.with_borrow(|s| s.as_ref().is_some_and(|s| s.window.link.is_some()));
            if linked && let Some(state) = state_message() {
                to_window(state);
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // "ImmersiveColorSet": the light/dark choice.
            let area = PCWSTR(lparam.0 as *const u16);
            if !area.is_null() && unsafe { area.to_string() }.is_ok_and(|a| a == "ImmersiveColorSet") {
                theme_changed(window);
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            // The tray's icon size may have changed.
            update(window, false);
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => {
            let taskbar_created = STATE.with_borrow(|state| state.as_ref().map(|s| s.taskbar_created)).unwrap_or(0);
            if taskbar_created != 0 && message == taskbar_created {
                // Explorer restarted and forgot every tray icon.
                update(window, true);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::devices::{Battery, Kind, Precision, Reading, Transport};
    use crate::i18n::Lang;

    #[test]
    fn messages_for_the_window_start_it_only_when_wanted() {
        let state = ToWindow::State(ipc::Snapshot::new(&[], true, 0));
        // Running and listening: everything goes.
        for message in [ToWindow::Click, ToWindow::Open, ToWindow::Quit, state.clone()] {
            assert_eq!(route(true, true, &message), Route::Send);
        }
        // Starting: commands wait for its hello, which brings the state anyway.
        assert_eq!(route(true, false, &ToWindow::Click), Route::Wait);
        assert_eq!(route(true, false, &ToWindow::Quit), Route::Wait);
        assert_eq!(route(true, false, &state), Route::Drop);
        // Not running: a click opens the flyout, Open the pinned window, and
        // a poll or Quit starts nothing.
        assert_eq!(route(false, false, &ToWindow::Click), Route::Start(Start::Flyout));
        assert_eq!(route(false, false, &ToWindow::Open), Route::Start(Start::Pinned));
        assert_eq!(route(false, false, &state), Route::Drop);
        assert_eq!(route(false, false, &ToWindow::Quit), Route::Drop);
    }

    #[test]
    fn text_is_cut_to_the_field_and_terminated() {
        let mut field = [0xffffu16; 6];
        copy_text(&mut field, "ab");
        assert_eq!(&field[..3], [b'a' as u16, b'b' as u16, 0]);
        copy_text(&mut field, "abcdefgh");
        assert_eq!(String::from_utf16_lossy(&field[..5]), "abcde");
        assert_eq!(field[5], 0);
        // A character that needs two units is kept whole or dropped.
        copy_text(&mut field, "abcd😀");
        assert_eq!(String::from_utf16_lossy(&field[..4]), "abcd");
        assert_eq!(field[4], 0);
    }

    fn devices(rules: &Rules) -> Vec<Device> {
        let mut devices = Vec::new();
        let readings = vec![
            Reading { battery: Battery::Level(82), ..Reading::new("buds", "Buds", Kind::Headphones, Transport::Bluetooth) },
            Reading { battery: Battery::Level(15), ..Reading::new("mouse", "Mouse", Kind::Mouse, Transport::Le) },
            Reading {
                battery: Battery::Level(60),
                precision: Precision::Coarse,
                charging: Some(true),
                ..Reading::new("pad", "Pad", Kind::Gamepad, Transport::Receiver)
            },
        ];
        devices::apply(&mut devices, readings, 1, rules, true);
        devices
    }

    #[test]
    fn each_tray_device_gets_its_own_stable_icon_and_tooltip() {
        let t = Lang::En.strings();
        let mut rules = Rules { licensed: true, tray: vec!["mouse".into(), "pad".into()], ..Rules::default() };
        let list = devices(&rules);
        let planned = plan(&list, &rules, false, t);
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].id, 1);
        assert_eq!(planned[0].device.as_deref(), Some("mouse"));
        assert_eq!(planned[0].spec.look, Look::Level(15, Precision::Percent));
        assert_eq!(planned[0].spec.tone, Tone::Low);
        // The first icon's tooltip also lists the device without an icon.
        assert_eq!(planned[0].tip, "Mouse: 15%\nBuds: 82%");
        assert_eq!(planned[1].tip, "Pad: Medium (charging)");
        assert_eq!(planned[1].spec.tone, Tone::Charging);
        assert!(planned[1].id >= 0x100);
        // Stable across runs and orderings.
        assert_eq!(planned[1].id, icon_id("pad", &[]));
        rules.tray = vec!["buds".into(), "pad".into(), "mouse".into()];
        let again = plan(&list, &rules, true, t);
        assert_eq!(again[1].id, planned[1].id);
        assert!(again.iter().all(|p| p.spec.light_taskbar));
        let ids: Vec<u32> = again.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids[1] != ids[2] && !ids[1..].contains(&1));
    }

    #[test]
    fn free_gets_one_icon_and_nothing_to_show_is_the_app_icon() {
        let t = Lang::En.strings();
        let rules = Rules { tray: vec!["mouse".into(), "pad".into()], ..Rules::default() };
        let list = devices(&rules);
        if crate::license::paid_tier_enabled() {
            assert_eq!(plan(&list, &rules, false, t).len(), 1);
        }
        let none = plan(&[], &rules, false, t);
        assert_eq!((none[0].id, none[0].spec.look, none[0].tip.as_str()), (1, Look::App, t.tray_none));
        // Only a device that never reported a level, not chosen: the app icon.
        let mut silent = Vec::new();
        devices::apply(&mut silent, vec![Reading::new("old", "Old", Kind::Keyboard, Transport::Bluetooth)], 1, &rules, true);
        assert_eq!(plan(&silent, &Rules::default(), false, t)[0].spec.look, Look::App);
        // Chosen, but off: a dash.
        let chosen = Rules { tray: vec!["old".into()], ..Rules::default() };
        assert_eq!(plan(&silent, &chosen, false, t)[0].spec.look, Look::NoLevel);
    }

    #[test]
    fn icon_ids_avoid_one_and_each_other() {
        let first = icon_id("a", &[]);
        assert_ne!(first, 1);
        assert_ne!(icon_id("a", &[first]), first);
    }

    #[test]
    fn notices_become_balloons_on_their_own_icons() {
        let t = Lang::En.strings();
        let rules = Rules { licensed: true, tray: vec!["mouse".into(), "pad".into()], ..Rules::default() };
        let list = devices(&rules);
        let planned = plan(&list, &rules, false, t);
        let notice = |kind, id: &str, name: &str, level: Option<u8>| Notice {
            kind,
            id: id.into(),
            name: name.into(),
            level,
            precision: Precision::Percent,
            parts: Vec::new(),
            transport: Transport::Bluetooth,
        };
        let out = balloons(
            &[
                notice(NoticeKind::Connected, "buds", "Buds", Some(82)),
                notice(NoticeKind::Low, "mouse", "Mouse", Some(15)),
                notice(NoticeKind::Full, "pad", "Pad", Some(100)),
                notice(NoticeKind::Connected, "x", "Keys", None),
            ],
            &planned,
            t,
        );
        assert_eq!(out.len(), 3);
        // Buds have no icon: the first one carries it, with Keys.
        assert_eq!(out[0].id, 1);
        assert_eq!(out[0].title, "Connected");
        assert_eq!(out[0].body, "Buds: 82%\nKeys: Bluetooth");
        assert_eq!((out[1].id, out[1].title.as_str(), out[1].body.as_str(), out[1].warning), (1, "Low battery", "Mouse is at 15%", true));
        assert_eq!(out[2].id, planned[1].id);
        assert!(!out[2].warning);
        let one = balloons(&[notice(NoticeKind::Connected, "buds", "Buds", Some(82))], &planned, t);
        assert_eq!((one[0].title.as_str(), one[0].body.as_str()), ("Buds connected", "82%"));
    }
}
