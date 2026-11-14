//! What the app knows about each Bluetooth device, and every rule that does
//! not need Windows: merging a fresh reading into what is remembered, when a
//! low-battery warning fires, which device the tray icon shows, and the text
//! of the tooltip. All of it is plain functions so it can be unit-tested.

use serde::{Deserialize, Serialize};

use crate::license;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Headphones,
    Mouse,
    Keyboard,
    Gamepad,
    #[default]
    Other,
}

impl Kind {
    /// From a classic Bluetooth class-of-device value: major class in bits
    /// 8-12, minor class in bits 2-7.
    pub fn from_class_of_device(raw: u32) -> Kind {
        let major = (raw >> 8) & 0x1f;
        let minor = (raw >> 2) & 0x3f;
        match major {
            4 => Kind::Headphones,
            5 => match (minor & 0x30, minor & 0x0f) {
                (0x10, _) | (0x30, _) => Kind::Keyboard,
                (0x20, _) => Kind::Mouse,
                (_, 1) | (_, 2) => Kind::Gamepad,
                _ => Kind::Other,
            },
            _ => Kind::Other,
        }
    }

    /// From a Bluetooth LE appearance value: category in the top ten bits,
    /// subcategory in the low six.
    pub fn from_appearance(raw: u16) -> Kind {
        match (raw >> 6, raw & 0x3f) {
            (0x0f, 1) => Kind::Keyboard,
            (0x0f, 2) => Kind::Mouse,
            (0x0f, 3) | (0x0f, 4) => Kind::Gamepad,
            // Audio sink, and wearable audio (earbuds, headsets, headphones).
            (0x21, _) | (0x25, _) => Kind::Headphones,
            _ => Kind::Other,
        }
    }
}

/// What a connected device said about its battery on the latest poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Battery {
    /// Percent, 0-100.
    Level(u8),
    /// The device has no battery level that Windows can read.
    NotReported,
    /// It should have one, but the read failed this time.
    #[default]
    Unreadable,
}

/// One device as a source saw it on one poll.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// Stable identity: the Bluetooth address as 12 lowercase hex digits.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Bluetooth LE rather than classic Bluetooth.
    pub le: bool,
    pub connected: bool,
    pub battery: Battery,
}

/// A paired device: what is remembered between runs, plus the latest poll.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub le: bool,
    /// The last level actually read, and when (Unix seconds).
    #[serde(default)]
    pub last_level: Option<u8>,
    #[serde(default)]
    pub last_seen: u64,
    /// Whether the next drop to the threshold should warn. Cleared when a
    /// warning is shown, set again once the level is back above it.
    #[serde(default = "default_true")]
    pub armed: bool,
    #[serde(skip)]
    pub connected: bool,
    #[serde(skip)]
    pub battery: Battery,
}

fn default_true() -> bool {
    true
}

/// What to show for a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Live(u8),
    NotReported,
    Unreadable,
    /// Off or out of range, with the last level read and when, if any.
    Off(Option<(u8, u64)>),
}

impl Device {
    pub fn state(&self) -> State {
        if !self.connected {
            return State::Off(self.last_level.map(|level| (level, self.last_seen)));
        }
        match self.battery {
            Battery::Level(level) => State::Live(level),
            Battery::NotReported => State::NotReported,
            Battery::Unreadable => State::Unreadable,
        }
    }

    pub fn live_level(&self) -> Option<u8> {
        match self.state() {
            State::Live(level) => Some(level),
            _ => None,
        }
    }
}

/// A low-battery warning that should be shown now.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub name: String,
    pub level: u8,
}

/// One step of the warning rule for one device: returns whether it stays
/// armed, and whether to warn now. A warning fires once when the level
/// reaches the threshold and not again until it has risen above it.
pub fn warn_step(armed: bool, level: u8, threshold: u8) -> (bool, bool) {
    if level <= threshold {
        (false, armed)
    } else {
        (true, false)
    }
}

/// The device the tray icon shows: the user's choice if it is still paired,
/// otherwise the first device that has ever reported a level, otherwise the
/// first device.
pub fn tray_device<'a>(devices: &'a [Device], chosen: Option<&str>) -> Option<&'a Device> {
    chosen
        .and_then(|id| devices.iter().find(|d| d.id == id))
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.live_level().is_some() || d.last_level.is_some())
        })
        .or_else(|| devices.first())
}

