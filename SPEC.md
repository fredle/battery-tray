# Battery Tray

Battery level of your Bluetooth headphones, mouse and keyboard in the tray, with low warnings.

## Problem

Windows keeps the battery level of Bluetooth devices several clicks deep in Settings and gives no warning when one is about to run out. People find out when the mouse stops or the headphones cut out.

## Features (1.0)

- Read the battery level of paired Bluetooth devices: classic Bluetooth from the level Windows holds for the device, Bluetooth LE from the GATT Battery Service
- AirPods and Beats: left bud, right bud and case levels (in steps of 10%) and which are charging, from Apple's proximity pairing Bluetooth LE advertisements, shown only on the user's own paired and connected AirPods (matched by model and signal strength, never a stranger's); scanned passively for a few seconds per poll and only while an Apple audio device is connected, never continuously
- Xbox controllers through XInput (works without focus): empty / low / medium / full, shown as words, never as a percentage; merged into the Bluetooth card when it is unambiguous which controller is which
- Logitech mice and keyboards on a Unifying, Bolt or Lightspeed receiver, and Logitech Bluetooth or wired devices, over HID++ 2.0 (Unified Battery, Battery Status, Battery Voltage as a band); a sleeping device keeps its last level
- PlayStation DualSense and DualShock 4 levels and charging from their HID input reports, on USB and Bluetooth (on Bluetooth only once another program has switched the controller to full reports; the app never switches it)
- The PC's own battery, on laptops only
- One card per device even when several sources see it; a source that fails keeps its last readings and never blanks the others
- Each card says how the device is connected (Bluetooth, Bluetooth LE, USB receiver, USB, This PC) and whether it is charging; no warning while a device charges
- Tray icons: one per device the user ticks, each with its own icon id so Windows remembers whether it was dragged onto the taskbar (the first keeps v0.1.4's id). The level is drawn as a number in the taskbar's text colour (light or dark taskbar, followed live) over a thin level bar: red at the device's warning level, green while charging. Coarse devices show a battery with 0-3 segments, 100% a full battery, never a made-up number; with no device to show, the app's own icon. Rendered at the size Windows uses for tray icons at the current scale (16, 20, 24, 32 px...), not scaled. Each icon's tooltip names its device; the first also lists the devices without an icon; every icon has the same menu
- Left-click any tray icon: a Windows 11 style flyout in the corner of the screen next to the tray (whichever edge the taskbar is on, on the monitor that has it, at that monitor's scale), borderless with Windows' rounded corners and shadow, as tall as its content, closed by clicking elsewhere, Esc or the icon again. Its pin button turns it into an ordinary window that stays; the tray menu's Open and starting the app from the Start menu open that window directly. Solid colours, no translucent backdrop (not reliable with the OpenGL renderer)
- Light and dark, following Windows' app setting live
- Closing the window hides it to the tray; the tray menu has Open and Quit
- One card per device: a drawn picture of its kind, name, connection, level (a word for coarse devices), a level bar or, for AirPods, three labelled bars for left, right and case, a charging bolt, and for a device that is off its last level and how long ago. A device that does not report its level says so, explains that some models only tell their maker's app, and offers "Tell us this model": it opens the feedback box with the name, kind, connection and Bluetooth vendor/product ID already written in, for the user to read, edit and send or cancel
- Clicking a card opens its battery history: a step graph of the last 7 days with charging in green and the warning level dashed. Under a discharging device's name, "about N hours/days left" from that history (wall-clock time at the device's usual use), shown only with enough history (Tens-precision devices need a longer baseline), never for coarse levels, never more than 60 days out
- Battery history: a sample whenever a device's level or charging state changes, kept 30 days and at most 2,000 samples per device, forgotten with the device; written only when a sample is added (history.json)
- Low-battery warning at a level the user sets (default 20%), or a level set for one device; once per discharge, re-armed when the level rises above it; never while charging
- "Fully charged" notification once per charge, for devices that report charging (off by default)
- "<name> connected - 82%" notification (with left, right and case for AirPods) when a device connects after being off for at least 15 minutes; never for what is connected when the app starts (on by default)
- Notifications are the tray icon's balloons, which Windows 10 and 11 show as toasts without an AppUserModelID or Start-menu shortcut; from the device's own icon if it has one
- First run: a hint that Windows hides new tray icons, how to bring Battery Tray out, and a button to the taskbar settings; dismissed for good with "Got it"
- A settings page in the same window: default warning level, each device's tray icon and warning level, the two notifications, start with Windows, language, and the unlock
- "Start with Windows" (current user's Run key, no admin rights), switched on once by default on an installed copy; unticking it in settings keeps it off
- Polls in a background thread (background priority, the process in efficiency mode): about once a minute while a device is connected and someone is at the screen; every five minutes while the session is locked, the screen is off or nothing but the PC's own battery is connected; twice as far apart under battery or energy saver. A device arriving or leaving, waking from sleep, a power status change and opening the window bring the next poll forward, so a connect pop-up still comes within seconds, and opening the window polls at once if the last poll is older than a regular gap. HID collections are listed again only when devices change; the window never waits for Bluetooth
- Light in the tray: what stays running all day is a small plain Win32 process (tray icons, menu, notifications, polling; a few MB). The window is the same exe started with `--ui` when it is wanted (started already while the pointer is on a tray icon), and ends itself 20 seconds after it is hidden. Nothing is written to disk while nothing changes (devices.json is written when something it keeps changes, a device connects or goes off, and on Quit, sleep and shutdown)
- Free: every device's level in the window, tooltip and menu, the "connected" notification, plus one tray icon and the low-battery and fully charged warnings for one chosen device. A one-off unlock (£2.99, licence key, checked offline) gives warnings and a tray icon for every device, the battery history graph and the time left. History is recorded for everyone, so it is there on the day of unlocking
- Screenshot mode for development and the Store: `battery_tray.exe --shots <folder> [--theme light|dark] [--scale 1.5] [--lang de]` draws made-up scenes and writes each as PNG, then exits; nothing real happens in it

## Non-goals

- No pairing, connecting or audio switching
- Nothing is ever written to a device except HID++ battery and name queries; no feature reports that change a controller's mode
- No percentages made up from coarse levels or voltages
- No vendor protocols beyond those above (no Razer, SteelSeries, Corsair, HyperX or Sony headsets; no Logitech headsets)
- No continuous Bluetooth scanning
