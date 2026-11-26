//! How the two processes talk. The resident process (`tray.rs`) sits in the
//! tray all day: tray icons, polling, notifications, no window. The window
//! process (`link.rs` and `app.rs`) is the same exe started with `--ui`
//! when the window is wanted, and gone again soon after it is hidden.
//!
//! - Tray to window: `ToWindow`, as JSON in a `WM_COPYDATA` sent to the
//!   window process's link window (it carries the devices, so it needs a
//!   payload).
//! - Window to tray: `ToTray`, posted messages without a payload, to the
//!   tray's hidden window. Settings travel through settings.json, which the
//!   window process writes before it says so.
//!
//! Both windows belong to the same user at the same integrity level, so
//! Windows lets the messages through without asking anything.

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_APP, WM_COPYDATA,
};

use crate::devices::{Device, Live};

/// Marks a `WM_COPYDATA` as one of ours, and its format.
pub const MAGIC: usize = 0x4254_0001;

/// The arguments that start the window process; the word after `--ui` is a
/// `Start`.
pub const UI_ARG: &str = "--ui";

/// How a window process starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Opened from a tray icon: the flyout.
    Flyout,
    /// The tray menu's Open, or the app started from the Start menu.
    Pinned,
}

impl Start {
    pub fn arg(self) -> &'static str {
        match self {
            Start::Flyout => "flyout",
            Start::Pinned => "pinned",
        }
    }

    /// The `Start` in a command line, if it starts a window process.
    pub fn from_args(args: &[String]) -> Option<Start> {
        let at = args.iter().position(|a| a == UI_ARG)?;
        match args.get(at + 1).map(String::as_str) {
            Some("pinned") => Some(Start::Pinned),
            _ => Some(Start::Flyout),
        }
    }
}

/// What the tray process tells the window process.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ToWindow {
    /// What the tray process knows now.
    State(Snapshot),
    /// A left click on a tray icon: open or close the flyout.
    Click,
    /// The tray menu's Open, or the app started again: the pinned window.
    Open,
    /// Quit from the tray menu.
    Quit,
}

/// The devices as the last poll left them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub devices: Vec<(Device, Live)>,
    /// False until the first poll has finished.
    pub polled: bool,
    /// Changes whenever history.json is written, so the window knows when
    /// to read it again.
    pub history: u64,
}

impl Snapshot {
    pub fn new(devices: &[Device], polled: bool, history: u64) -> Snapshot {
        Snapshot { devices: devices.iter().map(|d| (d.clone(), d.live())).collect(), polled, history }
    }

    pub fn into_devices(self) -> Vec<Device> {
        self.devices
            .into_iter()
            .map(|(mut device, live)| {
                device.set_live(live);
                device
            })
            .collect()
    }
}

pub fn encode(message: &ToWindow) -> Vec<u8> {
    serde_json::to_vec(message).unwrap_or_default()
}

/// `None` for anything that is not one of ours.
pub fn decode(tag: usize, bytes: &[u8]) -> Option<ToWindow> {
    if tag != MAGIC {
        return None;
    }
    serde_json::from_slice(bytes).ok()
}

/// Sends a message to the window process's link window. False if it is not
/// there any more (or hung for two seconds).
pub fn send(to: isize, message: &ToWindow) -> bool {
    let bytes = encode(message);
    let data = COPYDATASTRUCT { dwData: MAGIC, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd(to),
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&raw const data as isize),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
    };
    sent.0 != 0 && result == 1
}

