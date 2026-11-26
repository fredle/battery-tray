//! Xbox controllers, through XInput.
//!
//! XInput rather than `Windows.Gaming.Input`: Microsoft documents that a
//! Windows app must have focus to get data from `Windows.Gaming.Input`,
//! and this runs on a windowless background thread. XInput has no such
//! rule for desktop apps (it only stops delivering input after the app
//! itself calls `XInputEnable(false)`, which this app never does).
//!
//! XInput only says empty, low, medium or full, so these levels are
//! `Precision::Coarse` and shown as words, never as a percentage. It does
//! not say whether a controller is charging. It sees Xbox controllers on
//! the Xbox Wireless Adapter, on USB and on Bluetooth alike, but not which
//! is which and not their Bluetooth address, so:
//!
//! - when exactly one Xbox controller shows in the Bluetooth list without a
//!   level and exactly one wireless controller shows in XInput, they are
//!   the same controller and its level goes on the Bluetooth card;
//! - otherwise each XInput controller gets its own card (ids `xinput-0` to
//!   `xinput-3`, XInput's player slots), shown as on a USB receiver, and a
//!   Bluetooth Xbox card stays without a level rather than get one that
//!   might belong to another controller.
//!
//! A wired controller (XInput says "wired": no battery to report) gets a
//! card saying it does not report a level, so it is not mistaken for a
//! missing one.

use windows::Win32::UI::Input::XboxController::{
    BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL, BATTERY_LEVEL_LOW,
    BATTERY_LEVEL_MEDIUM, BATTERY_TYPE_ALKALINE, BATTERY_TYPE_NIMH,
    BATTERY_TYPE_UNKNOWN, BATTERY_TYPE_WIRED, XINPUT_BATTERY_INFORMATION, XINPUT_STATE,
    XInputGetBatteryInformation, XInputGetState,
};

use crate::devices::{Battery, Coarse, Kind, Precision, Reading, Transport};

/// One XInput slot with a controller in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    pub slot: u32,
    pub battery_type: u8,
    pub battery_level: u8,
}

impl Pad {
    pub fn wired(&self) -> bool {
        self.battery_type == BATTERY_TYPE_WIRED.0 as u8
    }
}

/// The connected controllers.
pub fn read() -> Vec<Pad> {
    let mut pads = Vec::new();
    for slot in 0..4 {
        let mut state = XINPUT_STATE::default();
        // ERROR_SUCCESS (0) means a controller is in this slot.
        if unsafe { XInputGetState(slot, &mut state) } != 0 {
            continue;
        }
        let mut info = XINPUT_BATTERY_INFORMATION::default();
        if unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut info) } != 0 {
            continue;
        }
        pads.push(Pad {
            slot,
            battery_type: info.BatteryType.0 as u8,
            battery_level: info.BatteryLevel.0 as u8,
        });
    }
    pads
}

/// XInput's battery report as a band, or `None` when it has nothing to say.
pub fn band(battery_type: u8, battery_level: u8) -> Option<Coarse> {
    // Wired and disconnected have no level to report.
    let has_battery = [BATTERY_TYPE_ALKALINE, BATTERY_TYPE_NIMH, BATTERY_TYPE_UNKNOWN]
        .iter()
        .any(|t| t.0 as u8 == battery_type);
    if !has_battery {
        return None;
    }
    match battery_level {
        l if l == BATTERY_LEVEL_EMPTY.0 as u8 => Some(Coarse::VeryLow),
        l if l == BATTERY_LEVEL_LOW.0 as u8 => Some(Coarse::Low),
        l if l == BATTERY_LEVEL_MEDIUM.0 as u8 => Some(Coarse::Medium),
        l if l == BATTERY_LEVEL_FULL.0 as u8 => Some(Coarse::Full),
        _ => None,
    }
}

fn reading(pad: &Pad) -> Reading {
    let transport = if pad.wired() { Transport::Usb } else { Transport::Receiver };
    let mut r = Reading::new(
        format!("xinput-{}", pad.slot),
        format!("Xbox controller {}", pad.slot + 1),
        Kind::Gamepad,
        transport,
    );
    match band(pad.battery_type, pad.battery_level) {
        Some(band) => {
            r.battery = Battery::Level(band.level());
            r.precision = Precision::Coarse;
        }
        None if pad.wired() => r.battery = Battery::NotReported,
        None => r.battery = Battery::Unreadable,
    }
    r
}

