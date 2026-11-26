//! Small persisted preferences in %LOCALAPPDATA%\BatteryTray\settings.json.
//! Add app-specific fields here, always with `#[serde(default)]` so older
//! settings files keep loading.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// Check for updates in the background and install them silently.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Unix seconds of the last background update check, to throttle to once
    /// a day.
    #[serde(default)]
    pub last_update_check: u64,
    /// The device the tray icon shows; `None` picks one automatically.
    /// Still written (as the first of `tray_devices`) so v0.1.4 reads it.
    #[serde(default)]
    pub tray_device: Option<String>,
    /// The devices with a tray icon of their own, first one first. Empty
    /// in files from v0.1.4, which only had `tray_device`.
    #[serde(default)]
    pub tray_devices: Vec<String>,
    /// Warn when a device's battery is at or below this percent.
    #[serde(default = "default_threshold")]
    pub threshold: u8,
    /// Warning levels set for one device, overriding `threshold`.
    #[serde(default)]
    pub thresholds: BTreeMap<String, u8>,
    /// A notification when a device connects.
    #[serde(default = "default_true")]
    pub notify_connect: bool,
    /// A notification when a charging device is full.
    #[serde(default)]
    pub notify_full: bool,
    /// The hint about Windows hiding new tray icons was dismissed.
    #[serde(default)]
    pub pin_hint_done: bool,
    /// The licence key as pasted; verified on every start.
    #[serde(default)]
    pub license_key: String,
    /// Display language; `None` follows Windows.
    #[serde(default)]
    pub language: Option<crate::i18n::Lang>,
}

fn default_threshold() -> u8 {
    20
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            last_update_check: 0,
            tray_device: None,
            tray_devices: Vec::new(),
            threshold: default_threshold(),
            thresholds: BTreeMap::new(),
            notify_connect: true,
            notify_full: false,
            pin_hint_done: false,
            license_key: String::new(),
            language: None,
        }
    }
}

impl Settings {
    /// The devices chosen for the tray, from either generation of the file.
    pub fn tray_list(&self) -> Vec<String> {
        if self.tray_devices.is_empty() {
            self.tray_device.iter().cloned().collect()
        } else {
            self.tray_devices.clone()
        }
    }

    pub fn set_tray_list(&mut self, list: Vec<String>) {
        self.tray_device = list.first().cloned();
        self.tray_devices = list;
    }
}

pub fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into()))
        .join("BatteryTray")
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(settings_path(), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v0_1_4_file_loads_with_new_defaults_and_keeps_its_tray_choice() {
        let old = r#"{"auto_update": true, "last_update_check": 5, "tray_device": "aabbccddeeff",
                      "threshold": 15, "license_key": "", "language": "De"}"#;
        let mut s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.tray_list(), ["aabbccddeeff"]);
        assert_eq!(s.threshold, 15);
        assert!(s.thresholds.is_empty());
        assert!(s.notify_connect && !s.notify_full && !s.pin_hint_done);
        s.set_tray_list(vec!["b".into(), "c".into()]);
        let json = serde_json::to_string(&s).unwrap();
        // What v0.1.4 reads is still there.
        assert!(json.contains(r#""tray_device":"b""#));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tray_list(), ["b", "c"]);
        s.set_tray_list(Vec::new());
        assert!(s.tray_list().is_empty() && s.tray_device.is_none());
    }
}