/// The bytes of a `WM_COPYDATA`, copied, with its tag. Only for the
/// `lparam` of a `WM_COPYDATA` the window procedure is handling.
pub unsafe fn received(lparam: LPARAM) -> Option<(usize, Vec<u8>)> {
    let data = unsafe { (lparam.0 as *const COPYDATASTRUCT).as_ref()? };
    if data.lpData.is_null() || data.cbData == 0 {
        return Some((data.dwData, Vec::new()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
    Some((data.dwData, bytes.to_vec()))
}

/// What the window process tells the tray process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToTray {
    /// The window process is up; its link window is this.
    Hello(isize),
    /// settings.json changed; `refresh` also asks for a poll now.
    Settings { refresh: bool },
    /// Poll now.
    Refresh,
    /// The window process is going away.
    Bye,
}

const WM_HELLO: u32 = WM_APP + 20;
const WM_SETTINGS: u32 = WM_APP + 21;
const WM_POLL: u32 = WM_APP + 22;
const WM_BYE: u32 = WM_APP + 23;

impl ToTray {
    /// As a window message: message, wparam, lparam.
    pub fn to_message(self) -> (u32, usize, isize) {
        match self {
            ToTray::Hello(window) => (WM_HELLO, window as usize, 0),
            ToTray::Settings { refresh } => (WM_SETTINGS, usize::from(refresh), 0),
            ToTray::Refresh => (WM_POLL, 0, 0),
            ToTray::Bye => (WM_BYE, 0, 0),
        }
    }

    pub fn from_message(message: u32, wparam: usize, _lparam: isize) -> Option<ToTray> {
        match message {
            WM_HELLO => Some(ToTray::Hello(wparam as isize)),
            WM_SETTINGS => Some(ToTray::Settings { refresh: wparam != 0 }),
            WM_POLL => Some(ToTray::Refresh),
            WM_BYE => Some(ToTray::Bye),
            _ => None,
        }
    }

    /// Posts it to the tray's hidden window.
    pub fn post(self, tray: isize) -> bool {
        let (message, wparam, lparam) = self.to_message();
        unsafe { PostMessageW(Some(hwnd(tray)), message, WPARAM(wparam), LPARAM(lparam)).is_ok() }
    }
}

pub fn hwnd(value: isize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{self, Battery, Kind, Reading, Rules, Transport};

    fn some_devices() -> Vec<Device> {
        let mut list = Vec::new();
        let rules = Rules::default();
        let readings = vec![
            Reading { battery: Battery::Level(64), charging: Some(true), ..Reading::new("a", "Buds", Kind::Headphones, Transport::Le) },
            Reading { connected: false, ..Reading::new("b", "Mouse", Kind::Mouse, Transport::Receiver) },
        ];
        devices::apply(&mut list, readings, 1000, &rules, true);
        // The mouse was seen once, then went off.
        let again = vec![
            Reading { battery: Battery::Level(63), charging: Some(false), ..Reading::new("a", "Buds", Kind::Headphones, Transport::Le) },
            Reading { connected: false, ..Reading::new("b", "Mouse", Kind::Mouse, Transport::Receiver) },
        ];
        devices::apply(&mut list, again, 1100, &rules, false);
        list
    }

    #[test]
    fn a_snapshot_arrives_with_what_a_poll_knows_and_the_file_does_not() {
        let list = some_devices();
        let message = ToWindow::State(Snapshot::new(&list, true, 7));
        let bytes = encode(&message);
        let Some(ToWindow::State(back)) = decode(MAGIC, &bytes) else { panic!("not decoded") };
        assert!(back.polled);
        assert_eq!(back.history, 7);
        let back = back.into_devices();
        // Everything, including what devices.json leaves out.
        assert_eq!(back, list);
        assert!(back[0].connected && back[0].is_charging() == false && back[0].live_level() == Some(63));
        assert!(!back[1].connected);
    }

    #[test]
    fn commands_round_trip_and_strangers_are_ignored() {
        for message in [ToWindow::Click, ToWindow::Open, ToWindow::Quit] {
            assert_eq!(decode(MAGIC, &encode(&message)), Some(message));
        }
        assert_eq!(decode(MAGIC + 1, &encode(&ToWindow::Click)), None);
        assert_eq!(decode(MAGIC, b"{not json"), None);
        assert_eq!(decode(MAGIC, b""), None);
    }

    #[test]
    fn tray_messages_survive_the_trip_through_a_window_message() {
        for message in [ToTray::Hello(0x1234_5678), ToTray::Settings { refresh: true }, ToTray::Settings { refresh: false }, ToTray::Refresh, ToTray::Bye] {
            let (msg, wparam, lparam) = message.to_message();
            assert_eq!(ToTray::from_message(msg, wparam, lparam), Some(message));
        }
        assert_eq!(ToTray::from_message(WM_APP + 1, 0, 0), None);
    }

    #[test]
    fn the_window_process_is_told_how_to_start() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Start::from_args(&args(&["--ui", "pinned"])), Some(Start::Pinned));
        assert_eq!(Start::from_args(&args(&["--ui", "flyout"])), Some(Start::Flyout));
        assert_eq!(Start::from_args(&args(&["--ui"])), Some(Start::Flyout));
        assert_eq!(Start::from_args(&args(&["--tray"])), None);
        for start in [Start::Flyout, Start::Pinned] {
            assert_eq!(Start::from_args(&args(&["--ui", start.arg()])), Some(start));
        }
    }
}
