//! The made-up scenes of screenshot mode:
//!
//!   battery_tray.exe --shots <folder> [--theme light|dark] [--scale 1.5] [--lang de]
//!
//! Each scene is drawn, captured by egui (`ViewportCommand::Screenshot`,
//! which reads back the app's own frame, so it works over Remote Desktop
//! too) and written as `<n>.png` and `<n>_<w>x<h>.rgba` by `shots.rs`.
//! Nothing real happens: no tray, no polling, no saving, nothing sent.
//! Only egui's frame is captured, so Windows' rounded corners and shadow
//! are not in the pictures.

use super::*;

use crate::devices::Battery;
use crate::history::History;

/// The clock the scenes are set at.
pub const NOW: u64 = 1_764_000_000;
pub const COUNT: usize = 7;

const H: u64 = 3600;

const DEVICES: &str = r#"[
  {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
    {"kind": "left", "level": 80}, {"kind": "right", "level": 70}, {"kind": "case", "level": 40, "charging": true}]},
  {"name": "MX Master 3", "kind": "mouse", "le": true, "level": 15, "charging": false},
  {"name": "Xbox Wireless Controller", "kind": "gamepad", "precision": "coarse", "level": 60, "transport": "receiver"},
  {"name": "DualSense", "kind": "gamepad", "precision": "tens", "level": 60, "transport": "usb", "charging": true},
  {"name": "This PC", "kind": "laptop", "transport": "internal", "level": 86, "charging": false},
  {"name": "WH-1000XM4", "kind": "headphones", "connected": false},
  {"name": "K380 Keyboard", "kind": "keyboard"}
]"#;

const MOUSE: &str = "fake-mx master 3";
const KEYBOARD: &str = "fake-k380 keyboard";
const LAPTOP: &str = "fake-this pc";

fn devices(rules: &Rules) -> Vec<Device> {
    let mut readings = crate::source::parse(DEVICES).unwrap_or_default();
    for reading in &mut readings {
        if reading.id == KEYBOARD {
            reading.hardware = Some((0x046d, 0xb342));
        }
    }
    // The headphones were on two hours ago, at 64%.
    let earlier: Vec<_> = readings
        .iter()
        .cloned()
        .map(|mut r| {
            if !r.connected {
                r.connected = true;
                r.battery = Battery::Level(64);
            }
            r
        })
        .collect();
    let mut devices = Vec::new();
    devices::apply(&mut devices, earlier, NOW - 2 * H - 600, rules, true);
    devices::apply(&mut devices, readings, NOW, rules, false);
    devices
}

/// A week of the mouse: charged to full, then a steady discharge to 15%;
/// and a laptop's day.
fn history() -> History {
    let mut history = History::default();
    let start = NOW - 7 * 86_400 + 5 * H;
    let mut t = start;
    for level in (34..=48).rev() {
        history.record(MOUSE, t, level, false);
        t += 100 * 60;
    }
    for level in (34..=100).step_by(6) {
        history.record(MOUSE, t, level, true);
        t += 9 * 60;
    }
    history.record(MOUSE, t, 100, false);
    let from = t;
    // 85 points to now.
    for (i, level) in (15..100).rev().enumerate() {
        history.record(MOUSE, from + (i as u64 + 1) * (NOW - from - 2 * H) / 85, level, false);
    }
    for (i, level) in (86..=100).rev().enumerate() {
        history.record(LAPTOP, NOW - 4 * H + i as u64 * 15 * 60, level, false);
    }
    history
}

pub fn set_up(app: &mut App, scene: usize, ctx: &egui::Context) {
    app.settings = crate::settings::Settings::default();
    app.licensed_to = None;
    app.page = Page::Devices;
    app.expanded.clear();
    app.unlock_open = false;
    app.shot_flyout = true;
    let licensed = Some("you@example.com".to_owned());
    match scene {
        // The flyout as a new user first sees it.
        0 => {}
        // Unlocked, a card opened: history and time left.
        1 => {
            app.licensed_to = licensed;
            app.settings.pin_hint_done = true;
            app.expanded.insert(MOUSE.into());
            app.settings.set_tray_list(vec![MOUSE.into(), "fake-airpods pro".into()]);
        }
        // Free, the same card: what the unlock adds.
        2 => {
            app.settings.pin_hint_done = true;
            app.expanded.insert(MOUSE.into());
        }
        // Settings, unlocked, two tray icons and a device's own level.
        3 => {
            app.licensed_to = licensed;
            app.page = Page::Settings;
            app.settings.set_tray_list(vec![MOUSE.into(), "fake-airpods pro".into()]);
            app.settings.thresholds.insert("fake-airpods pro".into(), 30);
            app.settings.notify_full = true;
        }
        // Settings, free.
        4 => app.page = Page::Settings,
        // Pinned.
        5 => {
            app.settings.pin_hint_done = true;
            app.shot_flyout = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(flyout::WIDTH, flyout::PINNED_HEIGHT)));
        }
        // "Tell us this model", filled in and waiting for the user.
        _ => {
            app.settings.pin_hint_done = true;
            app.shot_flyout = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(flyout::WIDTH, flyout::PINNED_HEIGHT)));
        }
    }
    let rules = app.rules();
    let devices = devices(&rules);
    if scene == 6
        && let Some(keyboard) = devices.iter().find(|d| d.id == KEYBOARD)
    {
        app.feedback.open_with(&model_report(keyboard, app.t));
    }
    let mut shared = lock(&app.shared);
    shared.devices = devices;
    shared.history = history();
    shared.rules = rules;
    shared.polled = true;
    shared.window.mode = if app.shot_flyout { Mode::Flyout } else { Mode::Pinned };
}
