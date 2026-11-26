//! The PC's own battery, from `GetSystemPowerStatus`. A PC without one (a
//! desktop, or a battery Windows cannot see) gets no card at all.

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::devices::{Battery, Kind, Reading, Transport};

pub const ID: &str = "this-pc";

/// `BatteryFlag` bits.
const FLAG_CHARGING: u8 = 8;
const FLAG_NO_BATTERY: u8 = 128;
const FLAG_UNKNOWN: u8 = 255;

pub fn read() -> Option<Reading> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    from_status(status.ACLineStatus, status.BatteryFlag, status.BatteryLifePercent, &computer_name())
}

/// The reading for a power status, or `None` when there is no battery.
pub fn from_status(ac_line: u8, flag: u8, percent: u8, name: &str) -> Option<Reading> {
    if flag == FLAG_UNKNOWN || flag & FLAG_NO_BATTERY != 0 {
        return None;
    }
    Some(Reading {
        battery: if percent <= 100 { Battery::Level(percent) } else { Battery::Unreadable },
        // Charging per the battery; on mains but not charging (full, or
        // held back by the PC's battery care) is not charging.
        charging: Some(flag & FLAG_CHARGING != 0 && ac_line != 0),
        ..Reading::new(ID, name, Kind::Laptop, Transport::Internal)
    })
}

/// Asks Windows to run this process in efficiency mode (EcoQoS: slower,
/// on efficient cores) or, with `false`, to never throttle it. The tray
/// process only waits and polls; the window process must feel instant.
pub fn efficiency_mode(on: bool) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if on { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
    };
    unsafe {
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// Puts the calling thread in background mode: low CPU, I/O and memory
/// priority, for the poll thread.
pub fn background_thread() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

/// This PC's name as the user sees it in Settings.
fn computer_name() -> String {
    use windows::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
    use windows::core::PWSTR;
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    match unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, Some(PWSTR(buffer.as_mut_ptr())), &mut size) } {
        Ok(()) => String::from_utf16_lossy(&buffer[..size as usize]),
        Err(_) => "PC".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pc_without_a_battery_has_no_card() {
        assert!(from_status(1, 128, 255, "Desk").is_none());
        assert!(from_status(255, 255, 255, "Desk").is_none());
    }

    #[test]
    fn a_laptop_battery_is_a_device() {
        // On battery, "high" flag.
        let r = from_status(0, 1, 76, "Laptop").unwrap();
        assert_eq!(r.battery, Battery::Level(76));
        assert_eq!(r.charging, Some(false));
        assert_eq!(r.kind, Kind::Laptop);
        assert_eq!(r.transport, Transport::Internal);
        assert_eq!(r.id, ID);
        // Plugged in and charging.
        assert_eq!(from_status(1, 8 | 2, 18, "Laptop").unwrap().charging, Some(true));
        // Plugged in, full.
        assert_eq!(from_status(1, 1, 100, "Laptop").unwrap().charging, Some(false));
        // Percentage unknown.
        assert_eq!(from_status(1, 0, 255, "Laptop").unwrap().battery, Battery::Unreadable);
    }
}
