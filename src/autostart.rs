//! "Start with Windows": a value under the current user's Run key, which
//! needs no admin rights. The app is started with `--tray` so it opens in
//! the tray, without its window.

use std::path::Path;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, w};

/// Starts the app hidden in the tray.
pub const TRAY_ARG: &str = "--tray";

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: windows::core::PCWSTR = w!("BatteryTray");

/// The command line Windows runs at sign-in.
pub fn command(exe: &Path) -> String {
    format!("\"{}\" {TRAY_ARG}", exe.display())
}

pub fn enabled() -> bool {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        ) == ERROR_SUCCESS
    }
}

pub fn set(on: bool) {
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let command = HSTRING::from(command(&exe));
        // Size in bytes, including the terminating NUL.
        let bytes = (command.len() as u32 + 1) * 2;
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            bytes,
        );
    }
}

/// Whether to switch "Start with Windows" on without being asked: once,
/// on an installed copy (not a build run from a source tree). After that
/// the tick box decides, so switching it off stays off.
pub fn default_on_now(offered: bool, installed: bool) -> bool {
    !offered && installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_once_and_only_when_installed() {
        assert!(default_on_now(false, true));
        assert!(!default_on_now(true, true));
        assert!(!default_on_now(false, false));
    }

    #[test]
    fn command_quotes_the_path_and_starts_in_the_tray() {
        let exe = Path::new(r"C:\Users\Some One\AppData\Local\BatteryTray\current\battery_tray.exe");
        assert_eq!(
            command(exe),
            r#""C:\Users\Some One\AppData\Local\BatteryTray\current\battery_tray.exe" --tray"#
        );
    }
}
