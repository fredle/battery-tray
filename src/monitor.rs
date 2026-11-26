//! The background thread that asks the source for battery levels, and the
//! state it shares with the tray (and, in the window process, the state
//! the tray process sends). The window never waits for Bluetooth: it only
//! reads what was last found.
//!
//! Polls come about once a minute while a device is connected and someone
//! is at the screen; every five minutes while the session is locked or the
//! screen is off, or nothing but the PC's own battery is connected; twice
//! as far apart under battery or energy saver. A device arriving, the PC
//! waking up, the power status changing or the window opening bring the
//! next poll forward (`Schedule`). Between polls the thread sleeps without
//! a timer; it runs at background priority.

use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::devices::{self, Device, Notice, Rules};
use crate::flyout::WindowState;
use crate::history::{self, History};
use crate::i18n::Lang;
use crate::source::Source;

/// Seconds between polls. `BT_POLL_SECS` overrides it, for trying things out.
const POLL_SECS: u64 = 60;

pub struct Shared {
    pub devices: Vec<Device>,
    /// False until the first poll has finished.
    pub polled: bool,
    /// False until a poll has read the devices. What the first such poll
    /// finds connected is not news.
    pub applied: bool,
    /// Warning levels, tray choice, unlock and notification switches.
    pub rules: Rules,
    pub lang: Lang,
    /// Notifications waiting for the tray to show them.
    pub pending: Vec<Notice>,
    pub history: History,
    /// Counts the writes of history.json, so the window process knows when
    /// to read it again.
    pub history_rev: u64,
    /// Whether the window is hidden, a flyout or pinned (window process).
    pub window: WindowState,
}

impl Shared {
    pub fn new(devices: Vec<Device>, history: History, rules: Rules, lang: Lang) -> Shared {
        Shared {
            devices,
            polled: false,
            applied: false,
            rules,
            lang,
            pending: Vec::new(),
            history,
            history_rev: 0,
            window: WindowState::default(),
        }
    }
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
        let now = now();
        let notices = devices::apply(&mut state.devices, readings, now, &state.rules, !state.applied);
        state.applied = true;
        state.pending.extend(notices);
        if devices::needs_saving(&before, &state.devices) {
            devices::save(&state.devices);
        }
        if record(&mut state.history, &state.devices, now) {
            history::save(&state.history);
            state.history_rev += 1;
        }
    }
    state.polled = true;
}

/// Adds what changed to the history and drops what is too old or no
/// longer paired. Returns whether the file needs writing.
pub fn record(history: &mut History, devices: &[Device], now: u64) -> bool {
    let mut changed = false;
    for device in devices {
        if let Some(level) = device.live_level() {
            changed |= history.record(&device.id, now, level, device.is_charging());
        }
    }
    changed | history.compact(now, |id| devices.iter().any(|d| d.id == id))
}

/// What Windows says about the PC that changes how often to poll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions {
    /// Battery saver (Windows 10) or energy saver (Windows 11) is on.
    pub saver: bool,
    /// The session is locked or its screen is off: nobody is looking.
    pub away: bool,
}

/// While nobody is looking, or nothing but the PC's own battery is
/// connected, polls come this far apart at least. A device that connects
/// meanwhile is noticed at once through `Wake::DevicesChanged`.
pub const SLOW: Duration = Duration::from_secs(5 * 60);
/// Windows announces a device before it is ready to say its level.
pub const SETTLE: Duration = Duration::from_secs(3);
/// After the PC wakes up, Bluetooth devices take a moment to reconnect.
pub const AFTER_RESUME: Duration = Duration::from_secs(8);

/// How long after a poll the next one is due. `connected`: some device
/// other than the PC's own battery is connected.
pub fn poll_gap(base: Duration, conditions: Conditions, connected: bool) -> Duration {
    let mut gap = base;
    if conditions.away || !connected {
        gap = gap.max(SLOW);
    }
    if conditions.saver {
        gap *= 2;
    }
    gap
}

