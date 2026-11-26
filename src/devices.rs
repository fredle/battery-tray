//! What the app knows about each device, and every rule that does not need
//! Windows: merging a fresh reading into what is remembered, when a
//! low-battery warning fires, which device the tray icon shows, and the text
//! of the tooltip. All of it is plain functions so it can be unit-tested.

use serde::{Deserialize, Serialize};

use crate::i18n::Strings;
use crate::license;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Headphones,
    Mouse,
    Keyboard,
    Gamepad,
    /// The PC's own battery.
    Laptop,
    #[default]
    Other,
}

/// How the device reaches this PC, for the line under the card.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// Classic Bluetooth.
    #[default]
    Bluetooth,
    /// Bluetooth Low Energy.
    #[serde(alias = "bluetoothle")]
    Le,
    /// A wireless USB receiver: Logitech Unifying/Bolt/Lightspeed, Xbox
    /// Wireless Adapter, Sony's DualShock 4 dongle.
    Receiver,
    /// A USB cable.
    Usb,
    /// Inside this PC (the laptop battery).
    Internal,
}

/// How exact a level is. The number is stored the same way for all three
/// so the tray icon, the bar and the threshold keep working; only the text
/// differs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// Percent, as the device said it.
    #[default]
    Percent,
    /// The device reports in steps of ten: 70 means 70-79% (AirPods,
    /// DualSense, DualShock 4).
    Tens,
    /// Only a band: one of `Coarse::level()` (XInput, Logitech devices
    /// without a percentage). Shown as a word, never as a number.
    Coarse,
}

/// The bands a coarse device reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coarse {
    /// XInput "empty", Logitech "critical".
    VeryLow,
    Low,
    /// XInput "medium", Logitech "good".
    Medium,
    Full,
}

impl Coarse {
    /// The number stored for a band: where the bar is drawn. Low and below
    /// count as low whatever the threshold (see `warn_level`).
    pub fn level(self) -> u8 {
        match self {
            Coarse::VeryLow => 5,
            Coarse::Low => 25,
            Coarse::Medium => 60,
            Coarse::Full => 100,
        }
    }

    pub fn from_level(level: u8) -> Coarse {
        match level {
            0..=5 => Coarse::VeryLow,
            6..=25 => Coarse::Low,
            26..=60 => Coarse::Medium,
            _ => Coarse::Full,
        }
    }
}

/// One part of a device with its own battery: an earbud or the case.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PartKind {
    Left,
    Right,
    Case,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub kind: PartKind,
    /// Percent in the device's own precision; `None` when this part is not
    /// reporting right now (a bud out of the case and out of range).
    #[serde(default)]
    pub level: Option<u8>,
    #[serde(default)]
    pub charging: bool,
}

/// The device's headline level from its parts: the lowest earbud. The case
/// never decides it; with no bud reporting there is no headline.
pub fn headline(parts: &[Part]) -> Option<u8> {
    parts
        .iter()
        .filter(|p| p.kind != PartKind::Case)
        .filter_map(|p| p.level)
        .min()
}

/// Whether the part that sets the headline is charging.
pub fn headline_charging(parts: &[Part]) -> Option<bool> {
    let low = headline(parts)?;
    Some(
        parts
            .iter()
            .filter(|p| p.kind != PartKind::Case && p.level == Some(low))
            .all(|p| p.charging),
    )
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
    /// Stable identity. A Bluetooth device (and anything reached over
    /// Bluetooth, such as a DualSense or a Logitech mouse) uses its address
    /// as 12 lowercase hex digits, so every source that sees the same device
    /// names it the same way; others use a prefix such as `logi-`,
    /// `xinput-`, `sony-` or `this-pc`.
    pub id: String,
    /// Empty when the source does not know it (a sleeping Logitech device):
    /// the remembered name is kept.
    pub name: String,
    pub kind: Kind,
    pub transport: Transport,
    pub connected: bool,
    /// The headline level: for a device with parts, its lowest earbud.
    pub battery: Battery,
    pub precision: Precision,
    /// `None` when the source cannot tell.
    pub charging: Option<bool>,
    /// Earbuds and case, for AirPods and Beats; empty for everything else.
    pub parts: Vec<Part>,
    /// Bluetooth vendor and product ID from Windows' device ID (Apple is
    /// 0x004C), used to match AirPods advertisements to the paired device.
    pub hardware: Option<(u16, u16)>,
}

