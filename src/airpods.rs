//! Left, right and case levels of AirPods and Beats, from the "proximity
//! pairing" Bluetooth LE advertisements they send: manufacturer data with
//! Apple's company ID 0x004C whose first byte is the message type 0x07.
//!
//! The layout (furiousMAC/continuity, OpenPods, AirPodsDesktop), counting
//! from the type byte:
//!
//! | byte | meaning |
//! |------|---------|
//! | 0 | 0x07, proximity pairing |
//! | 1 | length of the rest (0x19 for 27-byte messages) |
//! | 2 | prefix, 0x01 |
//! | 3-4 | model, little-endian: `0E 20` is 0x200E, AirPods Pro; the same number as the PID in the paired device's Windows device ID |
//! | 5 | status; bit 0x20 set means the message comes from the left bud |
//! | 6 | levels: low nibble the bud sending the message, high nibble the other one |
//! | 7 | high nibble charging bits (0x1 sending bud, 0x2 other bud, 0x4 case), low nibble case level |
//! | 8.. | lid counter, colour, then an encrypted part this module does not use |
//!
//! A level nibble is 0-10 in tens of percent; 0xF means that part is not
//! reporting (a bud away from the case, the case closed and far away).
//! These levels are `Precision::Tens`: 7 is shown as 70% and means 70-79%.
//!
//! Advertisements come from rotating random addresses, so they cannot be
//! tied to the paired device by address. They are only used when all of
//! these hold, so a stranger's AirPods on the train are never shown:
//!
//! - a paired AirPods/Beats device with that model (Apple vendor ID and the
//!   model as product ID in its Windows device ID) is connected to this PC,
//!   and only one such device with that model is;
//! - the advertisement is at least `MIN_RSSI` strong;
//! - if several addresses advertise that model, the strongest is at least
//!   `CLEAR_MARGIN` dB ahead of the next; otherwise nothing is shown.
//!
//! Scanning is passive (the radio only listens, it sends no scan requests)
//! and lasts a few seconds per poll (`source::SCAN_WINDOW`), not
//! continuously. A level seen in one window is kept for `KEEP` while the
//! AirPods stay connected, so a missed window does not blank the card.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEManufacturerData, BluetoothLEScanningMode,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::devices::{self, Battery, Kind, Part, PartKind, Precision, Reading};

pub const APPLE: u16 = 0x004c;
const PROXIMITY_PAIRING: u8 = 0x07;
/// Weaker than this is too far away to be on the user's head or desk.
pub const MIN_RSSI: i16 = -80;
/// How far ahead the strongest advertisement must be when several
/// addresses send the same model.
pub const CLEAR_MARGIN: i16 = 10;
/// How long levels from one scan stay on a connected device.
pub const KEEP: Duration = Duration::from_secs(300);

/// Model IDs seen in proximity pairing messages and in the PID of the
/// paired device (OpenPods, AirPodsDesktop). `true` for one-piece
/// headphones, which report one level and no case.
pub const MODELS: &[(u16, &str, bool)] = &[
    (0x2002, "AirPods", false),
    (0x200f, "AirPods (2nd generation)", false),
    (0x2013, "AirPods (3rd generation)", false),
    (0x2019, "AirPods 4", false),
    (0x201b, "AirPods 4 (ANC)", false),
    (0x200e, "AirPods Pro", false),
    (0x2014, "AirPods Pro (2nd generation)", false),
    (0x2024, "AirPods Pro (2nd generation, USB-C)", false),
    (0x2027, "AirPods Pro 3", false),
    (0x200a, "AirPods Max", true),
    (0x201f, "AirPods Max (USB-C)", true),
    (0x202d, "AirPods Max (USB-C)", true),
    (0x2003, "Powerbeats3", true),
    (0x2005, "BeatsX", true),
    (0x2006, "Beats Solo3", true),
    (0x2009, "Beats Studio3", true),
    (0x200b, "Powerbeats Pro", false),
    (0x200c, "Beats Solo Pro", true),
    (0x2010, "Beats Flex", true),
    (0x2011, "Beats Studio Buds", false),
    (0x2012, "Beats Fit Pro", false),
    (0x2016, "Beats Studio Buds +", false),
    (0x2017, "Beats Studio Pro", true),
];

