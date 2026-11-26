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
pub const COUNT: usize = 11;

const H: u64 = 3600;

const DEVICES: &str = r#"[
  {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
    {"kind": "left", "level": 80}, {"kind": "right", "level": 70}, {"kind": "case", "level": 40, "charging": true}]},
  {"name": "MX Master 3", "kind": "mouse", "le": true, "level": 15, "charging": false},
  {"name": "Xbox Wireless Controller", "kind": "gamepad", "precision": "coarse", "level": 60, "transport": "receiver"},
  {"name": "DualSense", "kind": "gamepad", "precision": "tens", "level": 60, "transport": "usb", "charging": true},
  {"name": "Surface Laptop", "kind": "laptop", "transport": "internal", "level": 86, "charging": false},
  {"name": "WH-1000XM4", "kind": "headphones", "connected": false},
  {"name": "K380 Keyboard", "kind": "keyboard"}
]"#;

const MOUSE: &str = "fake-mx master 3";
const KEYBOARD: &str = "fake-k380 keyboard";
const LAPTOP: &str = "fake-surface laptop";
const AIRPODS: &str = "fake-airpods pro";
const DUALSENSE: &str = "fake-dualsense";

/// The devices of the scenes that show the list as it should be: all with
/// a level, none unsupported or off. `MOUSE_LEVEL` is filled in.
const CLEAN: &str = r#"[
  {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
    {"kind": "left", "level": 80}, {"kind": "right", "level": 70}, {"kind": "case", "level": 40, "charging": true}]},
  {"name": "Xbox Wireless Controller", "kind": "gamepad", "precision": "coarse", "level": 60},
  {"name": "MX Master 3", "kind": "mouse", "transport": "receiver", "level": MOUSE_LEVEL, "charging": false},
  {"name": "MX Keys", "kind": "keyboard", "le": true, "level": 48, "charging": false},
  {"name": "DualSense", "kind": "gamepad", "precision": "tens", "level": 60, "transport": "usb", "charging": true},
  {"name": "Surface Laptop", "kind": "laptop", "transport": "internal", "level": 86, "charging": false}
]"#;

fn devices(rules: &Rules, json: &str, only: &[&str]) -> Vec<Device> {
    let mut readings = crate::source::parse(json).unwrap_or_default();
    if !only.is_empty() {
        readings.retain(|r| only.contains(&r.id.as_str()));
    }
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

/// A week of the mouse: charged to full, then a steady discharge to
/// `mouse_now`; and a laptop's day.
fn history(mouse_now: u8) -> History {
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
    // One point per percent to now.
    let steps = u64::from(100 - mouse_now);
    for (i, level) in (mouse_now..100).rev().enumerate() {
        history.record(MOUSE, from + (i as u64 + 1) * (NOW - from - 2 * H) / steps, level, false);
    }
    for (i, level) in (86..=100).rev().enumerate() {
        history.record(LAPTOP, NOW - 4 * H + i as u64 * 15 * 60, level, false);
    }
    history
}

/// What a scene shows and with which devices.
struct Setup {
    json: String,
    mouse_now: u8,
    /// The devices to keep; all when empty. The scenes that need room for
    /// a graph or the taskbar show four.
    only: &'static [&'static str],
}

const FOUR: &[&str] = &[AIRPODS, DUALSENSE, MOUSE, LAPTOP];

