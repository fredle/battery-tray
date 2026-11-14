//! Small persisted preferences in %LOCALAPPDATA%\BatteryTray\settings.json.
//! Add app-specific fields here, always with `#[serde(default)]` so older
//! settings files keep loading.

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
    #[serde(default)]
    pub tray_device: Option<String>,
    /// Warn when a device's battery is at or below this percent.
    #[serde(default = "default_threshold")]
    pub threshold: u8,
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
            threshold: default_threshold(),
            license_key: String::new(),
            language: None,
        }
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