/// Why the poll thread is woken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// Poll now: Refresh, a change to what gets a warning, the window opened.
    Now,
    /// Windows' device list changed: something was plugged in, paired,
    /// connected or removed.
    DevicesChanged,
    /// The power status changed (plugged in, battery level).
    Power,
    /// Back from sleep.
    Resumed,
    /// Someone opened the window: what it shows should not be older than
    /// a regular gap.
    Looked,
    Conditions(Conditions),
}

/// When the next poll is due.
#[derive(Clone, Copy, Debug)]
pub struct Schedule {
    pub base: Duration,
    pub conditions: Conditions,
    /// Whether the last poll found a device (other than the PC) connected.
    pub connected: bool,
    pub last: Option<Instant>,
    /// A poll asked for sooner than the regular one.
    pub soon: Option<Instant>,
}

impl Schedule {
    pub fn new(base: Duration) -> Schedule {
        Schedule { base, conditions: Conditions::default(), connected: true, last: None, soon: None }
    }

    pub fn due(&self) -> Instant {
        let regular = match self.last {
            Some(last) => last + poll_gap(self.base, self.conditions, self.connected),
            None => return Instant::now(),
        };
        self.soon.map_or(regular, |soon| soon.min(regular))
    }

    pub fn polled(&mut self, at: Instant, connected: bool) {
        self.last = Some(at);
        self.connected = connected;
        self.soon = None;
    }

    fn sooner(&mut self, at: Instant) {
        self.soon = Some(self.soon.map_or(at, |soon| soon.min(at)));
    }

    /// Takes a wake into account. Returns true when the device list may
    /// have changed, so the sources should look again.
    pub fn woken(&mut self, wake: Wake, now: Instant) -> bool {
        match wake {
            Wake::Now => self.sooner(now),
            Wake::DevicesChanged => {
                self.sooner(now + SETTLE);
                return true;
            }
            Wake::Power => self.sooner(now + SETTLE),
            Wake::Looked => {
                if let Some(last) = self.last {
                    self.sooner((last + self.base).max(now));
                }
            }
            Wake::Resumed => {
                self.sooner(now + AFTER_RESUME);
                return true;
            }
            Wake::Conditions(conditions) => {
                // Back at the screen: the regular gap counts from the last
                // poll, so a stale level is read again at once.
                self.conditions = conditions;
            }
        }
        false
    }
}

/// Whether any device but the PC's own battery is connected.
fn any_connected(shared: &SharedState) -> bool {
    lock(shared).devices.iter().any(|d| d.connected && d.transport != devices::Transport::Internal)
}

pub struct Monitor {
    wake: Sender<Wake>,
}