/// Whether a Bluetooth reading looks like an Xbox controller.
fn is_xbox(r: &Reading) -> bool {
    matches!(r.transport, Transport::Bluetooth | Transport::Le)
        && (r.hardware.is_some_and(|(vendor, _)| vendor == 0x045e) && r.kind == Kind::Gamepad
            || r.name.to_lowercase().contains("xbox"))
}

/// Puts the XInput controllers into the readings: onto the one Bluetooth
/// Xbox controller when that is unambiguous, otherwise as cards of their
/// own (see the module comment).
pub fn attach(readings: &mut Vec<Reading>, pads: Vec<Pad>) {
    let candidates: Vec<usize> = readings
        .iter()
        .enumerate()
        .filter(|(_, r)| r.connected && is_xbox(r) && !matches!(r.battery, Battery::Level(_)))
        .map(|(i, _)| i)
        .collect();
    let wireless: Vec<&Pad> = pads.iter().filter(|p| !p.wired()).collect();
    let mut merged_slot = None;
    if let ([index], [pad]) = (candidates.as_slice(), wireless.as_slice()) {
        let from_pad = reading(pad);
        let target = &mut readings[*index];
        if matches!(from_pad.battery, Battery::Level(_)) {
            target.battery = from_pad.battery;
            target.precision = from_pad.precision;
            target.kind = Kind::Gamepad;
        }
        merged_slot = Some(pad.slot);
    }
    for pad in &pads {
        if Some(pad.slot) != merged_slot {
            readings.push(reading(pad));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIMH: u8 = 3;
    const WIRED: u8 = 1;

    fn pad(slot: u32, battery_type: u8, battery_level: u8) -> Pad {
        Pad { slot, battery_type, battery_level }
    }

    #[test]
    fn xinput_levels_are_bands_not_percentages() {
        assert_eq!(band(NIMH, 0), Some(Coarse::VeryLow));
        assert_eq!(band(NIMH, 1), Some(Coarse::Low));
        assert_eq!(band(2, 2), Some(Coarse::Medium));
        assert_eq!(band(0xff, 3), Some(Coarse::Full));
        assert_eq!(band(WIRED, 3), None);
        assert_eq!(band(0, 3), None);
        assert_eq!(band(NIMH, 9), None);
        let r = reading(&pad(0, NIMH, 2));
        assert_eq!(r.precision, Precision::Coarse);
        assert_eq!(r.battery, Battery::Level(Coarse::Medium.level()));
        assert_eq!(r.transport, Transport::Receiver);
        let wired = reading(&pad(1, WIRED, 0));
        assert_eq!(wired.battery, Battery::NotReported);
        assert_eq!(wired.transport, Transport::Usb);
        assert_eq!(wired.id, "xinput-1");
    }

    fn xbox_bt(id: &str) -> Reading {
        Reading::new(id, "Xbox Wireless Controller", Kind::Gamepad, Transport::Le)
    }

    #[test]
    fn one_bluetooth_controller_and_one_pad_are_the_same_controller() {
        let mut readings = vec![xbox_bt("aa"), Reading::new("bb", "Buds", Kind::Headphones, Transport::Bluetooth)];
        attach(&mut readings, vec![pad(0, NIMH, 1)]);
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].battery, Battery::Level(Coarse::Low.level()));
        assert_eq!(readings[0].precision, Precision::Coarse);
        assert_eq!(readings[0].transport, Transport::Le);
    }

    #[test]
    fn ambiguous_controllers_are_never_given_each_others_levels() {
        // Two pads, one Bluetooth controller: which pad is it? Unknown.
        let mut readings = vec![xbox_bt("aa")];
        attach(&mut readings, vec![pad(0, NIMH, 1), pad(1, NIMH, 3)]);
        assert_eq!(readings.len(), 3);
        assert_eq!(readings[0].battery, Battery::NotReported);
        // A wired pad is not a candidate for the Bluetooth card.
        let mut readings = vec![xbox_bt("aa")];
        attach(&mut readings, vec![pad(0, WIRED, 0)]);
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].battery, Battery::NotReported);
        // No Bluetooth controller: the pad is its own card.
        let mut readings = Vec::new();
        attach(&mut readings, vec![pad(2, NIMH, 3)]);
        assert_eq!(readings[0].id, "xinput-2");
        // An Xbox controller that is off is not a candidate.
        let mut off = xbox_bt("aa");
        off.connected = false;
        let mut readings = vec![off];
        attach(&mut readings, vec![pad(0, NIMH, 3)]);
        assert_eq!(readings.len(), 2);
    }
}
