//! PlayStation DualSense and DualShock 4 controllers, from their HID input
//! reports (layout from Linux's `hid-playstation` driver).
//!
//! | controller | link | report | status byte |
//! |---|---|---|---|
//! | DualShock 4 | USB, Sony wireless adapter | 0x01, 64 bytes | 30 |
//! | DualShock 4 | Bluetooth | 0x11, 78 bytes | 32 |
//! | DualSense | USB | 0x01, 64 bytes | 53 |
//! | DualSense | Bluetooth | 0x31, 78 bytes | 54 |
//!
//! DualShock 4 status: low nibble the level in tens (0-10; 11 with the
//! cable in means full), bit 0x10 the USB cable. DualSense status: low
//! nibble the level in tens, high nibble 0 discharging, 1 charging, 2 full,
//! anything else an error. Levels are `Precision::Tens`.
//!
//! Over Bluetooth both controllers start in a basic mode (report 0x01,
//! without the battery) and only switch to the full reports when a program
//! asks for a feature report. Asking would change what other programs and
//! games get from the controller, so this module never does: a controller in
//! basic mode is shown as not reporting its level (Steam or DS4Windows
//! running switches it, and then the level shows).
//!
//! Collections are opened read-only and nothing is ever sent.

use std::time::{Duration, Instant};

use crate::devices::{Battery, Kind, Precision, Reading, Transport};
use crate::hid::{Device, Info};

pub const VENDOR: u16 = 0x054c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    DualShock4,
    DualSense,
}

/// The controller model and name for a product ID, and whether it is the
/// DualShock 4 wireless adapter.
pub fn model(product: u16) -> Option<(Model, &'static str, bool)> {
    match product {
        0x05c4 | 0x09cc => Some((Model::DualShock4, "DUALSHOCK 4", false)),
        0x0ba0 => Some((Model::DualShock4, "DUALSHOCK 4", true)),
        0x0ce6 => Some((Model::DualSense, "DualSense", false)),
        0x0df2 => Some((Model::DualSense, "DualSense Edge", false)),
        _ => None,
    }
}

/// What one input report says about the battery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// Level in tens of percent, and charging.
    Level(u8, bool),
    /// A basic Bluetooth report, without the battery.
    Basic,
    /// The controller reported a charging error, or a value out of range.
    Error,
    /// The adapter has no controller connected.
    NoController,
}

/// Decodes one input report; `None` if it is not one of the known ones.
pub fn parse(model: Model, bluetooth: bool, dongle: bool, report: &[u8]) -> Option<Report> {
    let id = *report.first()?;
    let at = match (model, bluetooth, id) {
        (_, true, 0x01) => return Some(Report::Basic),
        (Model::DualShock4, false, 0x01) => 30,
        (Model::DualShock4, true, 0x11) => 32,
        (Model::DualSense, false, 0x01) => 53,
        (Model::DualSense, true, 0x31) => 54,
        _ => return None,
    };
    let status = *report.get(at)?;
    let level = status & 0x0f;
    Some(match model {
        Model::DualShock4 => {
            // The second status byte's bit 0x04: the adapter has lost its controller.
            if dongle && report.get(at + 1).is_some_and(|s| s & 0x04 != 0) {
                return Some(Report::NoController);
            }
            let cable = status & 0x10 != 0;
            match (cable, level) {
                (true, 0..=10) => Report::Level(level * 10, true),
                (true, 11) => Report::Level(100, false),
                (false, 0..=10) => Report::Level(level * 10, false),
                _ => Report::Error,
            }
        }
        Model::DualSense => match (status >> 4, level) {
            (0, 0..=10) => Report::Level(level * 10, false),
            (1, 0..=10) => Report::Level(level * 10, true),
            (2, _) => Report::Level(100, false),
            _ => Report::Error,
        },
    })
}

/// How long to wait for a report from one controller. They send hundreds
/// a second, so this is only reached when one is not sending at all.
const WAIT: Duration = Duration::from_millis(150);