impl Monitor {
    /// Starts polling. `changed` runs after every poll, on the poll thread.
    pub fn start(
        mut source: Box<dyn Source>,
        shared: SharedState,
        changed: impl Fn() + Send + 'static,
    ) -> Monitor {
        let (wake, woken) = channel();
        let mut schedule = Schedule::new(interval());
        std::thread::spawn(move || {
            crate::power::background_thread();
            loop {
                let now = Instant::now();
                let due = schedule.due();
                if due <= now {
                    poll(source.as_mut(), &shared);
                    schedule.polled(Instant::now(), any_connected(&shared));
                    changed();
                    continue;
                }
                match woken.recv_timeout(due - now) {
                    Ok(wake) => {
                        if schedule.woken(wake, Instant::now()) {
                            source.devices_changed();
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Monitor { wake }
    }

    /// Polls again now: after Refresh, or a change to what gets a warning.
    pub fn refresh(&self) {
        let _ = self.wake.send(Wake::Now);
    }

    pub fn wake(&self, wake: Wake) {
        let _ = self.wake.send(wake);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{Battery, Kind, Reading, State, Transport};

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn polls_back_off_when_nobody_looks_or_nothing_is_connected() {
        let normal = Conditions::default();
        assert_eq!(poll_gap(MINUTE, normal, true), MINUTE);
        assert_eq!(poll_gap(MINUTE, normal, false), SLOW);
        assert_eq!(poll_gap(MINUTE, Conditions { away: true, saver: false }, true), SLOW);
        assert_eq!(poll_gap(MINUTE, Conditions { away: false, saver: true }, true), 2 * MINUTE);
        assert_eq!(poll_gap(MINUTE, Conditions { away: true, saver: true }, false), 2 * SLOW);
        // A short test interval stays short while in use.
        assert_eq!(poll_gap(Duration::from_secs(2), normal, true), Duration::from_secs(2));
    }

    #[test]
    fn the_first_poll_is_at_once_and_wakes_bring_the_next_forward() {
        let start = Instant::now();
        let mut s = Schedule::new(MINUTE);
        assert!(s.due() <= Instant::now());
        s.polled(start, true);
        assert_eq!(s.due(), start + MINUTE);

        // A device arrives: a poll once Windows has settled, and the sources
        // look for new devices.
        assert!(s.woken(Wake::DevicesChanged, start + Duration::from_secs(10)));
        assert_eq!(s.due(), start + Duration::from_secs(10) + SETTLE);
        // A later request does not push it back; an earlier one wins.
        assert!(!s.woken(Wake::Power, start + Duration::from_secs(20)));
        assert_eq!(s.due(), start + Duration::from_secs(10) + SETTLE);
        assert!(!s.woken(Wake::Now, start + Duration::from_secs(11)));
        assert_eq!(s.due(), start + Duration::from_secs(11));
        s.polled(start + Duration::from_secs(11), true);
        assert_eq!(s.due(), start + Duration::from_secs(11) + MINUTE);

        // Locked for a while: slow polls; unlocked, the next is due a
        // regular gap after the last, which may already have passed.
        s.woken(Wake::Conditions(Conditions { away: true, saver: false }), start + Duration::from_secs(12));
        assert_eq!(s.due(), start + Duration::from_secs(11) + SLOW);
        s.woken(Wake::Conditions(Conditions::default()), start + Duration::from_secs(200));
        assert_eq!(s.due(), start + Duration::from_secs(11) + MINUTE);

        // Back from sleep: a poll once Bluetooth has reconnected.
        assert!(s.woken(Wake::Resumed, start + Duration::from_secs(12)));
        assert_eq!(s.due(), start + Duration::from_secs(12) + AFTER_RESUME);

        // Nothing connected: slow, until a device arrives.
        s.polled(start, false);
        assert_eq!(s.due(), start + SLOW);
        // Or someone looks: then nothing shown is older than a regular gap.
        s.woken(Wake::Looked, start + Duration::from_secs(5));
        assert_eq!(s.due(), start + MINUTE);
        s.woken(Wake::Looked, start + Duration::from_secs(100));
        assert_eq!(s.due(), start + MINUTE);
        s.polled(start, false);
        s.woken(Wake::Looked, start + Duration::from_secs(100));
        assert_eq!(s.due(), start + Duration::from_secs(100));
    }

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

        let shared: SharedState = Arc::new(Mutex::new(Shared::new(Vec::new(), History::default(), Rules::default(), Lang::En)));
        let mut source = Script(vec![None, mouse(50), None, mouse(18), mouse(17), mouse(60), mouse(9)]);

        // A failed first read: polled, but nothing applied yet.
        poll(&mut source, &shared);
        assert!(lock(&shared).polled && !lock(&shared).applied);

        poll(&mut source, &shared);
        assert!(lock(&shared).applied);
        // Connected at the first read: no "connected" pop-up.
        assert!(lock(&shared).pending.is_empty());

        // A failed read changes nothing.
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).devices[0].state(), State::Live(50));

        poll(&mut source, &shared);
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).pending.len(), 1);
        assert_eq!(lock(&shared).pending[0].level, Some(18));

        poll(&mut source, &shared);
        poll(&mut source, &shared);
        assert_eq!(lock(&shared).pending.len(), 2);

        // What was seen is on disk for the next start.
        let saved = devices::load();
        assert_eq!(saved[0].last_level, Some(9));
        assert!(!saved[0].armed);
        // And the history of every change, for the graph.
        let levels: Vec<u8> = history::load().samples("m").iter().map(|s| s.level()).collect();
        assert_eq!(levels, [50, 18, 17, 60, 9]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
