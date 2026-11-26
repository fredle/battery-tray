//! The window's two shapes. A click on the tray icon opens it as a
//! flyout: borderless, in the corner of the screen next to the tray, on top,
//! without a taskbar button, gone as soon as it loses focus or Esc is
//! pressed, like Windows 11's own flyouts. Pinned (the pin button, the tray
//! menu's Open, or starting the app from the Start menu) it is an ordinary
//! window that stays until closed.
//!
//! The window is always undecorated: eframe keeps Windows' frame styles on
//! it, so Windows 11 still rounds its corners and draws its shadow, and
//! Windows 10 shows it square. The backdrop is a solid colour from the
//! palette; a translucent Mica backdrop needs a transparent OpenGL surface,
//! which is not reliable with eframe's glow backend.
//!
//! The geometry is plain functions (`flyout_rect`, `centred`); the rest is
//! Win32 on the window handle, callable from the tray thread.

use std::time::Instant;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromRect};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETTASKBARPOS, APPBARDATA, ITaskbarList, SHAppBarMessage, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos, ShowWindow,
};

/// Window width in points, both shapes.
pub const WIDTH: f32 = 380.0;
/// The pinned window's height in points.
pub const PINNED_HEIGHT: f32 = 640.0;
/// The flyout follows its content between these heights (points).
pub const MIN_HEIGHT: f32 = 220.0;
pub const MAX_HEIGHT: f32 = 720.0;
/// Until the content has been measured.
pub const DEFAULT_HEIGHT: f32 = 460.0;
/// Gap between the flyout and the edges of the work area, in points, as
/// Windows 11's flyouts have.
pub const MARGIN: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Hidden,
    Flyout,
    Pinned,
}

/// What the tray thread and the window agree on about the window.
#[derive(Clone, Copy, Debug)]
pub struct WindowState {
    pub mode: Mode,
    /// Counts every show, so the window can tell a fresh one.
    pub shown: u32,
    /// When the flyout last closed itself because it lost focus. A click on
    /// the tray icon takes the focus first, so that click must not reopen it.
    pub blurred_at: Option<Instant>,
    /// The height the flyout's content wants, in points.
    pub height: f32,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            mode: Mode::Hidden,
            shown: 0,
            blurred_at: None,
            height: DEFAULT_HEIGHT,
        }
    }
}

