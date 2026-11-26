//! Where device readings come from.
//!
//! The real source is a composite of everything Windows can tell us:
//!
//! - `bluetooth.rs`: paired Bluetooth devices (the list every other source
//!   is merged into);
//! - `airpods.rs`: earbud and case levels of the user's AirPods and Beats,
//!   from Apple's Bluetooth LE advertisements;
//! - `xinput.rs`: Xbox controllers;
//! - `hid.rs`: Logitech devices over HID++ (`hidpp.rs`) and PlayStation
//!   controllers (`sony.rs`);
//! - `power.rs`: the PC's own battery.
//!
//! Readings for the same device share an id (a Bluetooth device's address)
//! and are merged into one. A source that fails on a poll is replaced by
//! what it said last time, so one failure never blanks the others.
//!
//! The simulated source reads a list from JSON instead, so the window, the
//! tray and the warnings can be exercised on a PC without the hardware.
//! `BT_FAKE_DEVICES` holds either the JSON itself or the path of a JSON
//! file. A file is read again on every poll, so editing it changes what the
//! app sees:
//!
//! ```json
//! [
//!   {"name": "WH-1000XM4", "kind": "headphones", "level": 82},
//!   {"name": "MX Master 3", "kind": "mouse", "level": 15, "transport": "receiver", "charging": true},
//!   {"name": "Old keyboard", "kind": "keyboard", "le": true},
//!   {"name": "Pixel Buds", "kind": "headphones", "connected": false},
//!   {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
//!     {"kind": "left", "level": 80}, {"kind": "right", "level": 70, "charging": true},
//!     {"kind": "case", "level": 40}]},
//!   {"name": "Xbox Wireless Controller", "kind": "gamepad", "precision": "coarse", "level": 25},
//!   {"name": "DualSense", "kind": "gamepad", "precision": "tens", "level": 60, "transport": "usb", "charging": true},
//!   {"name": "This PC", "kind": "laptop", "transport": "internal", "level": 55, "charging": false}
//! ]
//! ```
//!
//! - `kind`: headphones, mouse, keyboard, gamepad, laptop or other.
//! - `transport`: bluetooth (default), le, receiver, usb or internal;
//!   `"le": true` is the same as `"transport": "le"`.
//! - `precision`: percent (default), tens (reports in steps of ten) or
//!   coarse (only a band: a level up to 5 is "very low", up to 25 "low", up
//!   to 60 "medium", above that "full").
//! - `charging`: true or false; leave it out for "not known".
//! - `parts`: earbuds and case, each with `kind` left, right or case, an
//!   optional `level` (missing = that part is not reporting) and
//!   `charging`. Without a `level` of its own, the device's level is its
//!   lowest earbud.
//! - A connected device with no `level` (and no parts with one) is one that
//!   does not report its battery; `"unreadable": true` simulates a failed
//!   read; `"connected": false` one that is off. `id` defaults to the name.

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::devices::{self, Battery, Coarse, Kind, Part, Precision, Reading, Transport};

pub const FAKE_ENV: &str = "BT_FAKE_DEVICES";

pub trait Source: Send {
    /// Every device, or `None` if the list could not be read this time
    /// (what is already known is then kept as it is).
    fn read(&mut self) -> Option<Vec<Reading>>;

    /// Windows' device list changed: look for devices again on the next
    /// read instead of trusting what was found before.
    fn devices_changed(&mut self) {}
}

/// The simulated source if `BT_FAKE_DEVICES` is set, otherwise Windows.
pub fn pick() -> Box<dyn Source> {
    match std::env::var(FAKE_ENV) {
        Ok(value) if !value.trim().is_empty() => Box::new(Fake { value }),
        _ => Box::new(Composite::default()),
    }
}

/// How long AirPods advertisements are listened for on each poll, and only
/// while an Apple audio device is connected. They advertise several times
/// a second while out of the case, so a few seconds is plenty; the rest of
/// the minute the radio is not scanning.
const SCAN_WINDOW: Duration = Duration::from_secs(3);

/// After this many failed reads in a row with nothing to fall back on, the
/// Bluetooth list is treated as empty so the other sources still show (a
/// PC whose Bluetooth is off or missing).
const BLUETOOTH_GIVE_UP: u32 = 3;