/// Folds one poll into the remembered devices and returns the warnings to
/// show. Devices missing from `readings` are no longer paired and are
/// forgotten. Connected devices come first, then by name.
pub fn apply(
    devices: &mut Vec<Device>,
    readings: Vec<Reading>,
    now: u64,
    threshold: u8,
    chosen: Option<&str>,
    licensed: bool,
) -> Vec<Warning> {
    let mut merged: Vec<Device> = Vec::with_capacity(readings.len());
    for reading in readings {
        if merged.iter().any(|d| d.id == reading.id) {
            continue;
        }
        let old = devices.iter().find(|d| d.id == reading.id);
        let mut device = Device {
            id: reading.id,
            name: reading.name,
            kind: reading.kind,
            le: reading.le,
            last_level: old.and_then(|d| d.last_level),
            last_seen: old.map_or(0, |d| d.last_seen),
            armed: old.is_none_or(|d| d.armed),
            connected: reading.connected,
            battery: reading.battery,
        };
        if let Some(level) = device.live_level() {
            device.last_level = Some(level);
            device.last_seen = now;
        }
        merged.push(device);
    }
    merged.sort_by_key(|d| (!d.connected, d.name.to_lowercase()));

    let tray = tray_device(&merged, chosen).map(|d| d.id.clone());
    let mut warnings = Vec::new();
    for device in &mut merged {
        let Some(level) = device.live_level() else {
            continue;
        };
        // A device the free tier does not warn about stays armed, so the
        // warning is not lost if it becomes the chosen one or is unlocked.
        if !license::warns_about(&device.id, tray.as_deref(), licensed) {
            continue;
        }
        let (armed, fire) = warn_step(device.armed, level, threshold);
        device.armed = armed;
        if fire {
            warnings.push(Warning {
                name: device.name.clone(),
                level,
            });
        }
    }
    *devices = merged;
    warnings
}

/// Devices that are low right now and that this licence warns about, for the
/// banner in the window.
pub fn low_now<'a>(
    devices: &'a [Device],
    threshold: u8,
    chosen: Option<&str>,
    licensed: bool,
) -> Vec<(&'a Device, u8)> {
    let tray = tray_device(devices, chosen).map(|d| d.id.as_str());
    devices
        .iter()
        .filter_map(|d| Some((d, d.live_level()?)))
        .filter(|(d, level)| *level <= threshold && license::warns_about(&d.id, tray, licensed))
        .collect()
}

/// "WH-1000XM4: 82%", for the tooltip and the tray menu.
pub fn summary_line(device: &Device, off: &str) -> String {
    let value = match device.state() {
        State::Live(level) => format!("{level}%"),
        State::Off(_) => off.to_owned(),
        State::NotReported | State::Unreadable => "–".to_owned(),
    };
    format!("{}: {value}", device.name)
}

/// Windows allows a tray tooltip 127 UTF-16 units.
pub const TOOLTIP_UNITS: usize = 127;

/// Every device on its own line, cut to what a tray tooltip can hold.
pub fn tooltip(devices: &[Device], off: &str, none: &str) -> String {
    if devices.is_empty() {
        return none.to_owned();
    }
    let full = devices
        .iter()
        .map(|d| summary_line(d, off))
        .collect::<Vec<_>>()
        .join("\n");
    if full.encode_utf16().count() <= TOOLTIP_UNITS {
        return full;
    }
    let mut cut = String::new();
    let mut units = 0;
    for c in full.chars() {
        units += c.len_utf16();
        if units > TOOLTIP_UNITS - 1 {
            break;
        }
        cut.push(c);
    }
    cut.push('…');
    cut
}

/// How long ago something was, in the largest sensible unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ago {
    Now,
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

pub fn ago(now: u64, then: u64) -> Ago {
    let seconds = now.saturating_sub(then);
    match seconds {
        0..=89 => Ago::Now,
        90..=3599 => Ago::Minutes((seconds + 30) / 60),
        3600..=86_399 => Ago::Hours(seconds / 3600),
        _ => Ago::Days(seconds / 86_400),
    }
}

/// The Bluetooth address inside a Windows device instance ID such as
/// `BTHENUM\{0000111E-…}_VID&0001004C_PID&2014\7&2D3E&0&AABBCCDDEEFF_C00000000`
/// or `BTHLE\DEV_AABBCCDDEEFF\8&1A2B&0&AABBCCDDEEFF`, as 12 lowercase hex
/// digits.
pub fn address_in_instance_id(instance_id: &str) -> Option<String> {
    instance_id
        .split(['\\', '&', '_'])
        .rev()
        .find(|part| part.len() == 12 && part.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_lowercase)
}

pub fn address_id(address: u64) -> String {
    format!("{address:012x}")
}

fn path() -> std::path::PathBuf {
    crate::settings::data_dir().join("devices.json")
}

