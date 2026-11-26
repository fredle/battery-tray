//! The background thread that asks the source for battery levels about once
//! a minute, and the state it shares with the window and the tray. The
//! window never waits for Bluetooth: it only reads what was last found.

use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::devices::{self, Device, Warning};
use crate::i18n::Lang;
use crate::source::Source;

/// Seconds between polls. `BT_POLL_SECS` overrides it, for trying things out.
const POLL_SECS: u64 = 60;

pub struct Shared {
    pub devices: Vec<Device>,
    /// False until the first poll has finished.
    pub polled: bool,
    /// The user's choice of tray device, if any (see `devices::tray_device`).
    pub tray_device: Option<String>,
    pub threshold: u8,
    pub licensed: bool,
    pub lang: Lang,
    /// Warnings waiting for the tray to show them as notifications.
    pub pending: Vec<Warning>,
}

pub type SharedState = Arc<Mutex<Shared>>;

/// A poisoned lock only means another thread panicked mid-update; the data
/// is still the best there is.
pub fn lock(shared: &SharedState) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn interval() -> Duration {
    let seconds = std::env::var("BT_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(POLL_SECS);
    Duration::from_secs(seconds.max(1))
}

/// One poll: read, fold into the shared state, remember.
pub fn poll(source: &mut dyn Source, shared: &SharedState) {
    // The slow part, with the lock free.
    let readings = source.read();
    let mut state = lock(shared);
    if let Some(readings) = readings {
        let state = &mut *state;
        let before = state.devices.clone();
        let warnings = devices::apply(
            &mut state.devices,
            readings,
            now(),
            state.threshold,
            state.tray_device.as_deref(),
            state.licensed,
        );
        state.pending.extend(warnings);
        if state.devices != before {
            devices::save(&state.devices);
        }
    }
    state.polled = true;
}

pub struct Monitor {
    wake: Sender<()>,
}

impl Monitor {
    /// Starts polling. `changed` runs after every poll, on the poll thread.
    pub fn start(
        mut source: Box<dyn Source>,
        shared: SharedState,
        changed: impl Fn() + Send + 'static,
    ) -> Monitor {
        let (wake, woken) = channel();
        let interval = interval();
        std::thread::spawn(move || {
            loop {
                poll(source.as_mut(), &shared);
                changed();
                if woken.recv_timeout(interval) == Err(RecvTimeoutError::Disconnected) {
                    break;
                }
                // Several requests while polling need only one more poll.
                while woken.try_recv().is_ok() {}
            }
        });
        Monitor { wake }
    }

    /// Polls again now: after Refresh, or a change to what gets a warning.
    pub fn refresh(&self) {
        let _ = self.wake.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{Battery, Kind, Reading, State, Transport};

    struct Script(Vec<Option<Vec<Reading>>>);

    impl Source for Script {
        fn read(&mut self) -> Option<Vec<Reading>> {
            self.0.remove(0)
        }
    }

    fn mouse(level: u8) -> Option<Vec<Reading>> {
        Some(vec![Reading {
            battery: Battery::Level(level),
            ..Reading::new("m", "Mouse", Kind::Mouse, Transport::Le)
        }])
    }

    #[test]
    fn polls_queue_one_warning_per_discharge_and_keep_devices_on_a_failed_read() {
        // Keep this test's devices.json out of the real data folder.
        let dir = std::env::temp_dir().join(format!("battery-tray-test-{}", std::process::id()));
        unsafe { std::env::set_var("LOCALAPPDATA", &dir) };

        let shared: SharedState = Arc::new(Mutex::new(Shared {
            devices: Vec::new(),
            polled: false,
            tray_device: None,
            threshold: 20,
            licensed: false,
            lang: Lang::En,
            pending: Vec::new(),
        }));
        let mut source = Script(vec![mouse(50), None, mouse(18), mouse(17), mouse(60), mouse(9)]);

        poll(&mut source, &shared);
        assert!(lock(&shared).polled);
        assert!(lock(&shared).pending.is_empty());

        // A failed read changes nothing.
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).devices[0].state(), State::Live(50));

        poll(&mut source, &shared);
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).pending.len(), 1);
        assert_eq!(lock(&shared).pending[0].level, 18);

        poll(&mut source, &shared);
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).pending.len(), 2);

        // What was seen is on disk for the next start.
        let saved = devices::load();
        assert_eq!(saved[0].last_level, Some(9));
        assert!(!saved[0].armed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