pub fn set_up(app: &mut App, scene: usize, ctx: &egui::Context) {
    app.settings = crate::settings::Settings::default();
    app.licensed_to = None;
    app.page = Page::Devices;
    app.expanded.clear();
    app.unlock_open = false;
    app.shot_flyout = true;
    app.stage = None;
    // A note left open by the last scene is closed.
    app.feedback = crate::feedback::Feedback::default();
    app.feedback.set_texts(app.lang.feedback_texts());
    // Every scene starts at the window's own size; a scene may change it.
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(flyout::WIDTH, flyout::PINNED_HEIGHT)));
    let licensed = Some("you@example.com".to_owned());
    let mut setup = Setup { json: DEVICES.to_owned(), mouse_now: 15, only: &[] };
    let clean = |mouse_now: u8, only| Setup { json: CLEAN.replace("MOUSE_LEVEL", &mouse_now.to_string()), mouse_now, only };
    match scene {
        // The flyout as a new user first sees it.
        0 => {}
        // Unlocked, a card opened: history and time left.
        1 => {
            app.licensed_to = licensed;
            app.settings.pin_hint_done = true;
            app.expanded.insert(MOUSE.into());
            app.settings.set_tray_list(vec![MOUSE.into(), AIRPODS.into()]);
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
            app.settings.set_tray_list(vec![MOUSE.into(), AIRPODS.into()]);
            app.settings.thresholds.insert(AIRPODS.into(), 30);
            app.settings.notify_full = true;
        }
        // Settings, free.
        4 => app.page = Page::Settings,
        // Pinned.
        5 => {
            app.settings.pin_hint_done = true;
            app.shot_flyout = false;
        }
        // "Tell us this model", filled in and waiting for the user.
        6 => {
            app.settings.pin_hint_done = true;
            app.shot_flyout = false;
            setup.only = &[AIRPODS, DUALSENSE, KEYBOARD, MOUSE];
        }
        // The list as it should look: every kind of device, each with a
        // level, nothing unsupported, nothing low.
        7 => {
            app.settings.pin_hint_done = true;
            setup = clean(72, &[]);
        }
        // The flyout where it opens, over the taskbar's corner: three
        // devices with a tray icon each, one of them low, one charging.
        8 => {
            app.licensed_to = licensed;
            app.settings.pin_hint_done = true;
            app.settings.set_tray_list(vec![AIRPODS.into(), MOUSE.into(), DUALSENSE.into()]);
            app.shot_flyout = false;
            setup = clean(15, FOUR);
        }
        // Unlocked: a device below its warning level, and its week.
        9 => {
            app.licensed_to = licensed;
            app.settings.pin_hint_done = true;
            app.expanded.insert(MOUSE.into());
            app.settings.set_tray_list(vec![MOUSE.into(), AIRPODS.into()]);
            setup = clean(15, FOUR);
        }
        // Unlocked: the week and the time left of a device that is fine.
        _ => {
            app.licensed_to = licensed;
            app.settings.pin_hint_done = true;
            app.expanded.insert(MOUSE.into());
            app.settings.set_tray_list(vec![MOUSE.into(), AIRPODS.into()]);
            setup = clean(41, FOUR);
        }
    }
    if matches!(scene, 5 | 6) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(flyout::WIDTH, flyout::PINNED_HEIGHT)));
    }
    let rules = app.rules();
    let devices = devices(&rules, &setup.json, setup.only);
    if scene == 8 {
        // Light or dark follows the theme asked for; the icons' own colours
        // follow the taskbar.
        let dark = ctx.theme() == egui::Theme::Dark;
        let icons = rules.tray_devices(&devices).into_iter().map(|d| crate::tray::look(d, &rules, !dark)).collect::<Vec<_>>();
        app.stage = icons.try_into().ok().map(|icons| taskbar::Stage { icons });
        app.stage_height = 480.0;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(taskbar::WIDTH, taskbar::canvas_height(app.stage_height))));
    }
    if scene == 6
        && let Some(keyboard) = devices.iter().find(|d| d.id == KEYBOARD)
    {
        app.feedback.open_with(&model_report(keyboard, app.t));
    }
    let mut shared = lock(&app.shared);
    shared.devices = devices;
    shared.history = history(setup.mouse_now);
    shared.rules = rules;
    shared.polled = true;
    shared.window.mode = if app.shot_flyout || app.stage.is_some() { Mode::Flyout } else { Mode::Pinned };
}
