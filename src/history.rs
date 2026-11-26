//! Each device's battery level over the last month, for the graph on an
//! expanded card and the "about 5 days left" estimate.
//!
//! A sample is kept only when the level or the charging state changes, so a
//! device that sits at 64% all week costs nothing, and the file
//! (`history.json` under `settings::data_dir()`) is written only when a
//! sample was added. It is bounded twice over: samples older than
//! `MAX_AGE_SECS` are dropped, and each device keeps at most `MAX_SAMPLES`.
//!
//! Recording happens for everyone; showing the graph and the estimate is
//! part of the unlock (`license::insights`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::devices::Precision;

/// How long a sample is kept.
pub const MAX_AGE_SECS: u64 = 30 * 86_400;
/// At most this many samples per device (a change every 20 minutes, day
/// and night, for a month).
pub const MAX_SAMPLES: usize = 2_000;
/// Only this recent history goes into the estimate: habits change.
pub const WINDOW_SECS: u64 = 14 * 86_400;
/// Estimates longer than this are not shown: too far out to be honest.
pub const LONGEST_SECS: u64 = 60 * 86_400;

const CHARGING: u8 = 1;

/// One change: Unix seconds, level, flags (bit 0: charging). Written as a
/// three-number array to keep the file small.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample(pub u64, pub u8, pub u8);

impl Sample {
    pub fn new(time: u64, level: u8, charging: bool) -> Sample {
        Sample(time, level, if charging { CHARGING } else { 0 })
    }

    pub fn time(self) -> u64 {
        self.0
    }

    pub fn level(self) -> u8 {
        self.1
    }

    pub fn charging(self) -> bool {
        self.2 & CHARGING != 0
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct History {
    #[serde(default)]
    devices: BTreeMap<String, Vec<Sample>>,
}

impl History {
    pub fn samples(&self, id: &str) -> &[Sample] {
        self.devices.get(id).map_or(&[], Vec::as_slice)
    }

    /// Notes a live reading. Adds a sample only if the level or charging
    /// state differs from the last one; returns whether it did.
    pub fn record(&mut self, id: &str, now: u64, level: u8, charging: bool) -> bool {
        if let Some(last) = self.devices.get(id).and_then(|list| list.last())
            && last.level() == level
            && last.charging() == charging
        {
            return false;
        }
        let list = self.devices.entry(id.to_owned()).or_default();
        list.push(Sample::new(now, level, charging));
        if list.len() > MAX_SAMPLES {
            let extra = list.len() - MAX_SAMPLES;
            list.drain(..extra);
        }
        true
    }

    /// Drops samples older than `MAX_AGE_SECS` and the history of devices
    /// that `keep` says are gone (unpaired). Returns whether anything went.
    pub fn compact(&mut self, now: u64, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.devices.len();
        let mut dropped = false;
        self.devices.retain(|id, list| {
            if !keep(id) {
                return false;
            }
            // Oldest first, so only a list whose first sample is old needs a look.
            if list.first().is_some_and(|s| now.saturating_sub(s.time()) > MAX_AGE_SECS) {
                list.retain(|s| now.saturating_sub(s.time()) <= MAX_AGE_SECS);
                dropped = true;
            }
            !list.is_empty()
        });
        dropped || self.devices.len() != before
    }
}

/// How much discharge must have been seen before estimating: points
/// dropped, separate steps, and time. A device that reports in tens moves
/// in big jumps, so it needs a longer baseline; a coarse one never gets an
/// estimate.
fn baseline(precision: Precision) -> Option<(u32, usize, u64)> {
    match precision {
        Precision::Percent => Some((5, 4, 2 * 3600)),
        Precision::Tens => Some((30, 3, 12 * 3600)),
        Precision::Coarse => None,
    }
}

/// Seconds until empty at the rate this device has been discharging, or
/// `None` when that would be a guess: charging, a coarse level, too little
/// history, or an answer more than `LONGEST_SECS` away.
///
/// The rate is the points dropped over the time taken, from every pair of
/// consecutive samples of the last `WINDOW_SECS` in which the level fell
/// and nothing was charging. Wall-clock time, so a pair of headphones used
/// an hour a day gets "days", not "an hour". If the current level has held
/// for longer than a typical step, that time counts too, so a device that
/// has slowed down is not rushed.
pub fn time_left(samples: &[Sample], now: u64, level: u8, precision: Precision, charging: bool) -> Option<u64> {
    if charging {
        return None;
    }
    let (min_drop, min_steps, min_secs) = baseline(precision)?;
    let (mut drop, mut secs, mut steps) = (0u32, 0u64, 0usize);
    for pair in samples.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.charging() || b.charging() || b.level() >= a.level() || b.time() <= a.time() {
            continue;
        }
        if now.saturating_sub(b.time()) > WINDOW_SECS {
            continue;
        }
        drop += u32::from(a.level() - b.level());
        secs += b.time() - a.time();
        steps += 1;
    }
    if drop < min_drop || steps < min_steps || secs < min_secs {
        return None;
    }
    if let Some(last) = samples.last()
        && !last.charging()
        && last.level() == level
    {
        let held = now.saturating_sub(last.time());
        if held > secs / steps as u64 {
            secs += held;
        }
    }
    // A device in tens says 70 for anything from 70 to 79.
    let remaining = match precision {
        Precision::Tens => f64::from(level) + 5.0,
        _ => f64::from(level),
    };
    let left = (remaining * secs as f64 / f64::from(drop)) as u64;
    (left <= LONGEST_SECS).then_some(left)
}

/// A point of the graph: x from 0 (start of the span) to 1 (its end),
/// level 0-100, and whether it was charging from there on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub level: u8,
    pub charging: bool,
}