pub fn model_name(model: u16) -> Option<&'static str> {
    MODELS.iter().find(|m| m.0 == model).map(|m| m.1)
}

fn one_piece(model: u16) -> bool {
    MODELS.iter().any(|m| m.0 == model && m.2)
}

/// What one proximity pairing message says.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub model: u16,
    /// Left, right and case for earbuds; empty for one-piece headphones.
    pub parts: Vec<Part>,
    /// The headline: lowest bud, or the one level of one-piece headphones.
    pub level: Option<u8>,
    pub charging: Option<bool>,
}

fn nibble_level(nibble: u8) -> Option<u8> {
    (nibble <= 10).then_some(nibble * 10)
}

/// Decodes Apple manufacturer data (after the company ID). `None` if it is
/// not a proximity pairing message or is too short.
pub fn parse(data: &[u8]) -> Option<Message> {
    if data.len() < 9 || data[0] != PROXIMITY_PAIRING || (data[1] as usize) + 2 > data.len() || data[1] < 7 {
        return None;
    }
    let model = u16::from_le_bytes([data[3], data[4]]);
    let from_left = data[5] & 0x20 != 0;
    let (this_level, other_level) = (data[6] & 0x0f, data[6] >> 4);
    let charge = data[7] >> 4;
    let (this_charging, other_charging, case_charging) = (charge & 1 != 0, charge & 2 != 0, charge & 4 != 0);
    let case_level = data[7] & 0x0f;

    if one_piece(model) {
        let level = nibble_level(this_level);
        return Some(Message {
            model,
            parts: Vec::new(),
            level,
            charging: level.map(|_| this_charging),
        });
    }
    let (left, right, left_charging, right_charging) = if from_left {
        (this_level, other_level, this_charging, other_charging)
    } else {
        (other_level, this_level, other_charging, this_charging)
    };
    let part = |kind, nibble, charging| Part {
        kind,
        level: nibble_level(nibble),
        charging,
    };
    let parts = vec![
        part(PartKind::Left, left, left_charging),
        part(PartKind::Right, right, right_charging),
        part(PartKind::Case, case_level, case_charging),
    ];
    Some(Message {
        model,
        level: devices::headline(&parts),
        charging: devices::headline_charging(&parts),
        parts,
    })
}

/// One advertisement heard: who sent it (a rotating address), how strong,
/// and the Apple manufacturer data.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub address: u64,
    pub rssi: i16,
    pub data: Vec<u8>,
}

/// The message to use for a paired device of this model, if one address
/// is clearly the user's (see the module comment).
pub fn choose(heard: &[Heard], model: u16) -> Option<Message> {
    // The strongest message per address.
    let mut best: HashMap<u64, (i16, Message)> = HashMap::new();
    for h in heard.iter().filter(|h| h.rssi >= MIN_RSSI) {
        let Some(message) = parse(&h.data).filter(|m| m.model == model) else {
            continue;
        };
        let entry = best.entry(h.address).or_insert((h.rssi, message.clone()));
        if h.rssi >= entry.0 {
            *entry = (h.rssi, message);
        }
    }
    let mut ranked: Vec<(i16, Message)> = best.into_values().collect();
    ranked.sort_by_key(|(rssi, _)| -rssi);
    match ranked.as_slice() {
        [] => None,
        [(_, only)] => Some(only.clone()),
        [(first, message), (second, _), ..] => (first - second >= CLEAR_MARGIN).then(|| message.clone()),
    }
}

/// Whether a scan could find anything: some connected device is Apple's and
/// could be AirPods or Beats (a known model, or one that is not obviously
/// a mouse or keyboard). Otherwise the radio is left alone.
pub fn wanted(readings: &[Reading]) -> bool {
    readings.iter().any(|r| match r.hardware {
        Some((APPLE, product)) if r.connected => {
            model_name(product).is_some() || !matches!(r.kind, Kind::Mouse | Kind::Keyboard | Kind::Gamepad)
        }
        _ => false,
    })
}

/// Puts AirPods levels on the paired devices, keeping each for `KEEP`.
#[derive(Default)]
pub struct Airpods {
    last: HashMap<String, (Instant, Message)>,
}