/// Sources that only find devices that are plugged in or paired (HID,
/// XInput) look again when Windows says the device list changed, and
/// otherwise this often in case a change went unannounced.
pub const LOOK_AGAIN: Duration = Duration::from_secs(10 * 60);

/// Whether a source that found nothing last time needs asking now.
pub fn look_again(found_before: bool, changed: bool, last_look: Option<Instant>, now: Instant) -> bool {
    found_before || changed || last_look.is_none_or(|at| now.duration_since(at) >= LOOK_AGAIN)
}

/// Every source on this PC, merged.
#[derive(Default)]
pub struct Composite {
    bluetooth: Option<Vec<Reading>>,
    bluetooth_failures: u32,
    hid: crate::hid::Hid,
    hid_last: Option<Vec<Reading>>,
    airpods: crate::airpods::Airpods,
    /// The device list changed since the last read.
    changed: bool,
    /// XInput: whether it found a controller last time, and when it last looked.
    pads_found: bool,
    pads_looked: Option<Instant>,
}

impl Source for Composite {
    fn devices_changed(&mut self) {
        self.changed = true;
        self.hid.devices_changed();
    }

    fn read(&mut self) -> Option<Vec<Reading>> {
        let changed = std::mem::take(&mut self.changed);
        let bluetooth = crate::bluetooth::Windows.read();
        if bluetooth.is_some() {
            self.bluetooth_failures = 0;
        } else {
            self.bluetooth_failures += 1;
        }
        let bluetooth = keep_last(&mut self.bluetooth, bluetooth);
        // Listen while the other sources are asked, if there is anything to
        // listen for.
        let started = Instant::now();
        let scan = bluetooth
            .as_deref()
            .is_some_and(crate::airpods::wanted)
            .then(crate::airpods::Scan::start)
            .flatten();
        let hid = keep_last(&mut self.hid_last, self.hid.read());
        let pads = if look_again(self.pads_found, changed, self.pads_looked, started) {
            self.pads_looked = Some(started);
            let pads = crate::xinput::read();
            self.pads_found = !pads.is_empty();
            pads
        } else {
            Vec::new()
        };
        let power = crate::power::read();

        let adverts = match scan {
            Some(scan) => {
                let elapsed = started.elapsed();
                if elapsed < SCAN_WINDOW {
                    std::thread::sleep(SCAN_WINDOW - elapsed);
                }
                scan.finish()
            }
            None => Vec::new(),
        };

        let mut readings = match bluetooth {
            Some(readings) => readings,
            None if self.bluetooth_failures >= BLUETOOTH_GIVE_UP => Vec::new(),
            // Nothing to show yet that would not forget the Bluetooth devices.
            None => return None,
        };
        self.airpods.attach(&mut readings, &adverts, Instant::now());
        crate::xinput::attach(&mut readings, pads);
        for reading in hid.unwrap_or_default() {
            merge(&mut readings, reading);
        }
        if let Some(pc) = power {
            merge(&mut readings, pc);
        }
        Some(readings)
    }
}

/// What a source said this time, or what it said last time if it failed.
pub fn keep_last(last: &mut Option<Vec<Reading>>, new: Option<Vec<Reading>>) -> Option<Vec<Reading>> {
    if let Some(new) = new {
        *last = Some(new);
    }
    last.clone()
}

/// Adds one reading to the list, or folds it into the reading already there
/// for the same device. The one already there (from the Bluetooth list)
/// keeps its name, kind and transport, which come from Windows; the battery
/// comes from whichever of the two actually has a level.
pub fn merge(readings: &mut Vec<Reading>, extra: Reading) {
    let Some(base) = readings.iter_mut().find(|r| r.id == extra.id) else {
        readings.push(extra);
        return;
    };
    if base.name.is_empty() {
        base.name = extra.name.clone();
    }
    if base.kind == Kind::Other {
        base.kind = extra.kind;
    }
    base.connected |= extra.connected;
    base.hardware = base.hardware.or(extra.hardware);
    let base_has_level = matches!(base.battery, Battery::Level(_));
    let extra_has_level = matches!(extra.battery, Battery::Level(_));
    if extra_has_level || (!base_has_level && extra.battery == Battery::Unreadable) {
        base.battery = extra.battery;
        base.precision = extra.precision;
        base.charging = extra.charging;
        base.parts = extra.parts;
    }
}

struct Fake {
    value: String,
}