/// The remembered devices: last level, when, and the warning state.
pub fn load() -> Vec<Device> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(devices: &[Device]) {
    let _ = std::fs::create_dir_all(crate::settings::data_dir());
    if let Ok(json) = serde_json::to_string_pretty(devices) {
        let _ = std::fs::write(path(), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(id: &str, name: &str, connected: bool, battery: Battery) -> Reading {
        Reading {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: Kind::Other,
            le: false,
            connected,
            battery,
        }
    }

    #[test]
    fn warning_fires_once_per_discharge_and_rearms_above_the_threshold() {
        // Armed, above: nothing. Reaches the threshold: fire and disarm.
        assert_eq!(warn_step(true, 21, 20), (true, false));
        assert_eq!(warn_step(true, 20, 20), (false, true));
        // Still low: quiet. Exactly at the threshold is still low.
        assert_eq!(warn_step(false, 12, 20), (false, false));
        assert_eq!(warn_step(false, 20, 20), (false, false));
        // Charged above it: armed again, and the next drop fires.
        assert_eq!(warn_step(false, 21, 20), (true, false));
        assert_eq!(warn_step(true, 5, 20), (false, true));
    }

    #[test]
    fn a_discharge_warns_once_across_polls() {
        let mut devices = Vec::new();
        let mut fired = Vec::new();
        for (now, level) in [(1, 30), (2, 19), (3, 15), (4, 80), (5, 20)] {
            let readings = vec![reading("a", "Mouse", true, Battery::Level(level))];
            let warnings = apply(&mut devices, readings, now, 20, None, false);
            fired.push(warnings.len());
        }
        assert_eq!(fired, [0, 1, 0, 0, 1]);
        assert_eq!(devices[0].last_level, Some(20));
        assert_eq!(devices[0].last_seen, 5);
    }

    #[test]
    fn free_warns_about_the_tray_device_only_and_unlock_warns_about_all() {
        let low = || {
            vec![
                reading("a", "Alpha", true, Battery::Level(10)),
                reading("b", "Beta", true, Battery::Level(10)),
            ]
        };
        if license::paid_tier_enabled() {
            let mut devices = Vec::new();
            let warnings = apply(&mut devices, low(), 1, 20, Some("b"), false);
            assert_eq!(warnings, [Warning { name: "Beta".into(), level: 10 }]);
            // Alpha was not warned about, so it is still armed: unlocking
            // (or choosing it) warns straight away.
            let warnings = apply(&mut devices, low(), 2, 20, Some("b"), true);
            assert_eq!(warnings, [Warning { name: "Alpha".into(), level: 10 }]);
            assert_eq!(low_now(&devices, 20, Some("b"), false).len(), 1);
        }
        let mut devices = Vec::new();
        assert_eq!(apply(&mut devices, low(), 1, 20, Some("b"), true).len(), 2);
        assert_eq!(low_now(&devices, 20, Some("b"), true).len(), 2);
    }

    #[test]
    fn a_device_that_goes_off_keeps_its_last_level_and_time() {
        let mut devices = Vec::new();
        apply(&mut devices, vec![reading("a", "Buds", true, Battery::Level(64))], 100, 20, None, false);
        assert_eq!(devices[0].state(), State::Live(64));
        apply(&mut devices, vec![reading("a", "Buds", false, Battery::Unreadable)], 200, 20, None, false);
        assert_eq!(devices[0].state(), State::Off(Some((64, 100))));
        // Unpaired: forgotten.
        apply(&mut devices, Vec::new(), 300, 20, None, false);
        assert!(devices.is_empty());
    }

    #[test]
    fn a_device_without_a_level_is_never_given_one() {
        let mut devices = Vec::new();
        let readings = vec![
            reading("a", "Old mouse", true, Battery::NotReported),
            reading("b", "Flaky", true, Battery::Unreadable),
            reading("c", "Never seen", false, Battery::Unreadable),
        ];
        assert!(apply(&mut devices, readings, 1, 20, None, true).is_empty());
        let state = |id: &str| devices.iter().find(|d| d.id == id).unwrap().state();
        assert_eq!(state("a"), State::NotReported);
        assert_eq!(state("b"), State::Unreadable);
        assert_eq!(state("c"), State::Off(None));
        assert!(devices.iter().all(|d| d.last_level.is_none()));
    }

    #[test]
    fn connected_devices_sort_first_then_by_name() {
        let mut devices = Vec::new();
        let readings = vec![
            reading("a", "zeta", false, Battery::Unreadable),
            reading("b", "Beta", true, Battery::Level(50)),
            reading("c", "alpha", true, Battery::Level(50)),
            reading("c", "duplicate", true, Battery::Level(1)),
        ];
        apply(&mut devices, readings, 1, 20, None, false);
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Beta", "zeta"]);
    }

    #[test]
    fn tray_device_is_the_choice_or_the_first_with_a_level() {
        let mut devices = Vec::new();
        let readings = vec![
            reading("a", "A", true, Battery::NotReported),
            reading("b", "B", true, Battery::Level(40)),
        ];
        apply(&mut devices, readings, 1, 20, None, false);
        assert_eq!(tray_device(&devices, None).unwrap().id, "b");
        assert_eq!(tray_device(&devices, Some("a")).unwrap().id, "a");
        assert_eq!(tray_device(&devices, Some("gone")).unwrap().id, "b");
        assert_eq!(tray_device(&devices[..1], None).unwrap().id, "a");
        assert!(tray_device(&[], None).is_none());
    }

    #[test]
    fn tooltip_lists_every_device_and_fits_windows_limit() {
        let mut devices = Vec::new();
        let readings = vec![
            reading("a", "Buds", true, Battery::Level(82)),
            reading("b", "Mouse", false, Battery::Unreadable),
            reading("c", "Pad", true, Battery::NotReported),
        ];
        apply(&mut devices, readings, 1, 20, None, false);
        assert_eq!(tooltip(&devices, "off", "none"), "Buds: 82%\nPad: –\nMouse: off");
        assert_eq!(tooltip(&[], "off", "none"), "none");

        let many: Vec<Reading> = (0..20)
            .map(|i| reading(&i.to_string(), &format!("Наушники {i:02}"), true, Battery::Level(50)))
            .collect();
        apply(&mut devices, many, 1, 20, None, false);
        let text = tooltip(&devices, "off", "none");
        assert_eq!(text.encode_utf16().count(), TOOLTIP_UNITS);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn ago_picks_a_unit() {
        assert_eq!(ago(100, 100), Ago::Now);
        assert_eq!(ago(100, 200), Ago::Now);
        assert_eq!(ago(189, 100), Ago::Now);
        assert_eq!(ago(190, 100), Ago::Minutes(2));
        assert_eq!(ago(3599, 0), Ago::Minutes(60));
        assert_eq!(ago(3600, 0), Ago::Hours(1));
        assert_eq!(ago(86_399, 0), Ago::Hours(23));
        assert_eq!(ago(3 * 86_400 + 5, 0), Ago::Days(3));
    }

    #[test]
    fn finds_the_address_in_windows_instance_ids() {
        for (id, address) in [
            (r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0001004C_PID&2014\7&2D3E4F5A&0&AABBCCDDEEFF_C00000000", Some("aabbccddeeff")),
            (r"BTHENUM\DEV_AABBCCDDEEFF\7&2D3E4F5A&0&BLUETOOTHDEVICE_AABBCCDDEEFF", Some("aabbccddeeff")),
            (r"BTHLE\DEV_E1A2B3C4D5E6\8&1A2B3C4D&0&E1A2B3C4D5E6", Some("e1a2b3c4d5e6")),
            (r"BTHLE\{0000180F-0000-1000-8000-00805F9B34FB}_DEV_VID&02046D_PID&B023_REV&0010_E1A2B3C4D5E6\9&2F&0&0020", Some("e1a2b3c4d5e6")),
            (r"USB\VID_046D&PID_C52B\6&1A2B3C4D&0&1", None),
        ] {
            assert_eq!(address_in_instance_id(id).as_deref(), address, "{id}");
        }
        assert_eq!(address_id(0xAABB_CCDD_EEFF), "aabbccddeeff");
        assert_eq!(address_id(0x1), "000000000001");
    }

    #[test]
    fn kinds_from_bluetooth_class_and_appearance() {
        // Headphones 0x240418, keyboard 0x002540, mouse 0x002580, gamepad 0x002508.
        assert_eq!(Kind::from_class_of_device(0x24_0418), Kind::Headphones);
        assert_eq!(Kind::from_class_of_device(0x00_2540), Kind::Keyboard);
        assert_eq!(Kind::from_class_of_device(0x00_2580), Kind::Mouse);
        assert_eq!(Kind::from_class_of_device(0x00_2508), Kind::Gamepad);
        assert_eq!(Kind::from_class_of_device(0x5A_020C), Kind::Other);
        assert_eq!(Kind::from_appearance(0x03C1), Kind::Keyboard);
        assert_eq!(Kind::from_appearance(0x03C2), Kind::Mouse);
        assert_eq!(Kind::from_appearance(0x03C4), Kind::Gamepad);
        assert_eq!(Kind::from_appearance(0x0941), Kind::Headphones);
        assert_eq!(Kind::from_appearance(0x0000), Kind::Other);
    }

    #[test]
    fn remembered_devices_round_trip_without_the_live_part() {
        let mut devices = Vec::new();
        apply(&mut devices, vec![reading("a", "Buds", true, Battery::Level(15))], 7, 20, None, false);
        let json = serde_json::to_string(&devices).unwrap();
        let back: Vec<Device> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].last_level, Some(15));
        assert_eq!(back[0].last_seen, 7);
        assert!(!back[0].armed);
        assert_eq!(back[0].state(), State::Off(Some((15, 7))));
        // Older or hand-written files without the optional fields still load.
        let minimal: Vec<Device> = serde_json::from_str(r#"[{"id":"a","name":"Buds"}]"#).unwrap();
        assert!(minimal[0].armed);
    }
}
