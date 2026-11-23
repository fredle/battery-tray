# Bluetooth Battery Tray

Battery level of your Bluetooth headphones, mouse and keyboard in the tray, with low warnings.

Windows keeps the battery level of Bluetooth devices several clicks deep in
Settings and says nothing when one is about to run out. Bluetooth Battery Tray draws the
percentage of one device into a tray icon, lists every device in the tooltip,
and warns when a battery falls to the level you set.

## What it does

- Shows a card for each paired Bluetooth device: name, kind, a level bar and
  whether it is connected.
- A device that is off shows its last level and how long ago that was.
- A device that gives Windows no battery level is labelled "This device does
  not report its battery level". The app never guesses.
- The tray icon shows the percentage of the device you choose (**Tray icon**
  on its card). The tooltip and the right-click menu list every device.
- Warns when a battery falls to the threshold (5-50%, default 20%): a Windows
  notification and a banner in the window. One warning per discharge; it is
  armed again once the level is back above the threshold.
- Closing the window hides it; the app stays in the tray. The tray menu has
  **Open** and **Quit**. Starting the app again opens the running copy.
- **Start with Windows** adds an entry under the current user's Run key (no
  admin rights). Started that way, the app opens straight into the tray.
- Checks about once a minute on a background thread, and at once when you
  press **Refresh**.

## Free and unlocked

Free:

- the battery level of every device, in the window, the tooltip and the menu;
- the tray icon and the low-battery warning for **one** device you choose.

A one-off unlock (£2.99, at https://battery.leatham.net/buy, paid through
Stripe) adds low-battery warnings for **every** device. There is no
subscription. A licence key is the buyer's email signed by the licence server;
paste it into **Unlock** and it is checked offline. If `BUY_URL` in
`src/license.rs` is emptied, every device gets warnings.

## Where the numbers come from

| Device | Source |
|---|---|
| Bluetooth LE | The GATT Battery Service (0x180F), Battery Level characteristic (0x2A19), through WinRT `Windows.Devices.Bluetooth`. |
| Classic Bluetooth | The level Windows' Bluetooth stack keeps on the device (device property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` 2, read with CfgMgr32). It is the number Windows Settings shows. Microsoft does not document this property. |

Only connected devices are asked, so the app never makes a device connect.
The LE read falls back to the Windows property when a device refuses it.

What it cannot read: separate levels for each earbud and the case (those need
each maker's own protocol), wired devices, devices on a 2.4 GHz USB receiver,
and the laptop's own battery. It does not pair, connect or switch audio.

## Languages

The window follows Windows' display language, and the picker in the footer
overrides it: English, German, French, Spanish, Italian, Portuguese, Dutch,
Polish, Turkish, Russian, Japanese, Korean, and Simplified and Traditional
Chinese. All text is in `src/i18n.rs`; a test checks that every language has
every text with the same placeholders. Japanese, Korean and Chinese are drawn
with Windows' own fonts, loaded only when needed. Right-to-left and
complex-script languages are not offered because egui cannot shape them.
The tray menu uses the same language. The licence server's pages
(`server/src/i18n.ts`) use the same languages, chosen from the browser's
`Accept-Language`.

## Install

- Microsoft Store: search for "Bluetooth Battery Tray".
- Or download the installer from
  [GitHub Releases](https://github.com/fredle/battery-tray/releases).

It installs per user, needs no admin rights and updates itself.

## Data and privacy

- Settings (tray device, threshold, language, licence key) are in
  `%LOCALAPPDATA%\BatteryTray\settings.json`. The last level seen for each
  device is in `devices.json` beside it.
- No telemetry and no accounts. The only network use is the update check
  against GitHub Releases, about once a day, which can be turned off in the
  footer. Buying an unlock opens your browser; the app itself sends nothing.

## The licence server

`server/` is a small Node service with no database (see `server/README.md`):

- `GET /buy` redirects to the Stripe Payment Link.
- `GET /thanks?session_id=…` checks with Stripe that the checkout was paid and
  shows the licence key for the buyer's email.

The signing key is `server/deploy/secrets/license-signing-key.pem`. It is
git-ignored and never deployed by the factory: copy it by hand to
`/srv/apps/battery-tray/deploy/secrets/` on the host and keep a backup,
because the app only accepts keys signed by it.

## Development

```text
cargo build --release
cargo test
```

Source layout: `devices.rs` holds the rules (merging a poll, when a warning
fires, the tooltip text), `license.rs` the free and unlocked rule, `bluetooth.rs` talks to
Windows, `monitor.rs` is the polling thread, `tray.rs` the tray icon and menu,
`icon.rs` draws the icon, `app.rs` the window.

### Simulated devices

Reading devices sits behind the `Source` trait (`src/source.rs`). Set
`BT_FAKE_DEVICES` to a JSON list, or to the path of a JSON file, to run the
app without any Bluetooth hardware. A file is read again on every poll, so
editing it changes what the app shows.

```json
[
  {"name": "WH-1000XM4", "kind": "headphones", "level": 82},
  {"name": "MX Master 3S", "kind": "mouse", "level": 15, "le": true},
  {"name": "Old keyboard", "kind": "keyboard"},
  {"name": "Pixel Buds", "kind": "headphones", "connected": false}
]
```

`kind` is `headphones`, `mouse`, `keyboard`, `gamepad` or `other`. A connected
device with no `level` is one that does not report its battery;
`"unreadable": true` simulates a failed read. Other switches:

| Variable | Effect |
|---|---|
| `BT_POLL_SECS` | Seconds between polls (default 60). |
| `BT_QUIET` | Set to anything to suppress Windows notifications. |
| `HWC_THEME` | `dark` or `light` forces the theme. |
| `HWC_LICENSE_KEY` | A key issued for `buyer@example.com`; makes one test check it against the embedded public key. |

To see a low-battery warning: give the tray device (the first device with a
level, until you choose one) a `level` at or below the threshold. With a file,
lower the number while the app runs and the warning appears on the next poll.

The Bluetooth code in `bluetooth.rs` cannot be unit-tested and has only been
run on a PC with one paired, switched-off device; the tests cover everything
above it.