impl Source for Fake {
    fn read(&mut self) -> Option<Vec<Reading>> {
        let value = self.value.trim();
        if value.starts_with('[') {
            parse(value)
        } else {
            parse(&std::fs::read_to_string(value).ok()?)
        }
    }
}

#[derive(Deserialize)]
struct FakeDevice {
    name: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    kind: Kind,
    #[serde(default)]
    le: bool,
    #[serde(default)]
    transport: Option<Transport>,
    #[serde(default)]
    precision: Precision,
    #[serde(default = "yes")]
    connected: bool,
    #[serde(default)]
    level: Option<u8>,
    #[serde(default)]
    charging: Option<bool>,
    #[serde(default)]
    parts: Vec<Part>,
    #[serde(default)]
    unreadable: bool,
}

fn yes() -> bool {
    true
}

pub fn parse(json: &str) -> Option<Vec<Reading>> {
    // Editors like to save UTF-8 with a byte-order mark.
    let devices: Vec<FakeDevice> = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    Some(
        devices
            .into_iter()
            .map(|d| {
                let normal = |level: u8| match d.precision {
                    Precision::Coarse => Coarse::from_level(level).level(),
                    _ => level.min(100),
                };
                let parts: Vec<Part> = d
                    .parts
                    .iter()
                    .map(|p| Part { level: p.level.map(normal), ..*p })
                    .collect();
                let level = d.level.or_else(|| devices::headline(&parts));
                let transport = d.transport.unwrap_or(if d.le { Transport::Le } else { Transport::Bluetooth });
                Reading {
                    id: d.id.unwrap_or_else(|| format!("fake-{}", d.name.to_lowercase())),
                    battery: match level {
                        _ if d.unreadable => Battery::Unreadable,
                        Some(level) => Battery::Level(normal(level)),
                        None => Battery::NotReported,
                    },
                    connected: d.connected,
                    precision: d.precision,
                    charging: d.charging.or_else(|| devices::headline_charging(&parts)),
                    parts,
                    ..Reading::new("", d.name, d.kind, transport)
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::PartKind;

    #[test]
    fn parses_simulated_devices() {
        let readings = parse(
            r#"[
                {"name": "WH-1000XM4", "kind": "headphones", "level": 82},
                {"name": "MX Master 3", "kind": "mouse", "level": 150, "le": true, "id": "m"},
                {"name": "Old keyboard", "kind": "keyboard"},
                {"name": "Flaky", "level": 40, "unreadable": true},
                {"name": "Pixel Buds", "connected": false}
            ]"#,
        )
        .unwrap();
        assert_eq!(readings.len(), 5);
        assert_eq!(readings[0].id, "fake-wh-1000xm4");
        assert_eq!(readings[0].kind, Kind::Headphones);
        assert_eq!(readings[0].battery, Battery::Level(82));
        assert!(readings[0].connected);
        assert_eq!(readings[0].transport, Transport::Bluetooth);
        assert_eq!(readings[1].id, "m");
        assert_eq!(readings[1].battery, Battery::Level(100));
        assert_eq!(readings[1].transport, Transport::Le);
        assert_eq!(readings[2].battery, Battery::NotReported);
        assert_eq!(readings[3].battery, Battery::Unreadable);
        assert_eq!(readings[4].kind, Kind::Other);
        assert!(!readings[4].connected);
        assert_eq!(readings[4].charging, None);
    }

    #[test]
    fn simulates_parts_charging_transport_precision_and_the_laptop() {
        let readings = parse(
            r#"[
                {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
                    {"kind": "left", "level": 80}, {"kind": "right", "level": 70, "charging": true},
                    {"kind": "case"}]},
                {"name": "Xbox", "kind": "gamepad", "precision": "coarse", "level": 20},
                {"name": "MX", "transport": "receiver", "level": 50, "charging": false},
                {"name": "This PC", "kind": "laptop", "transport": "internal", "level": 55, "charging": true}
            ]"#,
        )
        .unwrap();
        let pods = &readings[0];
        assert_eq!(pods.battery, Battery::Level(70));
        assert_eq!(pods.precision, Precision::Tens);
        assert_eq!(pods.charging, Some(true));
        assert_eq!(pods.parts.len(), 3);
        assert_eq!(pods.parts[2], Part { kind: PartKind::Case, level: None, charging: false });
        assert_eq!(readings[1].battery, Battery::Level(Coarse::Low.level()));
        assert_eq!(readings[2].transport, Transport::Receiver);
        assert_eq!(readings[2].charging, Some(false));
        assert_eq!(readings[3].kind, Kind::Laptop);
        assert_eq!(readings[3].transport, Transport::Internal);
    }

    #[test]
    fn bad_json_is_a_failed_read_not_an_empty_list() {
        assert!(parse("[{").is_none());
        assert!(parse("{}").is_none());
        assert_eq!(parse("\u{feff}[]"), Some(Vec::new()));
    }

    #[test]
    fn a_file_is_read_again_on_every_poll() {
        let path = std::env::temp_dir().join(format!("battery-tray-fake-{}.json", std::process::id()));
        let mut source = Fake {
            value: path.display().to_string(),
        };
        assert!(source.read().is_none());
        std::fs::write(&path, r#"[{"name": "A", "level": 50}]"#).unwrap();
        assert_eq!(source.read().unwrap()[0].battery, Battery::Level(50));
        std::fs::write(&path, r#"[{"name": "A", "level": 10}]"#).unwrap();
        assert_eq!(source.read().unwrap()[0].battery, Battery::Level(10));
        let _ = std::fs::remove_file(&path);
    }

    fn bt(id: &str, name: &str, battery: Battery) -> Reading {
        Reading { battery, ..Reading::new(id, name, Kind::Other, Transport::Le) }
    }

    #[test]
    fn the_same_device_from_two_sources_is_shown_once() {
        let mut readings = vec![
            bt("aabbccddeeff", "MX Anywhere 3", Battery::NotReported),
            bt("112233445566", "Buds", Battery::Level(40)),
        ];
        // Logitech over HID++ on the same Bluetooth address: its level wins,
        // Windows' name and transport stay.
        merge(
            &mut readings,
            Reading {
                battery: Battery::Level(65),
                charging: Some(false),
                ..Reading::new("aabbccddeeff", "MX Anywhere 3S", Kind::Mouse, Transport::Usb)
            },
        );
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].name, "MX Anywhere 3");
        assert_eq!(readings[0].kind, Kind::Mouse);
        assert_eq!(readings[0].transport, Transport::Le);
        assert_eq!(readings[0].battery, Battery::Level(65));
        assert_eq!(readings[0].charging, Some(false));
        // An extra reading without a level never hides one that has it.
        merge(&mut readings, bt("112233445566", "Buds", Battery::Unreadable));
        assert_eq!(readings[1].battery, Battery::Level(40));
        // A new device is added.
        merge(&mut readings, Reading::new("logi-4082-1", "M720", Kind::Mouse, Transport::Receiver));
        assert_eq!(readings.len(), 3);
    }

    /// Every real source on this PC, once, with how long it took.
    /// `cargo test composite_for_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn composite_for_real() {
        let mut source = Composite::default();
        for _ in 0..2 {
            let started = Instant::now();
            let readings = source.read();
            println!("poll took {:?}", started.elapsed());
            for r in readings.unwrap_or_default() {
                println!("{r:?}");
            }
        }
        println!("xinput: {:?}", crate::xinput::read());
        println!("power: {:?}", crate::power::read());
    }

    #[test]
    fn a_source_that_found_nothing_waits_for_a_device_change() {
        let now = Instant::now();
        // Never looked: look.
        assert!(look_again(false, false, None, now));
        // Found something last time: keep reading it.
        assert!(look_again(true, false, Some(now), now));
        // Found nothing a minute ago, nothing changed: skip.
        assert!(!look_again(false, false, Some(now), now + Duration::from_secs(60)));
        // Windows says the list changed, or it has been a while: look.
        assert!(look_again(false, true, Some(now), now + Duration::from_secs(60)));
        assert!(look_again(false, false, Some(now), now + LOOK_AGAIN));
    }

    #[test]
    fn a_failed_source_keeps_what_it_said_last() {
        let mut last = None;
        assert_eq!(keep_last(&mut last, None), None);
        let one = vec![bt("a", "A", Battery::Level(1))];
        assert_eq!(keep_last(&mut last, Some(one.clone())), Some(one.clone()));
        assert_eq!(keep_last(&mut last, None), Some(one));
        assert_eq!(keep_last(&mut last, Some(Vec::new())), Some(Vec::new()));
    }
}
