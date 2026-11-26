//! Battery Tray - Battery level of your Bluetooth headphones, mouse and keyboard in the tray, with low warnings.
//!
//!   battery_tray.exe                 open the app
//!   battery_tray.exe --auto-update   internal: background update check/download

#![windows_subsystem = "windows"]

mod airpods;
mod app;
mod feedback;
mod hid;
mod hidpp;
mod history;
mod autostart;
mod bluetooth;
mod devices;
mod flyout;
mod glyphs;
mod i18n;
mod icon;
mod license;
mod monitor;
mod power;
mod settings;
mod shots;
mod sony;
mod source;
mod theme;
mod tray;
mod update;
mod xinput;

fn main() {
    // First thing, always: Velopack may need to run an install/uninstall/
    // update hook and exit before any of our own argument handling runs.
    update::run_app_hooks();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // The detached child spawned below; must not spawn another.
    if args.first().map(String::as_str) == Some("--auto-update") {
        update::run_background_update();
        return;
    }
    update::maybe_spawn_background_update();

    if let Err(e) = app::run(args) {
        eprintln!("{e}");
    }
}