impl Reading {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: Kind, transport: Transport) -> Reading {
        Reading {
            id: id.into(),
            name: name.into(),
            kind,
            transport,
            connected: true,
            battery: Battery::NotReported,
            precision: Precision::Percent,
            charging: None,
            parts: Vec::new(),
            hardware: None,
        }
    }
}

/// A paired device: what is remembered between runs, plus the latest poll.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// v0.1.4 wrote `le` instead; `load` turns it into `Transport::Le`.
    #[serde(default)]
    pub transport: Transport,
    /// The precision of `last_level`.
    #[serde(default)]
    pub precision: Precision,
    /// The last level actually read, and when (Unix seconds).
    #[serde(default)]
    pub last_level: Option<u8>,
    #[serde(default)]
    pub last_seen: u64,
    /// Whether the next drop to the threshold should warn. Cleared when a
    /// warning is shown, set again once the level is back above it.
    #[serde(default = "default_true")]
    pub armed: bool,
    /// Earbuds and case: live while connected, the last ones seen when off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(skip)]
    pub connected: bool,
    #[serde(skip)]
    pub battery: Battery,
    #[serde(skip)]
    pub charging: Option<bool>,
    /// Read from files written by v0.1.4, never written.
    #[serde(default, rename = "le", skip_serializing)]
    legacy_le: bool,
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

    /// Charging right now, as far as the source knows.
    pub fn is_charging(&self) -> bool {
        self.connected && self.charging == Some(true)
    }
}

/// The level the warning rule compares with the threshold. A coarse device
/// that says "low" is low whatever the threshold, and one that says
/// "medium" or "full" is not.
pub fn warn_level(level: u8, precision: Precision) -> u8 {
    match precision {
        Precision::Coarse => match Coarse::from_level(level) {
            Coarse::VeryLow | Coarse::Low => 0,
            Coarse::Medium | Coarse::Full => 100,
        },
        _ => level,
    }
}

/// A level as text: "80%", or a word for a coarse level.
pub fn level_text(level: u8, precision: Precision, t: &Strings) -> String {
    match precision {
        Precision::Coarse => match Coarse::from_level(level) {
            Coarse::VeryLow => t.level_very_low.to_owned(),
            Coarse::Low => t.level_low.to_owned(),
            Coarse::Medium => t.level_medium.to_owned(),
            Coarse::Full => t.level_full.to_owned(),
        },
        Precision::Percent | Precision::Tens => format!("{level}%"),
    }
}

/// Fills a text with a `{level}` placeholder written as a percentage
/// ("{level}%", "{level} %", "%{level}"): a coarse level replaces the
/// percentage with its word.
pub fn fill_level(template: &str, level: u8, precision: Precision, t: &Strings, values: &[(&str, &str)]) -> String {
    let (template, level) = match precision {
        Precision::Coarse => (
            template.replace("{level} %", "{level}").replace("{level}%", "{level}").replace("%{level}", "{level}"),
            level_text(level, precision, t),
        ),
        _ => (template.to_owned(), level.to_string()),
    };
    let mut all = vec![("level", level.as_str())];
    all.extend_from_slice(values);
    crate::i18n::fill(&template, &all)
}

