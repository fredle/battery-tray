# Battery Tray

Battery level of your Bluetooth headphones, mouse and keyboard in the tray, with low warnings.

## Problem

Windows keeps the battery level of Bluetooth devices several clicks deep in Settings and gives no warning when one is about to run out. People find out when the mouse stops or the headphones cut out.

## Features (1.0)

- Read the battery level of paired Bluetooth devices: classic Bluetooth from the level Windows holds for the device, Bluetooth LE from the GATT Battery Service
- AirPods and Beats: left bud, right bud and case levels (in steps of 10%) and which are charging, from Apple's proximity pairing Bluetooth LE advertisements, shown only on the user's own paired and connected AirPods (matched by model and signal strength, never a stranger's); scanned passively for a few seconds per poll, not continuously
- Xbox controllers through XInput (works without focus): empty / low / medium / full, shown as words, never as a percentage; merged into the Bluetooth card when it is unambiguous which controller is which
- Logitech mice and keyboards on a Unifying, Bolt or Lightspeed receiver, and Logitech Bluetooth or wired devices, over HID++ 2.0 (Unified Battery, Battery Status, Battery Voltage as a band); a sleeping device keeps its last level
- PlayStation DualSense and DualShock 4 levels and charging from their HID input reports, on USB and Bluetooth (on Bluetooth only once another program has switched the controller to full reports; the app never switches it)
- The PC's own battery, on laptops only
- One card per device even when several sources see it; a source that fails keeps its last readings and never blanks the others
- Each card says how the device is connected (Bluetooth, Bluetooth LE, USB receiver, USB, This PC) and whether it is charging; no warning while a device charges
- Tray icon with the percentage of one chosen device drawn into it; every device in the tooltip and the tray menu
- Closing the window hides it to the tray; the tray menu has Open and Quit
- A window with one card per device: name, kind, level bar, and for a device that is off, its last level and how long ago
- A plain "this device does not report its battery level" instead of a guess
- Low-battery warning at a threshold the user sets (default 20%), once per discharge, re-armed when the level rises above the threshold
- "Start with Windows" (current user's Run key, no admin rights)
- Polls in a background thread about once a minute; the window never waits for Bluetooth
- Free: every device's level in the window, tooltip and menu, plus the tray icon and the low-battery warning for one chosen device. A one-off unlock (£2.99, licence key, checked offline) gives low-battery warnings for every device

## Non-goals

- No pairing, connecting or audio switching
- Nothing is ever written to a device except HID++ battery and name queries; no feature reports that change a controller's mode
- No percentages made up from coarse levels or voltages
- No vendor protocols beyond those above (no Razer, SteelSeries, Corsair, HyperX or Sony headsets; no Logitech headsets)
- No continuous Bluetooth scanning
