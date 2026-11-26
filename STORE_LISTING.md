# Store listing

The English text here matches `app-factory/apps/battery-tray/store/listings/en-us.json`,
which also has the other 13 languages.

## Product name

Battery Tray

## Short description

See the battery level of Bluetooth headphones, AirPods, Xbox and PlayStation controllers, Logitech mice and keyboards, and your laptop in the tray, and get a warning before one runs out.

## Description

Windows keeps the battery level of your Bluetooth devices several clicks deep in Settings, and says nothing when one is about to run out. You find out when the mouse stops or the headphones go quiet in the middle of a call.

Battery Tray shows the battery of your Bluetooth headphones, AirPods, Xbox and PlayStation controllers, Logitech mice and keyboards and your laptop in one place. Click the tray icon for a Windows 11 style flyout, in light or dark. Tick a device and its percentage is drawn into its own tray icon. You get a warning when a battery falls to the level you set, and a pop-up with the level when a device connects.

Every device's battery level is free, with no trial that runs out. So is the connect pop-up, and for one device you choose, a tray icon plus the low-battery and fully charged warnings. An optional one-off unlock (£2.99, bought on our website) adds a tray icon and warnings for every device, the battery history graph and an estimate of how long each battery has left.

What it reads:
- Bluetooth headphones, mice, keyboards and gamepads that report a level to Windows: Bluetooth LE devices with the standard Battery Service, and classic Bluetooth devices whose level Windows already knows.
- AirPods and Beats: left earbud, right earbud and case, and which are charging. They must be connected to this PC. Levels come in steps of 10%, and only your own paired AirPods are shown.
- Xbox controllers: empty, low, medium or full. Xbox controllers report a band, not a percentage, so Battery Tray shows the word and never a made-up number.
- Logitech mice and keyboards on a Unifying, Bolt or Lightspeed USB receiver, and Logitech devices connected by Bluetooth or cable.
- PlayStation DualSense and DualShock 4: level and charging over USB. Over Bluetooth only once another program such as Steam or DS4Windows has put the controller into full-report mode. Battery Tray never changes that.
- Your laptop's own battery.

What it does not read: Logitech headsets, the 2.4 GHz USB receivers of other brands (Razer, SteelSeries and similar), and wired devices that have no battery.

Useful details:
- One card per device with its name, how it connects, its level and whether it is charging. A device that is switched off shows its last level and how long ago that was.
- Click a card for its last 7 days of battery history, and an "about 14 hours left" under a draining device. That is part of the unlock, and it only appears once there is enough history.
- Each device can have its own warning level. The warning appears once per discharge, never while charging. An optional "fully charged" alert is off by default.
- A pop-up shows the level when a device connects after being off for a while, not for what is already connected when the app starts.
- A device that does not report its battery says so. The "Tell us this model" button opens a note with its name and ID for you to read and send, or cancel.
- Closing the window keeps the app in the tray. It starts with Windows by default; one tick in settings turns that off. It checks about once a minute in the background, so the window never freezes.

About accuracy: the number comes from the device itself. Some report in steps of 10 or 20, Xbox controllers in four bands, and a few report nothing at all. Battery Tray shows exactly what the device reports.

AirPods, Beats, Xbox, PlayStation and Logitech are trademarks of their owners. Battery Tray is independent and is not affiliated with or endorsed by them.

No account, no ads, no telemetry.

## Product features

- Battery level of Bluetooth headphones, mice, keyboards and gamepads in one window, plus your laptop's own battery
- AirPods and Beats: left earbud, right earbud and case levels, and which are charging
- Xbox controller battery as empty, low, medium or full, never a made-up percentage
- Logitech mice and keyboards on Unifying, Bolt or Lightspeed USB receivers, Bluetooth or cable
- PlayStation DualSense and DualShock 4 battery over USB, and over Bluetooth once Steam or DS4Windows has set full-report mode
- Windows 11 style flyout from the tray, in light and dark mode
- One tray icon per device, each with its own battery number (free: one device; unlock: any number)
- Low-battery warning at a level you set, per device, once per discharge, plus an optional fully charged alert
- Pop-up with the battery level when a device connects
- Battery history graph and about how many hours or days are left, per device (one-off unlock)
- Says plainly when a device does not report its battery, with a Tell us this model button
- No account, no ads, no telemetry. Installs without admin rights and updates itself

## Search terms

- bluetooth battery
- airpods battery
- xbox controller battery
- logitech mouse battery
- bluetooth headphones battery
- low battery alert
- ps5 controller battery

## Category and subcategory

Utilities & tools

## Pricing

Freemium. Free: the battery level of every device, the connect pop-up, plus one
tray icon and the low-battery and fully charged warnings for one chosen device.
Optional one-off unlock for a tray icon and warnings on every device, the battery
history graph and the time left (£2.99), bought on our website at
https://battery.leatham.net/buy. Store policy 10.8.4 requires this to be stated
in the description, and the age rating answer "Does the app allow users to
purchase digital goods?" to be Yes.

## Privacy

Stored locally in %LOCALAPPDATA%\BatteryTray: settings.json (tray devices,
warning levels, language, licence key), devices.json (each device's name,
Bluetooth address, last battery level and when it was read) and history.json
(each device's level and charging changes for the last 30 days, which feeds the
history graph and the time left). Nothing else is kept.

Network: a version check against GitHub Releases about once a day, which the
user can turn off. No device names, addresses or usage data are sent anywhere.

Feedback: the "What would make this better?" link in the window lets you send the developer a short note. Only the note, the app's name and version, and the platform are sent, to the developer's server at feedback.leatham.net, and only when you press Send. No name, email address or device identifier is sent, and the server does not keep IP addresses. Notes are used only to improve the app and are never shared or sold.

## Screenshots

Made by `app-factory/apps/battery-tray/store/make-shots.ps1` from the app's `--shots` scenes, in all 14 languages.

1. Every kind of device in the flyout, light: AirPods left, right and case, DualSense charging, keyboard, mouse on a USB receiver, the laptop and an Xbox controller's band, each with its level. Caption: "Every battery in one place" / "AirPods left, right and case, controller, mouse, keyboard and your laptop, each with its level."
2. A low battery and its week, light: the warning banner, and the mouse's graph with about how long is left. Caption: "A warning, and time left" / "Warns you at the level you set, once per discharge. The unlock adds a graph of the last week and about how long is left."
3. The flyout over the taskbar's corner, light: three Battery Tray icons (a number, a red one when low, a green one while charging) beside the overflow chevron, network, volume and clock. Caption: "One tray icon per device" / "Each device you tick gets its own icon with its battery number: red when low, green while charging. Click one to open the flyout."
4. The same flyout in dark, unlocked: a card opened to its 7-day graph, and "about 4 days left". Caption: "History in light or dark" / "Click a card for a graph of the last 7 days and about how long is left. The flyout follows your Windows theme."
5. The "Tell us this model" box. Caption: "Honest when a device stays silent" / "If a model does not report its battery, the card says so and you can tell us which model it is."