/// "L 80% · R 70% · Case 40%"; a charging part gets a "+" after it. Parts
/// that are not reporting show a dash.
pub fn parts_text(parts: &[Part], precision: Precision, t: &Strings) -> String {
    parts
        .iter()
        .map(|part| {
            let label = match part.kind {
                PartKind::Left => t.part_left,
                PartKind::Right => t.part_right,
                PartKind::Case => t.part_case,
            };
            let value = match part.level {
                Some(level) => level_text(level, precision, t),
                None => "–".to_owned(),
            };
            let plus = if part.charging && part.level.is_some() { "+" } else { "" };
            format!("{label} {value}{plus}")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How the device is connected, as the card and tooltip say it.
pub fn transport_text(transport: Transport, t: &Strings) -> &'static str {
    match transport {
        Transport::Bluetooth => "Bluetooth",
        Transport::Le => "Bluetooth LE",
        Transport::Receiver => t.transport_receiver,
        Transport::Usb => "USB",
        Transport::Internal => t.transport_internal,
    }
}

/// A low-battery warning that should be shown now.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub name: String,
    pub level: u8,
    pub precision: Precision,
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
        let name = if reading.name.is_empty() {
            old.map(|d| d.name.clone()).unwrap_or_default()
        } else {
            reading.name
        };
        let mut device = Device {
            id: reading.id,
            name,
            kind: reading.kind,
            transport: reading.transport,
            precision: old.map_or(reading.precision, |d| d.precision),
            last_level: old.and_then(|d| d.last_level),
            last_seen: old.map_or(0, |d| d.last_seen),
            armed: old.is_none_or(|d| d.armed),
            // Parts seen now replace the remembered ones; a device that
            // is off keeps the last ones it showed.
            parts: if reading.connected && !reading.parts.is_empty() {
                reading.parts
            } else {
                old.map(|d| d.parts.clone()).unwrap_or_default()
            },
            connected: reading.connected,
            battery: reading.battery,
            charging: reading.charging,
            legacy_le: false,
        };
        if let Some(level) = device.live_level() {
            device.last_level = Some(level);
            device.last_seen = now;
            device.precision = reading.precision;
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
        let (armed, fire) = warn_step(device.armed, warn_level(level, device.precision), threshold);
        if device.is_charging() {
            // No warning while it charges; rising above the threshold on
            // the charger still re-arms it, and unplugging it while low
            // warns on the next poll.
            if armed {
                device.armed = true;
            }
            continue;
        }
        device.armed = armed;
        if fire {
            warnings.push(Warning {
                name: device.name.clone(),
                level,
                precision: device.precision,
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
        .filter(|(d, level)| {
            warn_level(*level, d.precision) <= threshold
                && !d.is_charging()
                && license::warns_about(&d.id, tray, licensed)
        })
        .collect()
}

/// "WH-1000XM4: 82%", "AirPods Pro: L 80% · R 70% · Case 40%",
/// "Controller: Medium (charging)", for the tooltip and the tray menu.
pub fn summary_line(device: &Device, t: &Strings) -> String {
    let mut value = match device.state() {
        State::Live(_) if !device.parts.is_empty() => parts_text(&device.parts, device.precision, t),
        State::Live(level) => level_text(level, device.precision, t),
        State::Off(_) => t.off_short.to_owned(),
        State::NotReported | State::Unreadable => "–".to_owned(),
    };
    if device.is_charging() {
        value = format!("{value} ({})", t.charging);
    }
    format!("{}: {value}", device.name)
}

/// Windows allows a tray tooltip 127 UTF-16 units.
pub const TOOLTIP_UNITS: usize = 127;

/// Every device on its own line, cut to what a tray tooltip can hold.
pub fn tooltip(devices: &[Device], t: &Strings) -> String {
    if devices.is_empty() {
        return t.tray_none.to_owned();
    }
    let full = devices
        .iter()
        .map(|d| summary_line(d, t))
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
        .split(['\\', '&', '_', '#'])
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

/// The Bluetooth vendor and product ID in a Windows device instance ID such
/// as `BTHENUM\{…}_VID&0001004C_PID&2014\…` or `…_DEV_VID&02046D_PID&B023_…`:
/// the last four hex digits of each.
pub fn hardware_in_instance_id(instance_id: &str) -> Option<(u16, u16)> {
    let upper = instance_id.to_ascii_uppercase();
    let hex_after = |key: &str| -> Option<u16> {
        let start = upper.find(key)? + key.len();
        let digits: String = upper[start..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if digits.len() < 4 {
            return None;
        }
        u16::from_str_radix(&digits[digits.len() - 4..], 16).ok()
    };
    Some((hex_after("VID&")?, hex_after("PID&")?))
}

/// The remembered devices: last level, when, and the warning state.
pub fn load() -> Vec<Device> {
    parse_saved(&std::fs::read_to_string(path()).unwrap_or_default())
}

/// A saved devices.json, from this version or an older one.
pub fn parse_saved(json: &str) -> Vec<Device> {
    let mut devices: Vec<Device> = serde_json::from_str(json).unwrap_or_default();
    for device in &mut devices {
        if std::mem::take(&mut device.legacy_le) && device.transport == Transport::Bluetooth {
            device.transport = Transport::Le;
        }
    }
    devices
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

    use crate::i18n::Lang;

    fn reading(id: &str, name: &str, connected: bool, battery: Battery) -> Reading {
        Reading {
            connected,
            battery,
            ..Reading::new(id, name, Kind::Other, Transport::Bluetooth)
        }
    }

    fn buds(left: Option<u8>, right: Option<u8>, case: Option<u8>, charging: bool) -> Vec<Part> {
        vec![
            Part { kind: PartKind::Left, level: left, charging },
            Part { kind: PartKind::Right, level: right, charging },
            Part { kind: PartKind::Case, level: case, charging: false },
        ]
    }

    #[test]
    fn the_headline_is_the_lowest_bud_never_the_case() {
        assert_eq!(headline(&buds(Some(80), Some(70), Some(10), false)), Some(70));
        assert_eq!(headline(&buds(None, Some(90), Some(10), false)), Some(90));
        assert_eq!(headline(&buds(None, None, Some(10), false)), None);
        assert_eq!(headline_charging(&buds(Some(80), Some(70), None, true)), Some(true));
        let mut parts = buds(Some(80), Some(70), None, false);
        parts[0].charging = true;
        assert_eq!(headline_charging(&parts), Some(false));
        assert_eq!(headline_charging(&[]), None);
    }

    #[test]
    fn no_warning_while_charging_and_one_after_unplugging_low() {
        let mut devices = Vec::new();
        let mut fired = Vec::new();
        for (now, level, charging) in [(1, 30, false), (2, 15, true), (3, 10, true), (4, 10, false), (5, 9, false)] {
            let readings = vec![Reading {
                charging: Some(charging),
                ..reading("a", "Pad", true, Battery::Level(level))
            }];
            fired.push(apply(&mut devices, readings, now, 20, None, true).len());
        }
        assert_eq!(fired, [0, 0, 0, 1, 0]);
        assert!(low_now(&devices, 20, None, true).len() == 1);
        // Charged on the cable above the threshold: re-armed for next time.
        let readings = vec![Reading { charging: Some(true), ..reading("a", "Pad", true, Battery::Level(90)) }];
        apply(&mut devices, readings, 6, 20, None, true);
        assert!(devices[0].armed);
        assert!(low_now(&devices, 20, None, true).is_empty());
    }

    #[test]
    fn coarse_levels_warn_at_low_whatever_the_threshold() {
        let coarse = |band: Coarse| Reading {
            precision: Precision::Coarse,
            ..reading("x", "Xbox", true, Battery::Level(band.level()))
        };
        let mut devices = Vec::new();
        let mut fired = Vec::new();
        for band in [Coarse::Full, Coarse::Medium, Coarse::Low, Coarse::VeryLow, Coarse::Full, Coarse::Low] {
            fired.push(apply(&mut devices, vec![coarse(band)], 1, 10, None, true).len());
        }
        assert_eq!(fired, [0, 0, 1, 0, 0, 1]);
        for band in [Coarse::VeryLow, Coarse::Low, Coarse::Medium, Coarse::Full] {
            assert_eq!(Coarse::from_level(band.level()), band);
        }
        let t = Lang::En.strings();
        assert_eq!(level_text(60, Precision::Coarse, t), "Medium");
        assert_eq!(fill_level(t.low_body, 25, Precision::Coarse, t, &[("name", "Pad")]), "Pad is at Low");
        assert_eq!(fill_level(t.low_body, 25, Precision::Percent, t, &[("name", "Pad")]), "Pad is at 25%");
        for lang in Lang::ALL {
            let t = lang.strings();
            for text in [t.low_body, t.off_last] {
                let filled = fill_level(text, 25, Precision::Coarse, t, &[("name", "Pad"), ("when", "now")]);
                assert!(!filled.contains('%') && !filled.contains('{'), "{lang:?}: {filled}");
            }
        }
        assert_eq!(level_text(60, Precision::Tens, t), "60%");
    }

    #[test]
    fn parts_show_in_the_summary_and_are_remembered_when_off() {
        let t = Lang::En.strings();
        let mut devices = Vec::new();
        let live = Reading {
            precision: Precision::Tens,
            parts: buds(Some(80), Some(70), Some(40), false),
            ..reading("a", "AirPods Pro", true, Battery::Level(70))
        };
        apply(&mut devices, vec![live], 1, 20, None, false);
        assert_eq!(summary_line(&devices[0], t), "AirPods Pro: L 80% · R 70% · Case 40%");
        // In the case and charging, case not reporting.
        let charging = Reading {
            precision: Precision::Tens,
            charging: Some(true),
            parts: buds(Some(90), Some(90), None, true),
            ..reading("a", "AirPods Pro", true, Battery::Level(90))
        };
        apply(&mut devices, vec![charging], 2, 20, None, false);
        assert_eq!(summary_line(&devices[0], t), "AirPods Pro: L 90%+ · R 90%+ · Case – (charging)");
        // Off: the last parts stay for the card, the summary says off.
        apply(&mut devices, vec![reading("a", "AirPods Pro", false, Battery::Unreadable)], 3, 20, None, false);
        assert_eq!(devices[0].parts.len(), 3);
        assert_eq!(devices[0].precision, Precision::Tens);
        assert_eq!(summary_line(&devices[0], t), "AirPods Pro: off");
    }

    #[test]
    fn a_reading_without_a_name_keeps_the_remembered_one() {
        let mut devices = Vec::new();
        apply(&mut devices, vec![reading("l", "MX Master 3", true, Battery::Level(50))], 1, 20, None, false);
        apply(&mut devices, vec![reading("l", "", false, Battery::Unreadable)], 2, 20, None, false);
        assert_eq!(devices[0].name, "MX Master 3");
        assert_eq!(devices[0].state(), State::Off(Some((50, 1))));
    }

    #[test]
    fn a_v0_1_4_file_still_loads() {
        let old = r#"[
          {"id": "aabbccddeeff", "name": "MX Keys", "kind": "keyboard", "le": true,
           "last_level": 40, "last_seen": 1700000000, "armed": false},
          {"id": "112233445566", "name": "WH-1000XM4", "kind": "headphones", "le": false,
           "last_level": null, "last_seen": 0, "armed": true}
        ]"#;
        let devices = parse_saved(old);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].transport, Transport::Le);
        assert_eq!(devices[0].precision, Precision::Percent);
        assert_eq!(devices[0].state(), State::Off(Some((40, 1_700_000_000))));
        assert!(!devices[0].armed);
        assert_eq!(devices[1].transport, Transport::Bluetooth);
        assert!(devices[1].parts.is_empty());
        // And what this version writes does not carry the old field.
        let json = serde_json::to_string(&devices).unwrap();
        assert!(!json.contains("\"le\":"));
        assert_eq!(parse_saved(&json), devices);
        assert!(parse_saved("not json").is_empty());
    }

    #[test]
    fn finds_vendor_and_product_in_instance_ids() {
        assert_eq!(
            hardware_in_instance_id(r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0001004C_PID&2014\7&2D3E&0&AABBCCDDEEFF_C00000000"),
            Some((0x004c, 0x2014))
        );
        assert_eq!(
            hardware_in_instance_id(r"BTHLE\{00001812-0000-1000-8000-00805F9B34FB}_DEV_VID&02046D_PID&B023_REV&0010_E1A2B3C4D5E6\9&2F&0&0020"),
            Some((0x046d, 0xb023))
        );
        assert_eq!(hardware_in_instance_id(r"BTHENUM\DEV_AABBCCDDEEFF\7&2D3E4F5A&0&BLUETOOTHDEVICE_AABBCCDDEEFF"), None);
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
            assert_eq!(warnings, [Warning { name: "Beta".into(), level: 10, precision: Precision::Percent }]);
            // Alpha was not warned about, so it is still armed: unlocking
            // (or choosing it) warns straight away.
            let warnings = apply(&mut devices, low(), 2, 20, Some("b"), true);
            assert_eq!(warnings, [Warning { name: "Alpha".into(), level: 10, precision: Precision::Percent }]);
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
        let t = Lang::En.strings();
        assert_eq!(tooltip(&devices, t), "Buds: 82%\nPad: –\nMouse: off");
        assert_eq!(tooltip(&[], t), t.tray_none);

        let many: Vec<Reading> = (0..20)
            .map(|i| reading(&i.to_string(), &format!("Наушники {i:02}"), true, Battery::Level(50)))
            .collect();
        apply(&mut devices, many, 1, 20, None, false);
        let text = tooltip(&devices, t);
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
