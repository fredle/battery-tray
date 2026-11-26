//! Logitech devices over HID++ (Solaar's documentation and source are the
//! reference): mice and keyboards on a Unifying, Bolt or Lightspeed/nano
//! receiver, Logitech devices on Bluetooth LE that do not offer the GATT
//! battery, and wired HID++ 2.0 devices that have a battery.
//!
//! Reports: short 0x10 (7 bytes), long 0x11 (20 bytes):
//! `[report, device index, feature index or sub-ID, function << 4 | software ID or register, params…]`.
//! Windows gives the short and long reports their own HID collections
//! (vendor page 0xFF00, usages 1 and 2; 0xFF43/0x0202 for Bluetooth), so
//! both are opened and read together.
//!
//! - Receiver (HID++ 1.0, device index 0xFF): register 0x2B5 (long read
//!   0x83 0xB5) with sub-address 0x20+n-1 / 0x40+n-1 gives a Unifying-style
//!   receiver's pairing slot n (wireless PID, kind) and name; on Bolt it is
//!   0x50+n / 0x60+n. A slot that is not paired answers with an error.
//!   The receiver answers for a device even while the device sleeps.
//! - Device (HID++ 2.0, index 1-6 behind a receiver, 0xFF direct): IRoot
//!   (feature 0x0000) function 0 finds a feature's index; then Unified
//!   Battery 0x1004 (function 1, get status), Battery Status 0x1000
//!   (function 0) or Battery Voltage 0x1001 (function 0), and Device Name
//!   0x0005 for a direct device. Feature indexes are remembered per device.
//!
//! Only these queries are ever sent: nothing is written to a device or a
//! receiver. A device that does not answer (asleep, switched off) shows as
//! off, so the app keeps its last level. Every request has a short timeout
//! and the whole pass shares `hid::BUDGET`.
//!
//! Levels: Unified Battery's percentage when the device has one, otherwise
//! its band (critical, low, good, full) as a coarse level; Battery Status'
//! percentage; Battery Voltage only as a band, from the voltage curve
//! Solaar uses, because a percentage from a voltage would be a guess.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::devices::{Battery, Coarse, Kind, Precision, Reading, Transport};
use crate::hid::{Device, Info, read_any};

pub const VENDOR: u16 = 0x046d;

const SHORT: u8 = 0x10;
const LONG: u8 = 0x11;
const VERY_LONG: u8 = 0x12;
/// Marks our requests so replies to other software are not taken for ours.
const SOFTWARE_ID: u8 = 0x0b;
const RECEIVER: u8 = 0xff;

pub const FEATURE_NAME: u16 = 0x0005;
pub const FEATURE_BATTERY_STATUS: u16 = 0x1000;
pub const FEATURE_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEATURE_UNIFIED_BATTERY: u16 = 0x1004;

/// Receivers by product ID: Bolt uses its own pairing registers.
const BOLT: &[u16] = &[0xc548];
const RECEIVERS: &[u16] = &[
    0xc517, 0xc518, 0xc51a, 0xc51b, 0xc521, 0xc525, 0xc526, 0xc52b, 0xc52e, 0xc52f, 0xc531, 0xc532,
    0xc534, 0xc537, 0xc539, 0xc53a, 0xc53d, 0xc53f, 0xc541, 0xc545, 0xc547, 0xc548,
];

/// A HID++ 2.0 feature call: long report.
pub fn feature_request(index: u8, feature: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let mut r = vec![LONG, index, feature, (function << 4) | SOFTWARE_ID];
    r.extend_from_slice(params);
    r.resize(20, 0);
    r
}

/// A HID++ 1.0 long register read from the receiver: short report.
pub fn register_request(sub_address: u8, param: u8) -> Vec<u8> {
    vec![SHORT, RECEIVER, 0x83, 0xb5, sub_address, param, 0]
}

/// What a report means for a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The parameters after the four header bytes.
    Ok(Vec<u8>),
    Error(u8),
    /// Not an answer to this request (a notification, someone else's).
    Other,
}