/// The steps to draw for the `span` seconds before `until`: the level that
/// held at the start, then each change. The last level is held to `until`
/// by the caller. Empty when nothing in the history reaches into the span.
pub fn graph(samples: &[Sample], until: u64, span: u64) -> Vec<Point> {
    let start = until.saturating_sub(span);
    let x = |t: u64| ((t.saturating_sub(start)) as f32 / span.max(1) as f32).clamp(0.0, 1.0);
    let first = samples.iter().position(|s| s.time() > start).unwrap_or(samples.len());
    let mut points = Vec::new();
    // The level as the span began, if known.
    if first > 0 {
        let before = samples[first - 1];
        points.push(Point { x: 0.0, level: before.level(), charging: before.charging() });
    }
    for s in &samples[first..] {
        if s.time() > until {
            break;
        }
        points.push(Point { x: x(s.time()), level: s.level(), charging: s.charging() });
    }
    points
}

fn path() -> std::path::PathBuf {
    crate::settings::data_dir().join("history.json")
}

pub fn load() -> History {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Writes the file whole, through a temporary file so a crash mid-write
/// never leaves half of it.
pub fn save(history: &History) {
    let _ = std::fs::create_dir_all(crate::settings::data_dir());
    let Ok(json) = serde_json::to_string(history) else {
        return;
    };
    let temp = path().with_extension("json.tmp");
    if std::fs::write(&temp, json).is_ok() {
        let _ = std::fs::rename(&temp, path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600;
    const DAY: u64 = 86_400;

    /// A steady discharge from `from` to `to` percent, one sample per point.
    fn discharge(history: &mut History, id: &str, start: u64, from: u8, to: u8, secs_per_point: u64) -> u64 {
        let mut t = start;
        for level in (to..=from).rev() {
            history.record(id, t, level, false);
            t += secs_per_point;
        }
        t - secs_per_point
    }

    #[test]
    fn a_sample_is_kept_only_when_something_changes() {
        let mut h = History::default();
        assert!(h.record("a", 1, 80, false));
        assert!(!h.record("a", 61, 80, false));
        assert!(!h.record("a", 121, 80, false));
        assert!(h.record("a", 181, 79, false));
        assert!(h.record("a", 241, 79, true));
        assert_eq!(h.samples("a"), [Sample::new(1, 80, false), Sample::new(181, 79, false), Sample::new(241, 79, true)]);
        assert!(h.samples("b").is_empty());
    }

    #[test]
    fn history_is_bounded_by_count_age_and_pairing() {
        let mut h = History::default();
        for i in 0..(MAX_SAMPLES as u64 + 50) {
            h.record("a", i, (i % 2) as u8, false);
        }
        assert_eq!(h.samples("a").len(), MAX_SAMPLES);
        assert_eq!(h.samples("a")[0].time(), 50);

        let mut h = History::default();
        h.record("a", 0, 50, false);
        h.record("a", 10 * DAY, 40, false);
        h.record("gone", 10 * DAY, 40, false);
        let now = 35 * DAY;
        // Nothing to do yet: no rewrite.
        assert!(!h.compact(20 * DAY, |id| id != "x"));
        assert!(h.compact(now, |id| id != "gone"));
        assert_eq!(h.samples("a"), [Sample::new(10 * DAY, 40, false)]);
        assert!(h.samples("gone").is_empty());
        assert!(!h.compact(now, |_| true));
        assert!(h.compact(now + 30 * DAY, |_| true));
        assert_eq!(h, History::default());
    }

    #[test]
    fn the_file_is_small_and_round_trips() {
        let mut h = History::default();
        h.record("aabbccddeeff", 1_700_000_000, 80, false);
        h.record("aabbccddeeff", 1_700_000_600, 79, true);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"devices":{"aabbccddeeff":[[1700000000,80,0],[1700000600,79,1]]}}"#);
        assert_eq!(serde_json::from_str::<History>(&json).unwrap(), h);
        assert_eq!(serde_json::from_str::<History>("{}").unwrap(), History::default());
    }

    #[test]
    fn a_steady_discharge_gives_an_estimate() {
        let mut h = History::default();
        // 1% an hour from 100 to 60.
        let end = discharge(&mut h, "a", 0, 100, 60, H);
        let left = time_left(h.samples("a"), end + 60, 60, Precision::Percent, false).unwrap();
        assert!((59 * H..=61 * H).contains(&left), "{left}");
        // Not while charging, never for a coarse level.
        assert_eq!(time_left(h.samples("a"), end, 60, Precision::Percent, true), None);
        assert_eq!(time_left(h.samples("a"), end, 60, Precision::Coarse, false), None);
    }

    #[test]
    fn thin_history_says_nothing() {
        let mut h = History::default();
        // Three points in an hour and a half: too little to go on.
        let end = discharge(&mut h, "a", 0, 80, 77, 30 * 60);
        assert_eq!(time_left(h.samples("a"), end, 77, Precision::Percent, false), None);
        assert_eq!(time_left(&[], 0, 50, Precision::Percent, false), None);
        // Old history alone does not count.
        let mut h = History::default();
        let end = discharge(&mut h, "a", 0, 100, 60, H);
        assert_eq!(time_left(h.samples("a"), end + 20 * DAY, 60, Precision::Percent, false), None);
    }

    #[test]
    fn tens_need_a_longer_baseline() {
        let mut h = History::default();
        // Two steps of ten in four hours: a percent device would have enough
        // points, a tens device not.
        h.record("a", 0, 90, false);
        h.record("a", 2 * H, 80, false);
        h.record("a", 4 * H, 70, false);
        assert_eq!(time_left(h.samples("a"), 4 * H, 70, Precision::Tens, false), None);
        // Three steps over a day: about 3 h per 10 points, 75 points left.
        h.record("a", 24 * H, 60, false);
        let left = time_left(h.samples("a"), 24 * H, 60, Precision::Tens, false).unwrap();
        // 30 points in 24 h, 65 left (60-69).
        assert!((51 * H..=53 * H).contains(&left), "{left}");
    }

    #[test]
    fn charging_and_rises_are_left_out() {
        let mut h = History::default();
        let end = discharge(&mut h, "a", 0, 50, 40, H);
        // On the charger to 100, then down to 95 at the same rate.
        h.record("a", end + H, 41, true);
        h.record("a", end + 3 * H, 100, true);
        let end = discharge(&mut h, "a", end + 4 * H, 100, 95, H);
        let left = time_left(h.samples("a"), end, 95, Precision::Percent, false).unwrap();
        assert!((94 * H..=96 * H).contains(&left), "{left}");
    }

    #[test]
    fn a_level_that_holds_long_slows_the_estimate() {
        let mut h = History::default();
        let end = discharge(&mut h, "a", 0, 100, 90, H);
        let quick = time_left(h.samples("a"), end + H / 2, 90, Precision::Percent, false).unwrap();
        // Still 90% two days later.
        let slow = time_left(h.samples("a"), end + 2 * DAY, 90, Precision::Percent, false).unwrap();
        assert!(slow > quick * 3, "{quick} {slow}");
        // So slow it is not worth saying.
        assert_eq!(time_left(h.samples("a"), end + 13 * DAY, 90, Precision::Percent, false), None);
    }

    #[test]
    fn the_graph_starts_with_the_level_before_the_span() {
        let samples = [Sample::new(0, 90, false), Sample::new(10, 80, false), Sample::new(15, 70, true), Sample::new(25, 75, true)];
        let points = graph(&samples, 20, 10);
        assert_eq!(
            points,
            [
                Point { x: 0.0, level: 80, charging: false },
                Point { x: 0.5, level: 70, charging: true },
            ]
        );
        assert!(graph(&[], 20, 10).is_empty());
        // All of it older than the span: the last level still holds.
        assert_eq!(graph(&samples[..1], 100, 10), [Point { x: 0.0, level: 90, charging: false }]);
    }
}