impl Airpods {
    pub fn attach(&mut self, readings: &mut [Reading], heard: &[Heard], now: Instant) {
        let apple = |r: &Reading| match r.hardware {
            Some((APPLE, product)) if r.connected => Some(product),
            _ => None,
        };
        let models: Vec<u16> = readings.iter().filter_map(apple).collect();
        for reading in readings.iter_mut() {
            let Some(model) = apple(reading) else {
                self.last.remove(&reading.id);
                continue;
            };
            // Two connected AirPods of the same model: no way to tell whose
            // advertisement is whose.
            if models.iter().filter(|m| **m == model).count() > 1 {
                continue;
            }
            if let Some(message) = choose(heard, model) {
                self.last.insert(reading.id.clone(), (now, message));
            }
            let Some((when, message)) = self.last.get(&reading.id) else {
                continue;
            };
            if now.duration_since(*when) > KEEP {
                self.last.remove(&reading.id);
                continue;
            }
            apply(reading, message);
        }
    }
}

fn apply(reading: &mut Reading, message: &Message) {
    if reading.kind == Kind::Other && model_name(message.model).is_some() {
        reading.kind = Kind::Headphones;
    }
    reading.parts = message.parts.clone();
    if let Some(level) = message.level {
        reading.battery = Battery::Level(level);
        reading.precision = Precision::Tens;
        reading.charging = message.charging;
    }
}

/// A passive scan for Apple advertisements, from `start` to `finish`.
pub struct Scan {
    watcher: BluetoothLEAdvertisementWatcher,
    token: i64,
    heard: Arc<Mutex<Vec<Heard>>>,
}

/// Enough for a few seconds of advertisements from a busy room.
const MAX_HEARD: usize = 2000;