pub fn reply_to(request: &[u8], report: &[u8]) -> Reply {
    if report.len() < 6 || ![SHORT, LONG, VERY_LONG].contains(&report[0]) || report[1] != request[1] {
        return Reply::Other;
    }
    // Errors: HID++ 2.0 (0xFF) and 1.0 (0x8F), echoing the request's two bytes.
    if (report[2] == 0xff || report[2] == 0x8f) && report[3] == request[2] && report[4] == request[3] {
        return Reply::Error(report[5]);
    }
    if report[2] != request[2] || report[3] != request[3] {
        return Reply::Other;
    }
    // A register reply also echoes the sub-address.
    let register = request[0] == SHORT && request[2] & 0x80 != 0;
    if register && report.get(4) != request.get(4) {
        return Reply::Other;
    }
    Reply::Ok(report[4..].to_vec())
}

/// HID++ 1.0 device kinds from receiver pairing info.
fn kind_from_receiver(code: u8) -> Kind {
    match code & 0x0f {
        1 | 3 => Kind::Keyboard,
        2 | 8 | 9 => Kind::Mouse,
        _ => Kind::Other,
    }
}

/// HID++ 2.0 Device Name feature, getDeviceType.
fn kind_from_device_type(code: u8) -> Kind {
    match code {
        0 | 2 => Kind::Keyboard,
        3..=5 => Kind::Mouse,
        8 => Kind::Headphones,
        _ => Kind::Other,
    }
}

/// A receiver's pairing slot: wireless PID and kind. `params` start with
/// the echoed sub-address.
pub fn parse_pairing(bolt: bool, params: &[u8]) -> Option<(u16, Kind)> {
    if bolt {
        Some((u16::from_le_bytes([*params.get(2)?, *params.get(3)?]), kind_from_receiver(*params.get(1)?)))
    } else {
        Some((u16::from_be_bytes([*params.get(3)?, *params.get(4)?]), kind_from_receiver(*params.get(7)?)))
    }
}

pub fn parse_name(bolt: bool, params: &[u8]) -> Option<String> {
    let (start, len) = if bolt {
        (3, (*params.get(2)? as usize).min(14))
    } else {
        (2, *params.get(1)? as usize)
    };
    let bytes = params.get(start..start + len)?;
    let name = String::from_utf8_lossy(bytes).trim_end_matches('\0').trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// A battery reading: level, its precision, and charging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub level: u8,
    pub precision: Precision,
    pub charging: bool,
}

/// Unified Battery (0x1004) get status: state of charge, level bits,
/// charging status, external power.
pub fn parse_unified(params: &[u8]) -> Option<Level> {
    let (soc, bits, status) = (*params.first()?, *params.get(1)?, *params.get(2)?);
    let charging = matches!(status, 1 | 2);
    if (1..=100).contains(&soc) {
        return Some(Level { level: soc, precision: Precision::Percent, charging });
    }
    let band = if bits & 8 != 0 {
        Coarse::Full
    } else if bits & 4 != 0 {
        Coarse::Medium
    } else if bits & 2 != 0 {
        Coarse::Low
    } else if bits & 1 != 0 {
        Coarse::VeryLow
    } else if status == 3 {
        // Charge complete with no level given.
        Coarse::Full
    } else {
        return None;
    };
    Some(Level { level: band.level(), precision: Precision::Coarse, charging })
}

/// Battery Status (0x1000) function 0: discharge level, next level, status.
pub fn parse_status(params: &[u8]) -> Option<Level> {
    let (level, status) = (*params.first()?, *params.get(2)?);
    // 1 recharging, 2 almost full, 4 slow recharge; 3 full; 5+ errors.
    let charging = matches!(status, 1 | 2 | 4);
    match (level, status) {
        (1..=100, 0..=4) => Some(Level { level, precision: Precision::Percent, charging }),
        (0, 3) => Some(Level { level: 100, precision: Precision::Percent, charging: false }),
        _ => None,
    }
}

/// Solaar's discharge curve for one Li-ion cell: (millivolts, percent).
const CURVE: &[(u16, u8)] = &[
    (4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50),
    (3778, 40), (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0),
];