pub fn read(infos: &[Info], deadline: Instant) -> Vec<Reading> {
    let mut readings: Vec<Reading> = Vec::new();
    for info in infos {
        if info.vendor != VENDOR || info.usage_page != 0x01 || info.usage != 0x05 {
            continue;
        }
        let Some((model, name, dongle)) = model(info.product) else {
            continue;
        };
        let bluetooth = info.address.is_some();
        let mut result = None;
        if Instant::now() < deadline
            && let Some(mut device) = Device::open(info, false)
        {
            let until = (Instant::now() + WAIT).min(deadline);
            // A few reports in, in case the first is not the kind we know.
            for _ in 0..4 {
                let Some(report) = device.read(until) else { break };
                if let Some(parsed) = parse(model, bluetooth, dongle, &report) {
                    result = Some(parsed);
                    break;
                }
            }
        }
        if result == Some(Report::NoController) {
            continue;
        }
        let id = match &info.address {
            Some(address) => address.clone(),
            None => {
                let base = format!("sony-{:04x}", info.product);
                let same = readings.iter().filter(|r| r.id.starts_with(&base)).count();
                if same == 0 { base } else { format!("{base}-{}", same + 1) }
            }
        };
        let transport = match (bluetooth, dongle) {
            (true, _) => Transport::Bluetooth,
            (false, true) => Transport::Receiver,
            (false, false) => Transport::Usb,
        };
        let mut reading = Reading::new(id, name, Kind::Gamepad, transport);
        match result {
            Some(Report::Level(level, charging)) => {
                reading.battery = Battery::Level(level);
                reading.precision = Precision::Tens;
                reading.charging = Some(charging);
            }
            Some(Report::Basic) => reading.battery = Battery::NotReported,
            _ => reading.battery = Battery::Unreadable,
        }
        readings.push(reading);
    }
    readings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, len: usize, at: usize, status: u8) -> Vec<u8> {
        let mut r = vec![0u8; len];
        r[0] = id;
        r[at] = status;
        r
    }

    #[test]
    fn dualsense_usb_and_bluetooth() {
        // USB, discharging, level 7.
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0x07)), Some(Report::Level(70, false)));
        // USB, charging, level 3.
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0x13)), Some(Report::Level(30, true)));
        // Full.
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0x2a)), Some(Report::Level(100, false)));
        // Bluetooth full report: status one byte later.
        assert_eq!(parse(Model::DualSense, true, false, &report(0x31, 78, 54, 0x15)), Some(Report::Level(50, true)));
        // Temperature or charging errors are not levels.
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0xa5)), Some(Report::Error));
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0xf5)), Some(Report::Error));
        assert_eq!(parse(Model::DualSense, false, false, &report(0x01, 64, 53, 0x0c)), Some(Report::Error));
    }

    #[test]
    fn dualshock4_usb_and_bluetooth() {
        // On battery, level 6.
        assert_eq!(parse(Model::DualShock4, true, false, &report(0x11, 78, 32, 0x06)), Some(Report::Level(60, false)));
        // Cable in, level 4: charging.
        assert_eq!(parse(Model::DualShock4, false, false, &report(0x01, 64, 30, 0x14)), Some(Report::Level(40, true)));
        // Cable in, 11: full.
        assert_eq!(parse(Model::DualShock4, false, false, &report(0x01, 64, 30, 0x1b)), Some(Report::Level(100, false)));
        // Cable in, 14/15: unknown.
        assert_eq!(parse(Model::DualShock4, false, false, &report(0x01, 64, 30, 0x1e)), Some(Report::Error));
        // The adapter with no controller.
        let mut lost = report(0x01, 64, 30, 0x05);
        lost[31] = 0x04;
        assert_eq!(parse(Model::DualShock4, false, true, &lost), Some(Report::NoController));
        assert_eq!(parse(Model::DualShock4, false, false, &lost), Some(Report::Level(50, false)));
    }

    #[test]
    fn basic_bluetooth_reports_and_strangers() {
        assert_eq!(parse(Model::DualSense, true, false, &report(0x01, 78, 9, 0)), Some(Report::Basic));
        assert_eq!(parse(Model::DualShock4, true, false, &report(0x01, 10, 9, 0)), Some(Report::Basic));
        assert_eq!(parse(Model::DualSense, false, false, &report(0x05, 64, 9, 0)), None);
        assert_eq!(parse(Model::DualSense, false, false, &[0x01, 0, 0]), None);
        assert_eq!(parse(Model::DualSense, false, false, &[]), None);
        assert_eq!(model(0x0ce6).unwrap().0, Model::DualSense);
        assert!(model(0x0ba0).unwrap().2);
        assert!(model(0x1234).is_none());
    }
}
