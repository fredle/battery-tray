//! Where device readings come from. The real source asks Windows
//! (`bluetooth.rs`); the simulated one reads a list from JSON so the window,
//! the tray and the warnings can be exercised on a PC with no Bluetooth
//! devices.
//!
//! `BT_FAKE_DEVICES` holds either the JSON itself or the path of a JSON
//! file. A file is read again on every poll, so editing it changes what the
//! app sees:
//!
//! ```json
//! [
//!   {"name": "WH-1000XM4", "kind": "headphones", "level": 82},
//!   {"name": "MX Master 3", "kind": "mouse", "level": 15, "le": true},
//!   {"name": "Old keyboard", "kind": "keyboard"},
//!   {"name": "Pixel Buds", "kind": "headphones", "connected": false}
//! ]
//! ```
//!
//! `kind` is headphones, mouse, keyboard, gamepad or other. A connected
//! device with no `level` is one that does not report its battery;
//! `"unreadable": true` simulates a failed read. `id` defaults to the name.

use serde::Deserialize;

use crate::devices::{Battery, Kind, Reading};

pub const FAKE_ENV: &str = "BT_FAKE_DEVICES";

pub trait Source: Send {
    /// Every paired device, or `None` if the list could not be read this
    /// time (what is already known is then kept as it is).
    fn read(&mut self) -> Option<Vec<Reading>>;
}

/// The simulated source if `BT_FAKE_DEVICES` is set, otherwise Windows.
pub fn pick() -> Box<dyn Source> {
    match std::env::var(FAKE_ENV) {
        Ok(value) if !value.trim().is_empty() => Box::new(Fake { value }),
        _ => Box::new(crate::bluetooth::Windows),
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
    #[serde(default = "yes")]
    connected: bool,
    #[serde(default)]
    level: Option<u8>,
    #[serde(default)]
    unreadable: bool,
}

fn yes() -> bool {
    true
}

fn parse(json: &str) -> Option<Vec<Reading>> {
    // Editors like to save UTF-8 with a byte-order mark.
    let devices: Vec<FakeDevice> = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    Some(
        devices
            .into_iter()
            .map(|d| Reading {
                id: d.id.unwrap_or_else(|| format!("fake-{}", d.name.to_lowercase())),
                battery: match d.level {
                    _ if d.unreadable => Battery::Unreadable,
                    Some(level) => Battery::Level(level.min(100)),
                    None => Battery::NotReported,
                },
                name: d.name,
                kind: d.kind,
                le: d.le,
                connected: d.connected,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(readings[0].connected && !readings[0].le);
        assert_eq!(readings[1].id, "m");
        assert_eq!(readings[1].battery, Battery::Level(100));
        assert!(readings[1].le);
        assert_eq!(readings[2].battery, Battery::NotReported);
        assert_eq!(readings[3].battery, Battery::Unreadable);
        assert_eq!(readings[4].kind, Kind::Other);
        assert!(!readings[4].connected);
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
}