/// Battery Voltage (0x1001) function 0: millivolts (big-endian) and flags.
/// Only a band comes out: a percentage from a voltage would be a guess.
pub fn parse_voltage(params: &[u8]) -> Option<Level> {
    let millivolts = u16::from_be_bytes([*params.first()?, *params.get(1)?]);
    let flags = *params.get(2)?;
    if millivolts == 0 {
        return None;
    }
    // Bit 7: on external power; then bits 0-1: 0 charging, 1 full, 2 not
    // charging.
    let powered = flags & 0x80 != 0;
    let full = powered && flags & 0x01 != 0;
    let charging = powered && flags & 0x03 == 0;
    let estimate = CURVE.iter().find(|(mv, _)| millivolts >= *mv).map_or(0, |(_, p)| *p);
    let band = if full {
        Coarse::Full
    } else if flags & 0x20 != 0 && flags & 0x18 == 0 {
        // The device says critical.
        Coarse::VeryLow
    } else {
        match estimate {
            0..=9 => Coarse::VeryLow,
            10..=29 => Coarse::Low,
            30..=69 => Coarse::Medium,
            _ => Coarse::Full,
        }
    };
    Some(Level { level: band.level(), precision: Precision::Coarse, charging })
}

/// How long one request may wait. Receivers answer in milliseconds; a
/// sleeping device not at all.
const REQUEST_WAIT: Duration = Duration::from_millis(350);

/// One receiver or device: its short and long collections, open.
struct Channel {
    short: Option<Device>,
    long: Device,
    deadline: Instant,
}

impl Channel {
    fn call(&mut self, request: &[u8]) -> Option<Reply> {
        let now = Instant::now();
        if now >= self.deadline {
            return None;
        }
        let until = (now + REQUEST_WAIT).min(self.deadline);
        let sent = match (request[0], self.short.as_mut()) {
            (SHORT, Some(short)) => short.write(request),
            (SHORT, None) => return None,
            _ => self.long.write(request),
        };
        if !sent {
            return None;
        }
        loop {
            let mut handles: Vec<&mut Device> = vec![&mut self.long];
            if let Some(short) = self.short.as_mut() {
                handles.push(short);
            }
            let (_, report) = read_any(&mut handles, until)?;
            match reply_to(request, &report) {
                Reply::Other => continue,
                reply => return Some(reply),
            }
        }
    }

    fn feature(&mut self, index: u8, feature: u8, function: u8, params: &[u8]) -> Option<Reply> {
        self.call(&feature_request(index, feature, function, params))
    }

    /// The index of a feature on a device; `Some(None)` when it does not
    /// have it, `None` when it did not answer.
    fn find(&mut self, index: u8, feature: u16) -> Option<Option<u8>> {
        // IRoot (0x0000) is always at index 0; function 0 is getFeature.
        match self.feature(index, 0, 0, &feature.to_be_bytes())? {
            Reply::Ok(params) => Some(params.first().copied().filter(|i| *i != 0)),
            Reply::Error(_) => None,
            Reply::Other => None,
        }
    }
}

/// Which battery feature a device has, and at which index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatteryFeature {
    Unified(u8),
    Status(u8),
    Voltage(u8),
    None,
}

/// What a device said: `None` if it did not answer.
fn battery(channel: &mut Channel, index: u8, feature: BatteryFeature) -> Option<Option<Level>> {
    let (feature_index, function, parse): (u8, u8, fn(&[u8]) -> Option<Level>) = match feature {
        BatteryFeature::Unified(i) => (i, 1, parse_unified),
        BatteryFeature::Status(i) => (i, 0, parse_status),
        BatteryFeature::Voltage(i) => (i, 0, parse_voltage),
        BatteryFeature::None => return Some(None),
    };
    match channel.feature(index, feature_index, function, &[])? {
        Reply::Ok(params) => Some(parse(&params)),
        _ => None,
    }
}

fn discover(channel: &mut Channel, index: u8) -> Option<BatteryFeature> {
    if let Some(i) = channel.find(index, FEATURE_UNIFIED_BATTERY)? {
        return Some(BatteryFeature::Unified(i));
    }
    if let Some(i) = channel.find(index, FEATURE_BATTERY_STATUS)? {
        return Some(BatteryFeature::Status(i));
    }
    if let Some(i) = channel.find(index, FEATURE_BATTERY_VOLTAGE)? {
        return Some(BatteryFeature::Voltage(i));
    }
    Some(BatteryFeature::None)
}

