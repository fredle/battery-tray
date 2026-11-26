//! What Windows tells the tray window that changes when to poll: devices
//! arriving or leaving, the PC going to sleep and waking, the power status,
//! battery or energy saver, the screen going off and the session locking.
//! Each becomes a `Wake` for the poll thread (`monitor.rs`).

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
use windows::Win32::UI::WindowsAndMessaging::{
    DBT_DEVNODES_CHANGED, DEVICE_NOTIFY_WINDOW_HANDLE, PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_DEVICECHANGE, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE,
};
use windows::core::GUID;

use crate::monitor::{Conditions, Wake};

/// The session's screen: 0 off, 1 on, 2 dimmed.
const GUID_SESSION_DISPLAY_STATUS: GUID = GUID::from_u128(0x2b84c20e_ad23_4ddf_93db_05ffbd7efca5);
/// Windows 10's battery saver: 0 off, 1 on.
const GUID_POWER_SAVING_STATUS: GUID = GUID::from_u128(0xe00958c0_c213_4ace_ac77_fecced2eeea5);
/// Windows 11's energy saver (GUID_ENERGY_SAVER_STATUS, not in the
/// `windows` crate's metadata yet): 0 off, 1 standard, 2 high savings.
const GUID_ENERGY_SAVER_STATUS: GUID = GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

/// Asks Windows to tell `window` about all of the above.
pub fn register(window: HWND) {
    let recipient = HANDLE(window.0);
    unsafe {
        for setting in [GUID_SESSION_DISPLAY_STATUS, GUID_POWER_SAVING_STATUS, GUID_ENERGY_SAVER_STATUS] {
            let _ = RegisterPowerSettingNotification(recipient, &setting, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
        let _ = WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION);
    }
}

/// What happened, from one window message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Wake(Wake),
    /// The PC is about to sleep.
    Suspending,
    Changed(Signal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    BatterySaver(bool),
    EnergySaver(bool),
    DisplayOff(bool),
    Locked(bool),
}

/// The signals so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signals {
    battery_saver: bool,
    energy_saver: bool,
    display_off: bool,
    locked: bool,
}

impl Signals {
    pub fn conditions(&self) -> Conditions {
        Conditions { saver: self.battery_saver || self.energy_saver, away: self.display_off || self.locked }
    }

    /// Takes a signal in; returns the new conditions if they changed.
    pub fn apply(&mut self, signal: Signal) -> Option<Conditions> {
        let before = self.conditions();
        match signal {
            Signal::BatterySaver(on) => self.battery_saver = on,
            Signal::EnergySaver(on) => self.energy_saver = on,
            Signal::DisplayOff(off) => self.display_off = off,
            Signal::Locked(locked) => self.locked = locked,
        }
        let after = self.conditions();
        (after != before).then_some(after)
    }
}

/// The value of a power setting notification: its GUID and first DWORD.
pub fn power_setting(guid: GUID, value: u32) -> Option<Signal> {
    if guid == GUID_SESSION_DISPLAY_STATUS {
        // Dimmed is still on.
        Some(Signal::DisplayOff(value == 0))
    } else if guid == GUID_POWER_SAVING_STATUS {
        Some(Signal::BatterySaver(value != 0))
    } else if guid == GUID_ENERGY_SAVER_STATUS {
        Some(Signal::EnergySaver(value != 0))
    } else {
        None
    }
}

/// The event in a window message, if it is one of these.
pub fn event(message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<Event> {
    match message {
        WM_DEVICECHANGE if wparam.0 as u32 == DBT_DEVNODES_CHANGED => Some(Event::Wake(Wake::DevicesChanged)),
        WM_POWERBROADCAST => match wparam.0 as u32 {
            PBT_APMRESUMEAUTOMATIC => Some(Event::Wake(Wake::Resumed)),
            PBT_APMSUSPEND => Some(Event::Suspending),
            PBT_APMPOWERSTATUSCHANGE => Some(Event::Wake(Wake::Power)),
            PBT_POWERSETTINGCHANGE => {
                let setting = unsafe { (lparam.0 as *const POWERBROADCAST_SETTING).as_ref()? };
                if setting.DataLength < 4 {
                    return None;
                }
                let value = unsafe { (setting.Data.as_ptr() as *const u32).read_unaligned() };
                power_setting(setting.PowerSetting, value).map(Event::Changed)
            }
            _ => None,
        },
        WM_WTSSESSION_CHANGE => match wparam.0 {
            WTS_SESSION_LOCK => Some(Event::Changed(Signal::Locked(true))),
            WTS_SESSION_UNLOCK => Some(Event::Changed(Signal::Locked(false))),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn away_and_saver_follow_what_windows_says() {
        let mut s = Signals::default();
        assert_eq!(s.conditions(), Conditions::default());
        assert_eq!(s.apply(Signal::Locked(true)), Some(Conditions { saver: false, away: true }));
        // The screen going off while locked changes nothing more.
        assert_eq!(s.apply(Signal::DisplayOff(true)), None);
        assert_eq!(s.apply(Signal::Locked(false)), None);
        assert_eq!(s.apply(Signal::DisplayOff(false)), Some(Conditions::default()));
        assert_eq!(s.apply(Signal::EnergySaver(true)), Some(Conditions { saver: true, away: false }));
        assert_eq!(s.apply(Signal::BatterySaver(true)), None);
        assert_eq!(s.apply(Signal::EnergySaver(false)), None);
        assert_eq!(s.apply(Signal::BatterySaver(false)), Some(Conditions::default()));
    }

    #[test]
    fn power_settings_are_read_by_their_guid() {
        assert_eq!(power_setting(GUID_SESSION_DISPLAY_STATUS, 0), Some(Signal::DisplayOff(true)));
        assert_eq!(power_setting(GUID_SESSION_DISPLAY_STATUS, 1), Some(Signal::DisplayOff(false)));
        // Dimmed is still on.
        assert_eq!(power_setting(GUID_SESSION_DISPLAY_STATUS, 2), Some(Signal::DisplayOff(false)));
        assert_eq!(power_setting(GUID_POWER_SAVING_STATUS, 1), Some(Signal::BatterySaver(true)));
        assert_eq!(power_setting(GUID_ENERGY_SAVER_STATUS, 2), Some(Signal::EnergySaver(true)));
        assert_eq!(power_setting(GUID_ENERGY_SAVER_STATUS, 0), Some(Signal::EnergySaver(false)));
        assert_eq!(power_setting(GUID::zeroed(), 1), None);
    }

    #[test]
    fn window_messages_become_wakes() {
        let event = |m, w: usize| event(m, WPARAM(w), LPARAM(0));
        assert_eq!(event(WM_DEVICECHANGE, DBT_DEVNODES_CHANGED as usize), Some(Event::Wake(Wake::DevicesChanged)));
        assert_eq!(event(WM_DEVICECHANGE, 0x8000), None);
        assert_eq!(event(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC as usize), Some(Event::Wake(Wake::Resumed)));
        assert_eq!(event(WM_POWERBROADCAST, PBT_APMSUSPEND as usize), Some(Event::Suspending));
        assert_eq!(event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK), Some(Event::Changed(Signal::Locked(true))));
        assert_eq!(event(WM_WTSSESSION_CHANGE, 5), None);
    }
}
