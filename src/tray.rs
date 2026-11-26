//! The tray icon, its tooltip, menu and low-battery notifications, plus
//! hiding and showing the main window. It runs on its own thread with its
//! own hidden window and message loop, so it keeps working while the main
//! window is hidden (eframe stops calling the app then).
//!
//! Plain Win32 through the `windows` crate: `Shell_NotifyIconW` for the icon
//! and notifications, a popup menu on right-click.

use std::cell::RefCell;
use std::sync::mpsc::channel;

use eframe::egui;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, HICON, IsIconic,
    MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::devices;
use crate::icon;
use crate::monitor::{SharedState, lock};

/// Also how a second copy of the app finds the running one.
const CLASS: PCWSTR = w!("BatteryTrayHiddenWindow");
/// From the tray icon; `lparam` is the mouse message.
const WM_TRAY: u32 = WM_APP + 1;
/// Show the main window.
const WM_OPEN: u32 = WM_APP + 2;
/// The shared state changed: redraw the icon, show waiting warnings.
const WM_REFRESH: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;
/// Set to anything to keep notifications off the screen (for screenshots).
const QUIET_ENV: &str = "BT_QUIET";

struct State {
    shared: SharedState,
    /// The main window, as a number so it can cross threads.
    main_window: isize,
    ctx: egui::Context,
    icon: HICON,
    /// What the icon currently shows, to redraw only on a change.
    shown: Option<(Option<u8>, bool)>,
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// A handle to the tray thread.
#[derive(Clone, Copy)]
pub struct Tray {
    window: isize,
    main_window: isize,
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
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

impl Tray {
    /// Starts the tray thread. `None` if Windows would not create it, in
    /// which case the app behaves like an ordinary window.
    pub fn start(shared: SharedState, main_window: isize, ctx: egui::Context) -> Option<Tray> {
        let (sender, receiver) = channel();
        std::thread::spawn(move || unsafe {
            let window = create_window();
            let _ = sender.send(window.map(|w| w.0 as isize));
            let Some(window) = window else { return };
            STATE.with_borrow_mut(|state| {
                *state = Some(State {
                    shared,
                    main_window,
                    ctx,
                    icon: HICON::default(),
                    shown: None,
                    taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                });
            });
            update(window, true);
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        let window = receiver.recv().ok()??;
        Some(Tray {
            window,
            main_window,
        })
    }

    /// Tells the tray that the shared state changed. Safe from any thread.
    pub fn refresh(&self) {
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.window)), WM_REFRESH, WPARAM(0), LPARAM(0));
        }
    }

    /// Hides the main window; the app carries on in the tray.
    pub fn hide_main(&self) {
        unsafe {
            let _ = ShowWindow(hwnd(self.main_window), SW_HIDE);
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
        // "TaskbarCreated" broadcast and cannot be found by another copy.
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

fn show_main(main_window: isize, ctx: &egui::Context) {
    unsafe {
        let window = hwnd(main_window);
        let _ = ShowWindow(window, if IsIconic(window).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(window);
    }
    ctx.request_repaint();
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

fn icon_data(window: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn make_icon(level: Option<u8>, low: bool) -> HICON {
    let mut pixels = icon::render(level, low);
    // Windows wants blue first.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // With 32-bit colour the alpha channel decides transparency; the mask
    // is still required.
    let mask = vec![0u8; icon::SIZE * icon::SIZE / 8];
    let size = icon::SIZE as i32;
    unsafe { CreateIcon(None, size, size, 1, 32, mask.as_ptr(), pixels.as_ptr()).unwrap_or_default() }
}

/// Brings the icon, tooltip and notifications in line with the shared
/// state. `add` puts the icon in the tray (at start, and when Explorer
/// restarts).
fn update(window: HWND, add: bool) {
    // The state is only borrowed for a moment at a time: Windows may call
    // `window_proc` again while a tray call below is waiting for Explorer.
    let Some((shared, shown)) =
        STATE.with_borrow(|state| state.as_ref().map(|s| (s.shared.clone(), s.shown)))
    else {
        return;
    };
    let (level, low, tip, warnings, t) = {
        let mut shared = lock(&shared);
        let t = shared.lang.strings();
        let chosen = shared.tray_device.as_deref();
        let level = devices::tray_device(&shared.devices, chosen).and_then(|d| d.live_level());
        let low = level.is_some_and(|level| level <= shared.threshold);
        let tip = devices::tooltip(&shared.devices, t);
        (level, low, tip, std::mem::take(&mut shared.pending), t)
    };

    let mut data = icon_data(window);
    data.uFlags = NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    copy_text(&mut data.szTip, &tip);
    if add || shown != Some((level, low)) {
        let new = make_icon(level, low);
        let old = STATE.with_borrow_mut(|state| {
            let state = state.as_mut()?;
            state.shown = Some((level, low));
            Some(std::mem::replace(&mut state.icon, new))
        });
        data.uFlags |= NIF_ICON;
        data.hIcon = new;
        send(if add { NIM_ADD } else { NIM_MODIFY }, &data);
        if let Some(old) = old
            && !old.is_invalid()
        {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
    } else {
        send(NIM_MODIFY, &data);
    }

    if !warnings.is_empty() && std::env::var_os(QUIET_ENV).is_none() {
        let body = warnings
            .iter()
            .map(|w| devices::fill_level(t.low_body, w.level, w.precision, t, &[("name", &w.name)]))
            .collect::<Vec<_>>()
            .join("\n");
        let mut data = icon_data(window);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_WARNING;
        copy_text(&mut data.szInfoTitle, t.low_title);
        copy_text(&mut data.szInfo, &body);
        send(NIM_MODIFY, &data);
    }
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
    let Some((lines, open, quit, main_window, ctx)) = STATE.with_borrow(|state| {
        let state = state.as_ref()?;
        let shared = lock(&state.shared);
        let t = shared.lang.strings();
        let lines: Vec<String> = shared
            .devices
            .iter()
            .map(|d| devices::summary_line(d, t))
            .collect();
        Some((lines, t.tray_open, t.tray_quit, state.main_window, state.ctx.clone()))
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
        let _ = AppendMenuW(popup, MF_STRING, MENU_QUIT, &HSTRING::from(quit));
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // Without this the menu does not close when the user clicks away.
        let _ = SetForegroundWindow(window);
        let chosen = TrackPopupMenu(
            popup,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            window,
            None,
        );
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(popup);
        match chosen.0 as usize {
            MENU_OPEN => show_main(main_window, &ctx),
            MENU_QUIT => {
                send(NIM_DELETE, &icon_data(window));
                // Settings and device history are saved as they change.
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAY => {
            match lparam.0 as u32 & 0xffff {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    let _ = unsafe { PostMessageW(Some(window), WM_OPEN, WPARAM(0), LPARAM(0)) };
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(window),
                _ => {}
            }
            LRESULT(0)
        }
        WM_OPEN => {
            let target = STATE.with_borrow(|state| {
                state.as_ref().map(|s| (s.main_window, s.ctx.clone()))
            });
            if let Some((main_window, ctx)) = target {
                show_main(main_window, &ctx);
            }
            LRESULT(0)
        }
        WM_REFRESH => {
            update(window, false);
            LRESULT(0)
        }
        _ => {
            let taskbar_created = STATE
                .with_borrow(|state| state.as_ref().map(|s| s.taskbar_created))
                .unwrap_or(0);
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
}