/// A direct device's name and kind from its Device Name feature.
fn direct_name(channel: &mut Channel) -> Option<(String, Kind)> {
    let feature = channel.find(RECEIVER, FEATURE_NAME)??;
    let Reply::Ok(count) = channel.feature(RECEIVER, feature, 0, &[])? else { return None };
    let length = (*count.first()? as usize).min(64);
    let mut name = Vec::new();
    while name.len() < length {
        let Reply::Ok(chunk) = channel.feature(RECEIVER, feature, 1, &[name.len() as u8])? else { break };
        let take = (length - name.len()).min(chunk.len());
        if take == 0 {
            break;
        }
        name.extend_from_slice(&chunk[..take]);
    }
    let kind = match channel.feature(RECEIVER, feature, 2, &[]) {
        Some(Reply::Ok(params)) => params.first().map_or(Kind::Other, |c| kind_from_device_type(*c)),
        _ => Kind::Other,
    };
    let name = String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_owned();
    (!name.is_empty()).then_some((name, kind))
}

/// The Logitech source. Remembers each device's battery feature, so a poll
/// usually costs one request per device.
#[derive(Default)]
pub struct Logitech {
    features: HashMap<String, BatteryFeature>,
}

impl Logitech {
    pub fn read(&mut self, infos: &[Info], deadline: Instant) -> Vec<Reading> {
        let mut readings = Vec::new();
        // The HID++ collections of one receiver or device share a parent.
        let mut groups: Vec<(u32, Option<&Info>, Option<&Info>)> = Vec::new();
        for info in infos.iter().filter(|i| i.vendor == VENDOR) {
            let slot = match (info.usage_page, info.usage) {
                (0xff00, 0x0001) => 0,
                (0xff00, 0x0002) | (0xff43, 0x0202) => 1,
                _ => continue,
            };
            let group = match groups.iter_mut().position(|g| g.0 == info.parent) {
                Some(i) => &mut groups[i],
                None => {
                    groups.push((info.parent, None, None));
                    groups.last_mut().unwrap()
                }
            };
            if slot == 0 { group.1 = Some(info) } else { group.2 = Some(info) }
        }
        for (_, short, long) in groups {
            let Some(long) = long else { continue };
            if Instant::now() >= deadline {
                break;
            }
            let Some(long_device) = Device::open(long, true) else { continue };
            let mut channel = Channel {
                short: short.and_then(|s| Device::open(s, true)),
                long: long_device,
                deadline,
            };
            if long.address.is_none() && RECEIVERS.contains(&long.product) {
                self.receiver(&mut channel, long, &mut readings);
            } else {
                self.direct(&mut channel, long, &mut readings);
            }
        }
        readings
    }

    fn receiver(&mut self, channel: &mut Channel, info: &Info, readings: &mut Vec<Reading>) {
        let bolt = BOLT.contains(&info.product);
        let mut any = false;
        for n in 1..=6u8 {
            let (pairing, name) = if bolt { (0x50 + n, 0x60 + n) } else { (0x20 + n - 1, 0x40 + n - 1) };
            let Some(Reply::Ok(params)) = channel.call(&register_request(pairing, 0)) else { continue };
            let Some((wpid, kind)) = parse_pairing(bolt, &params) else { continue };
            any = true;
            let name = match channel.call(&register_request(name, if bolt { 1 } else { 0 })) {
                Some(Reply::Ok(params)) => parse_name(bolt, &params),
                _ => None,
            };
            let mut id = format!("logi-{wpid:04x}");
            if readings.iter().any(|r: &Reading| r.id == id) {
                id = format!("{id}-{n}");
            }
            let reading = Reading::new(id, name.unwrap_or_default(), kind, Transport::Receiver);
            readings.push(self.ask(channel, n, reading));
        }
        // A Lightspeed or nano receiver that does not have the pairing
        // registers: its device is in slot 1.
        if !any && !bolt {
            let reading = Reading::new(format!("logi-{:04x}-1", info.product), "", Kind::Other, Transport::Receiver);
            let reading = self.ask(channel, 1, reading);
            if matches!(reading.battery, Battery::Level(_)) {
                readings.push(reading);
            }
        }
    }