impl WindowState {
    /// What a left click on the tray icon should do.
    pub fn on_tray_click(&self, now: Instant) -> TrayClick {
        match self.mode {
            Mode::Flyout => TrayClick::Hide,
            Mode::Pinned => TrayClick::Raise,
            Mode::Hidden if self.blurred_at.is_some_and(|at| now.duration_since(at).as_millis() < 500) => TrayClick::Nothing,
            Mode::Hidden => TrayClick::Flyout,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayClick {
    Flyout,
    Hide,
    /// Bring the pinned window to the front.
    Raise,
    /// The click that just closed the flyout by taking its focus.
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Px {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Px {
    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }

    fn sized(left: i32, top: i32, width: i32, height: i32) -> Px {
        Px { left, top, right: left + width, bottom: top + height }
    }
}

/// Points to pixels at a DPI.
pub fn px(points: f32, dpi: u32) -> i32 {
    (points * dpi as f32 / 96.0).round() as i32
}

/// The flyout's place: in the corner of the work area nearest the tray
/// (bottom right for a taskbar at the bottom, the top or right; bottom
/// left for one on the left), `margin` from both edges, and never larger
/// than the work area.
pub fn flyout_rect(work: Px, edge: Edge, size: (i32, i32), margin: i32) -> Px {
    let width = size.0.min(work.width() - 2 * margin).max(1);
    let height = size.1.min(work.height() - 2 * margin).max(1);
    let left = match edge {
        Edge::Left => work.left + margin,
        _ => work.right - margin - width,
    };
    let top = match edge {
        Edge::Top => work.top + margin,
        _ => work.bottom - margin - height,
    };
    Px::sized(left, top, width, height)
}

/// The middle of the work area, shrunk to fit if need be.
pub fn centred(work: Px, size: (i32, i32)) -> Px {
    let width = size.0.min(work.width()).max(1);
    let height = size.1.min(work.height()).max(1);
    Px::sized(work.left + (work.width() - width) / 2, work.top + (work.height() - height) / 2, width, height)
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

/// The work area and DPI of the monitor with the taskbar, and the edge the
/// taskbar is on. The primary monitor, bottom, if Windows will not say.
pub fn taskbar_monitor() -> (Px, u32, Edge) {
    unsafe {
        let mut bar = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, ..Default::default() };
        let found = SHAppBarMessage(ABM_GETTASKBARPOS, &mut bar) != 0;
        let edge = match bar.uEdge {
            _ if !found => Edge::Bottom,
            ABE_LEFT => Edge::Left,
            ABE_TOP => Edge::Top,
            ABE_RIGHT => Edge::Right,
            _ => Edge::Bottom,
        };
        let monitor = MonitorFromRect(&bar.rc, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let work = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcWork;
            Px { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
        } else {
            Px { left: 0, top: 0, right: 1280, bottom: 720 }
        };
        let (mut x, mut y) = (96u32, 96u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() {
            x = 96;
        }
        (work, x.max(48), edge)
    }
}

/// Moves and sizes the window, on top or not. Twice: if the move takes it
/// to a monitor with another DPI, Windows rescales it on the way, and the
/// second call puts the right size back.
fn place(window: HWND, rect: Px, topmost: bool) {
    let after = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
    for _ in 0..2 {
        unsafe {
            let _ = SetWindowPos(window, Some(after), rect.left, rect.top, rect.width(), rect.height(), SWP_NOACTIVATE);
        }
    }
}

/// Shows or hides the window's taskbar button.
fn set_taskbar_button(window: HWND, on: bool) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(list) = CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            && list.HrInit().is_ok()
        {
            let _ = if on { list.AddTab(window) } else { list.DeleteTab(window) };
        }
    }
}

fn bring_up(window: HWND) {
    unsafe {
        let _ = ShowWindow(window, if IsIconic(window).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(window);
    }
}

/// Opens the window as the flyout, `height` points tall.
pub fn show_flyout(window: isize, height: f32) {
    let window = hwnd(window);
    let (work, dpi, edge) = taskbar_monitor();
    let size = (px(WIDTH, dpi), px(height.clamp(MIN_HEIGHT, MAX_HEIGHT), dpi));
    place(window, flyout_rect(work, edge, size, px(MARGIN, dpi)), true);
    bring_up(window);
    set_taskbar_button(window, false);
}

/// Follows the flyout's content: same corner, new height.
pub fn resize_flyout(window: isize, height: f32) {
    let window = hwnd(window);
    let (work, dpi, edge) = taskbar_monitor();
    let size = (px(WIDTH, dpi), px(height.clamp(MIN_HEIGHT, MAX_HEIGHT), dpi));
    let rect = flyout_rect(work, edge, size, px(MARGIN, dpi));
    let mut now = RECT::default();
    unsafe {
        let _ = GetWindowRect(window, &mut now);
    }
    if (now.bottom - now.top - rect.height()).abs() > 1 || now.left != rect.left {
        place(window, rect, true);
    }
}

/// Opens the window pinned. `recentre` puts it in the middle of the
/// taskbar's monitor; otherwise it stays where it was (where the user
/// dragged it, or where the flyout was), grown to the pinned height and
/// kept on screen.
pub fn show_pinned(window: isize, recentre: bool) {
    let window = hwnd(window);
    let (work, dpi, edge) = taskbar_monitor();
    let size = (px(WIDTH, dpi), px(PINNED_HEIGHT, dpi));
    let rect = if recentre {
        centred(work, size)
    } else {
        let mut now = RECT::default();
        unsafe {
            let _ = GetWindowRect(window, &mut now);
        }
        let at = Px { left: now.left, top: now.top, right: now.right, bottom: now.bottom };
        if at.width() <= 0 || at.right < work.left || at.left > work.right {
            centred(work, size)
        } else {
            // Grow upwards from the flyout's corner, as it sits on the taskbar.
            flyout_rect(work, edge, size, (work.right - at.right).max(0).min(px(MARGIN, dpi)))
        }
    };
    place(window, rect, false);
    bring_up(window);
    set_taskbar_button(window, true);
}

/// Puts the (not yet shown) window in the middle of the taskbar's monitor
/// at the pinned size, for a start from the Start menu.
pub fn centre(window: isize) {
    let (work, dpi, _) = taskbar_monitor();
    place(hwnd(window), centred(work, (px(WIDTH, dpi), px(PINNED_HEIGHT, dpi))), false);
}

/// Brings a pinned window to the front without moving it.
pub fn raise(window: isize) {
    let window = hwnd(window);
    unsafe {
        let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    bring_up(window);
}

pub fn hide(window: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(window), SW_HIDE);
    }
}

pub fn visible(window: isize) -> bool {
    unsafe { IsWindowVisible(hwnd(window)).as_bool() }
}

/// Asks Windows 11 for round corners and a border in the palette's colour.
/// Returns false on Windows 10, where neither exists and the window draws
/// its own border.
pub fn style(window: isize, border: [u8; 3]) -> bool {
    let window = hwnd(window);
    unsafe {
        let round = DWMWCP_ROUND;
        let rounded = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const round).cast(),
            size_of_val(&round) as u32,
        )
        .is_ok();
        let colour = u32::from(border[0]) | u32::from(border[1]) << 8 | u32::from(border[2]) << 16;
        let _ = DwmSetWindowAttribute(window, DWMWA_BORDER_COLOR, (&raw const colour).cast(), size_of_val(&colour) as u32);
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Px = Px { left: 0, top: 0, right: 1920, bottom: 1032 };

    #[test]
    fn the_flyout_sits_in_the_tray_corner_of_the_work_area() {
        // 1080p, taskbar at the bottom, 100%.
        assert_eq!(flyout_rect(WORK, Edge::Bottom, (380, 460), 12), Px { left: 1528, top: 560, right: 1908, bottom: 1020 });
        // Taskbar on top: the work area starts below it.
        let top = Px { top: 48, ..WORK };
        assert_eq!(flyout_rect(top, Edge::Top, (380, 460), 12).top, 60);
        // On the left (Windows 10): the tray is bottom left.
        let left = Px { left: 62, ..WORK };
        assert_eq!(flyout_rect(left, Edge::Left, (380, 460), 12).left, 74);
        assert_eq!(flyout_rect(left, Edge::Left, (380, 460), 12).bottom, 1020);
        // On the right: bottom right, clear of the taskbar.
        let right = Px { right: 1858, ..WORK };
        assert_eq!(flyout_rect(right, Edge::Right, (380, 460), 12).right, 1846);
    }

    #[test]
    fn the_flyout_never_leaves_the_work_area_and_scales_with_dpi() {
        // 150% on a small laptop screen: taller than the work area.
        let work = Px { left: 0, top: 0, right: 1366, bottom: 720 };
        let r = flyout_rect(work, Edge::Bottom, (px(WIDTH, 144), px(MAX_HEIGHT, 144)), px(MARGIN, 144));
        assert_eq!(r.width(), 570);
        assert_eq!((r.top, r.bottom), (18, 702));
        // A second monitor to the left of the primary one.
        let second = Px { left: -2560, top: 0, right: 0, bottom: 1400 };
        let r = flyout_rect(second, Edge::Bottom, (760, 920), 24);
        assert_eq!((r.left, r.right, r.bottom), (-784, -24, 1376));
        assert_eq!(px(380.0, 96), 380);
        assert_eq!(px(380.0, 120), 475);
        assert_eq!(px(12.0, 192), 24);
    }

    #[test]
    fn the_pinned_window_is_centred_and_fits() {
        assert_eq!(centred(WORK, (380, 640)), Px { left: 770, top: 196, right: 1150, bottom: 836 });
        let small = Px { left: 0, top: 0, right: 800, bottom: 500 };
        assert_eq!(centred(small, (380, 640)), Px { left: 210, top: 0, right: 590, bottom: 500 });
    }

    #[test]
    fn a_tray_click_toggles_the_flyout_but_not_right_after_it_lost_focus() {
        let now = Instant::now();
        let mut state = WindowState::default();
        assert_eq!(state.on_tray_click(now), TrayClick::Flyout);
        state.mode = Mode::Flyout;
        assert_eq!(state.on_tray_click(now), TrayClick::Hide);
        // The click took the focus, the flyout closed itself, then the click arrives.
        state.mode = Mode::Hidden;
        state.blurred_at = Some(now);
        assert_eq!(state.on_tray_click(now + std::time::Duration::from_millis(80)), TrayClick::Nothing);
        assert_eq!(state.on_tray_click(now + std::time::Duration::from_secs(2)), TrayClick::Flyout);
        state.mode = Mode::Pinned;
        assert_eq!(state.on_tray_click(now), TrayClick::Raise);
    }
}