impl Scan {
    /// `None` if Windows has no Bluetooth LE to listen with.
    pub fn start() -> Option<Scan> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        Scan::try_start().ok()
    }

    fn try_start() -> windows::core::Result<Scan> {
        let watcher = BluetoothLEAdvertisementWatcher::new()?;
        watcher.SetScanningMode(BluetoothLEScanningMode::Passive)?;
        // Let Windows drop everything that is not from Apple.
        let filter = BluetoothLEManufacturerData::new()?;
        filter.SetCompanyId(APPLE)?;
        let writer = DataWriter::new()?;
        writer.WriteByte(PROXIMITY_PAIRING)?;
        filter.SetData(&writer.DetachBuffer()?)?;
        watcher.AdvertisementFilter()?.Advertisement()?.ManufacturerData()?.Append(&filter)?;

        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let token = watcher.Received(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    let _ = record(args, &sink);
                }
                Ok(())
            },
        ))?;
        watcher.Start()?;
        Ok(Scan { watcher, token, heard })
    }

    /// Stops listening and returns what was heard.
    pub fn finish(self) -> Vec<Heard> {
        let _ = self.watcher.Stop();
        let _ = self.watcher.RemoveReceived(self.token);
        std::mem::take(&mut *self.heard.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

fn record(args: &BluetoothLEAdvertisementReceivedEventArgs, sink: &Mutex<Vec<Heard>>) -> windows::core::Result<()> {
    let sections = args.Advertisement()?.GetManufacturerDataByCompanyId(APPLE)?;
    for i in 0..sections.Size()? {
        let buffer = sections.GetAt(i)?.Data()?;
        let mut data = vec![0u8; buffer.Length()? as usize];
        DataReader::FromBuffer(&buffer)?.ReadBytes(&mut data)?;
        if data.first() != Some(&PROXIMITY_PAIRING) {
            continue;
        }
        let mut heard = sink.lock().unwrap_or_else(|e| e.into_inner());
        if heard.len() < MAX_HEARD {
            heard.push(Heard {
                address: args.BluetoothAddress()?,
                rssi: args.RawSignalStrengthInDBm()?,
                data,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::Transport;

    /// A 27-byte AirPods Pro message: sent by the left bud (status 0x2B has
    /// bit 0x20), left 8 (80%), right 7 (70%), case 4 (40%), case charging,
    /// buds not charging.
    const PRO_FROM_LEFT: [u8; 27] = [
        0x07, 0x19, 0x01, 0x0e, 0x20, 0x2b, 0x78, 0x44, 0x05, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ];

    fn with(base: [u8; 27], edits: &[(usize, u8)]) -> Vec<u8> {
        let mut data = base.to_vec();
        for (i, v) in edits {
            data[*i] = *v;
        }
        data
    }

    fn level(m: &Message, kind: PartKind) -> (Option<u8>, bool) {
        let p = m.parts.iter().find(|p| p.kind == kind).unwrap();
        (p.level, p.charging)
    }

    #[test]
    fn decodes_buds_case_and_the_model() {
        let m = parse(&PRO_FROM_LEFT).unwrap();
        assert_eq!(m.model, 0x200e);
        assert_eq!(model_name(m.model), Some("AirPods Pro"));
        assert_eq!(level(&m, PartKind::Left), (Some(80), false));
        assert_eq!(level(&m, PartKind::Right), (Some(70), false));
        assert_eq!(level(&m, PartKind::Case), (Some(40), true));
        assert_eq!(m.level, Some(70));
        assert_eq!(m.charging, Some(false));
    }

    #[test]
    fn a_message_from_the_right_bud_is_flipped() {
        // Status 0x0B: bit 0x20 clear, so the low nibble is the right bud.
        // Charging nibble 0x1: the sending (right) bud is charging.
        let m = parse(&with(PRO_FROM_LEFT, &[(5, 0x0b), (7, 0x14)])).unwrap();
        assert_eq!(level(&m, PartKind::Right), (Some(80), true));
        assert_eq!(level(&m, PartKind::Left), (Some(70), false));
        assert_eq!(level(&m, PartKind::Case), (Some(40), false));
        // The headline is the lowest bud, left at 70, not charging.
        assert_eq!(m.level, Some(70));
        assert_eq!(m.charging, Some(false));
    }

    #[test]
    fn unknown_nibbles_are_not_levels() {
        // Left 0xF (away), right 10 (100%), case 0xF (closed, far).
        let m = parse(&with(PRO_FROM_LEFT, &[(6, 0xaf), (7, 0x2f)])).unwrap();
        assert_eq!(level(&m, PartKind::Left).0, None);
        assert_eq!(level(&m, PartKind::Right), (Some(100), true));
        assert_eq!(level(&m, PartKind::Case).0, None);
        assert_eq!(m.level, Some(100));
        assert_eq!(m.charging, Some(true));
        // Nothing but the case: no headline.
        let m = parse(&with(PRO_FROM_LEFT, &[(6, 0xff), (7, 0x05)])).unwrap();
        assert_eq!(m.level, None);
        assert_eq!(level(&m, PartKind::Case).0, Some(50));
        // 11-14 are not levels either.
        assert_eq!(parse(&with(PRO_FROM_LEFT, &[(6, 0xbb)])).unwrap().level, None);
    }

    #[test]
    fn one_piece_headphones_have_one_level_and_no_case() {
        // AirPods Max (0x200A), level 6, charging.
        let m = parse(&with(PRO_FROM_LEFT, &[(3, 0x0a), (6, 0xf6), (7, 0x1f)])).unwrap();
        assert_eq!(model_name(m.model), Some("AirPods Max"));
        assert!(m.parts.is_empty());
        assert_eq!(m.level, Some(60));
        assert_eq!(m.charging, Some(true));
    }

    #[test]
    fn other_apple_messages_and_short_data_are_ignored() {
        assert!(parse(&[0x10, 0x05, 0x01, 0x18, 0x44, 0x00, 0x00]).is_none());
        assert!(parse(&PRO_FROM_LEFT[..8]).is_none());
        assert!(parse(&with(PRO_FROM_LEFT, &[(0, 0x12)])).is_none());
        // A length byte longer than the data.
        assert!(parse(&with(PRO_FROM_LEFT, &[(1, 0x40)])).is_none());
    }

    fn heard(address: u64, rssi: i16, data: Vec<u8>) -> Heard {
        Heard { address, rssi, data }
    }

    #[test]
    fn only_a_clear_strongest_advertisement_is_used() {
        let mine = PRO_FROM_LEFT.to_vec();
        let theirs = with(PRO_FROM_LEFT, &[(6, 0x22)]);
        // Alone and close.
        assert_eq!(choose(&[heard(1, -50, mine.clone())], 0x200e).unwrap().level, Some(70));
        // Too far away.
        assert!(choose(&[heard(1, -90, mine.clone())], 0x200e).is_none());
        // Another model is not ours.
        assert!(choose(&[heard(1, -50, mine.clone())], 0x2014).is_none());
        // Two close together: can't tell.
        assert!(choose(&[heard(1, -50, mine.clone()), heard(2, -55, theirs.clone())], 0x200e).is_none());
        // One clearly closer wins; repeats from one address count once.
        let picked = choose(
            &[heard(2, -75, theirs.clone()), heard(1, -48, mine.clone()), heard(1, -52, mine.clone()), heard(2, -74, theirs)],
            0x200e,
        )
        .unwrap();
        assert_eq!(picked.level, Some(70));
    }

    fn paired(id: &str, product: u16, connected: bool) -> Reading {
        Reading {
            connected,
            hardware: Some((APPLE, product)),
            ..Reading::new(id, "AirPods Pro", Kind::Headphones, Transport::Bluetooth)
        }
    }

    #[test]
    fn levels_go_only_to_connected_paired_airpods_of_that_model() {
        let t0 = Instant::now();
        let mut airpods = Airpods::default();
        let heard = [heard(1, -50, PRO_FROM_LEFT.to_vec())];
        let mut readings = vec![
            paired("pro", 0x200e, true),
            paired("pro2", 0x2014, true),
            paired("off", 0x200e, false),
            Reading::new("mouse", "Mouse", Kind::Mouse, Transport::Le),
        ];
        airpods.attach(&mut readings, &heard, t0);
        assert_eq!(readings[0].battery, Battery::Level(70));
        assert_eq!(readings[0].precision, Precision::Tens);
        assert_eq!(readings[0].parts.len(), 3);
        assert_eq!(readings[1].battery, Battery::NotReported);
        assert_eq!(readings[2].battery, Battery::NotReported);
        assert!(readings[3].parts.is_empty());

        // A window with nothing heard keeps the levels for a while...
        let mut readings = vec![paired("pro", 0x200e, true)];
        airpods.attach(&mut readings, &[], t0 + Duration::from_secs(60));
        assert_eq!(readings[0].battery, Battery::Level(70));
        // ...but not forever.
        let mut readings = vec![paired("pro", 0x200e, true)];
        airpods.attach(&mut readings, &[], t0 + KEEP + Duration::from_secs(61));
        assert_eq!(readings[0].battery, Battery::NotReported);
    }

    #[test]
    fn the_radio_listens_only_while_apple_audio_is_connected() {
        let apple = |product, kind, connected| Reading {
            hardware: Some((APPLE, product)),
            connected,
            ..Reading::new("a", "x", kind, Transport::Bluetooth)
        };
        let other = Reading { hardware: Some((0x054c, 0x0cd3)), ..Reading::new("s", "Sony", Kind::Headphones, Transport::Bluetooth) };
        assert!(!wanted(&[]));
        assert!(!wanted(std::slice::from_ref(&other)));
        assert!(wanted(&[other.clone(), apple(0x2014, Kind::Other, true)]));
        // Paired but off.
        assert!(!wanted(&[apple(0x2014, Kind::Headphones, false)]));
        // A model not in the table yet, as headphones: worth a listen.
        assert!(wanted(&[apple(0x2099, Kind::Headphones, true)]));
        // A Magic Mouse or Keyboard is not.
        assert!(!wanted(&[apple(0x0269, Kind::Mouse, true), apple(0x0267, Kind::Keyboard, true)]));
    }

    #[test]
    fn two_connected_airpods_of_the_same_model_get_nothing() {
        let mut airpods = Airpods::default();
        let heard = [heard(1, -50, PRO_FROM_LEFT.to_vec())];
        let mut readings = vec![paired("a", 0x200e, true), paired("b", 0x200e, true)];
        airpods.attach(&mut readings, &heard, Instant::now());
        assert!(readings.iter().all(|r| r.battery == Battery::NotReported));
    }

    /// Listens for real for a few seconds and prints what Apple devices
    /// nearby advertise. `cargo test scan_for_real -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scan_for_real() {
        let scan = Scan::start().expect("no Bluetooth LE");
        std::thread::sleep(Duration::from_secs(8));
        let heard = scan.finish();
        println!("heard {} proximity pairing advertisements", heard.len());
        for h in &heard {
            println!("{:012x} {} dBm {:02x?} -> {:?}", h.address, h.rssi, h.data, parse(&h.data));
        }
    }
}