    fn direct(&mut self, channel: &mut Channel, info: &Info, readings: &mut Vec<Reading>) {
        let (id, transport) = match &info.address {
            Some(address) => (address.clone(), Transport::Le),
            None => (format!("logi-usb-{:04x}", info.product), Transport::Usb),
        };
        let fresh = !self.features.contains_key(&id);
        let mut reading = Reading::new(id, "", Kind::Other, transport);
        if fresh && let Some((name, kind)) = direct_name(channel) {
            reading.name = name;
            reading.kind = kind;
        }
        let reading = self.ask(channel, RECEIVER, reading);
        // A wired device without a battery is not worth a card; over
        // Bluetooth the Bluetooth list shows it anyway.
        if matches!(reading.battery, Battery::Level(_)) || (transport == Transport::Le && reading.battery == Battery::Unreadable) {
            readings.push(reading);
        }
    }

    /// Fills in the battery, or marks the device off if it did not answer.
    fn ask(&mut self, channel: &mut Channel, index: u8, mut reading: Reading) -> Reading {
        let feature = match self.features.get(&reading.id) {
            Some(feature) => Some(*feature),
            None => discover(channel, index),
        };
        let Some(feature) = feature else {
            // Asleep or out of range.
            reading.connected = false;
            reading.battery = Battery::Unreadable;
            return reading;
        };
        self.features.insert(reading.id.clone(), feature);
        match battery(channel, index, feature) {
            Some(Some(level)) => {
                reading.battery = Battery::Level(level.level);
                reading.precision = level.precision;
                reading.charging = Some(level.charging);
            }
            Some(None) => reading.battery = Battery::NotReported,
            None => {
                reading.connected = false;
                reading.battery = Battery::Unreadable;
            }
        }
        reading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_laid_out_as_hidpp_says() {
        // IRoot getFeature(0x1004) to device 2.
        let r = feature_request(2, 0, 0, &FEATURE_UNIFIED_BATTERY.to_be_bytes());
        assert_eq!(r.len(), 20);
        assert_eq!(&r[..6], &[0x11, 0x02, 0x00, 0x0b, 0x10, 0x04]);
        // Unified Battery get_status at feature index 8: function 1.
        assert_eq!(&feature_request(1, 8, 1, &[])[..4], &[0x11, 0x01, 0x08, 0x1b]);
        // Unifying pairing info of slot 1.
        assert_eq!(register_request(0x20, 0), [0x10, 0xff, 0x83, 0xb5, 0x20, 0x00, 0x00]);
    }

    #[test]
    fn replies_errors_and_notifications_are_told_apart() {
        let request = feature_request(2, 0, 0, &[0x10, 0x04]);
        let mut ok = vec![0x11, 0x02, 0x00, 0x0b, 0x06, 0x00, 0x01];
        ok.resize(20, 0);
        assert_eq!(reply_to(&request, &ok), Reply::Ok(ok[4..].to_vec()));
        // HID++ 2.0 error: feature not supported (0x05)... rarely; any code.
        let mut error = vec![0x11, 0x02, 0xff, 0x00, 0x0b, 0x05];
        error.resize(20, 0);
        assert_eq!(reply_to(&request, &error), Reply::Error(5));
        // The receiver saying the device is unreachable (HID++ 1.0 error).
        assert_eq!(reply_to(&request, &[0x10, 0x02, 0x8f, 0x00, 0x0b, 0x09, 0x00]), Reply::Error(9));
        // A connection notification from the receiver, another device, and
        // another program's software ID.
        assert_eq!(reply_to(&request, &[0x10, 0x02, 0x41, 0x04, 0x61, 0x82, 0x40]), Reply::Other);
        assert_eq!(reply_to(&request, &[0x11, 0x03, 0x00, 0x0b, 0x06, 0, 0]), Reply::Other);
        assert_eq!(reply_to(&request, &[0x11, 0x02, 0x00, 0x01, 0x06, 0, 0]), Reply::Other);
        // A register reply for another slot is not ours.
        let register = register_request(0x21, 0);
        let mut other_slot = vec![0x11, 0xff, 0x83, 0xb5, 0x22];
        other_slot.resize(20, 0);
        assert_eq!(reply_to(&register, &other_slot), Reply::Other);
        // Unpaired slot.
        assert_eq!(reply_to(&register, &[0x10, 0xff, 0x8f, 0x83, 0xb5, 0x03, 0x00]), Reply::Error(3));
    }

    #[test]
    fn receiver_pairing_and_names() {
        // Unifying, slot 1: sub 0x20, destination, interval, wpid 0x4082
        // (big-endian), 2 more, kind mouse (0x02).
        let pairing = [0x20, 0x52, 0x08, 0x40, 0x82, 0x04, 0x00, 0x02, 0x07, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(parse_pairing(false, &pairing), Some((0x4082, Kind::Mouse)));
        let name = [0x40, 0x04, b'M', b'7', b'2', b'0', 0, 0];
        assert_eq!(parse_name(false, &name).as_deref(), Some("M720"));
        // Bolt, slot 1: sub 0x51, kind keyboard, wpid 0xB369 little-endian.
        let pairing = [0x51, 0x01, 0x69, 0xb3, 0x12, 0x34, 0x56, 0x78];
        assert_eq!(parse_pairing(true, &pairing), Some((0xb369, Kind::Keyboard)));
        let name = [0x61, 0x01, 0x07, b'M', b'X', b' ', b'K', b'e', b'y', b's', 0];
        assert_eq!(parse_name(true, &name).as_deref(), Some("MX Keys"));
        assert_eq!(parse_name(true, &[0x61, 0x01]), None);
        assert_eq!(parse_pairing(false, &[0x20, 0x00]), None);
    }

    #[test]
    fn unified_battery() {
        // 85%, good, discharging.
        assert_eq!(
            parse_unified(&[85, 0x04, 0x00, 0x00]),
            Some(Level { level: 85, precision: Precision::Percent, charging: false })
        );
        // 40%, charging.
        assert_eq!(parse_unified(&[40, 0x04, 0x01, 0x01]).unwrap().charging, true);
        // No percentage, only "low": a band.
        assert_eq!(
            parse_unified(&[0, 0x02, 0x00, 0x00]),
            Some(Level { level: Coarse::Low.level(), precision: Precision::Coarse, charging: false })
        );
        assert_eq!(parse_unified(&[0, 0x01, 0x00, 0x00]).unwrap().level, Coarse::VeryLow.level());
        assert_eq!(parse_unified(&[0, 0x08, 0x03, 0x01]).unwrap().level, Coarse::Full.level());
        assert_eq!(parse_unified(&[0, 0x00, 0x00, 0x00]), None);
        assert_eq!(parse_unified(&[50]), None);
    }

    #[test]
    fn battery_status() {
        assert_eq!(
            parse_status(&[50, 20, 0x00]),
            Some(Level { level: 50, precision: Precision::Percent, charging: false })
        );
        assert!(parse_status(&[90, 50, 0x01]).unwrap().charging);
        assert!(parse_status(&[90, 50, 0x02]).unwrap().charging);
        assert_eq!(parse_status(&[0, 0, 0x03]).unwrap().level, 100);
        // Invalid battery, thermal error.
        assert_eq!(parse_status(&[50, 20, 0x05]), None);
        assert_eq!(parse_status(&[0, 0, 0x00]), None);
    }

    #[test]
    fn battery_voltage_is_only_a_band() {
        // 4.10 V, discharging: full band.
        let full = parse_voltage(&[0x10, 0x04, 0x00]).unwrap();
        assert_eq!(full.precision, Precision::Coarse);
        assert_eq!(full.level, Coarse::Full.level());
        // 3.85 V: medium.
        assert_eq!(parse_voltage(&[0x0f, 0x0a, 0x00]).unwrap().level, Coarse::Medium.level());
        // 3.73 V: low.
        assert_eq!(parse_voltage(&[0x0e, 0x92, 0x00]).unwrap().level, Coarse::Low.level());
        // 3.60 V: very low.
        assert_eq!(parse_voltage(&[0x0e, 0x10, 0x00]).unwrap().level, Coarse::VeryLow.level());
        // Critical flag wins over the voltage.
        assert_eq!(parse_voltage(&[0x0f, 0x0a, 0x20]).unwrap().level, Coarse::VeryLow.level());
        // Charging; charging and full.
        let charging = parse_voltage(&[0x0f, 0x0a, 0x80]).unwrap();
        assert!(charging.charging);
        let full = parse_voltage(&[0x0f, 0x0a, 0x81]).unwrap();
        assert_eq!(full.level, Coarse::Full.level());
        assert!(!full.charging);
        // On power but not charging.
        assert!(!parse_voltage(&[0x0f, 0x0a, 0x82]).unwrap().charging);
        assert_eq!(parse_voltage(&[0x00, 0x00, 0x00]), None);
    }
}
