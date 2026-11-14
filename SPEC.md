# Battery Tray

Battery level of your Bluetooth headphones, mouse and keyboard in the tray, with low warnings.

## Problem

Windows keeps the battery level of Bluetooth devices several clicks deep in Settings and gives no warning when one is about to run out. People find out when the mouse stops or the headphones cut out.

## Features (1.0)

- Read the battery level of paired Bluetooth devices: classic Bluetooth from the level Windows holds for the device, Bluetooth LE from the GATT Battery Service
- Tray icon with the percentage of one chosen device drawn into it; every device in the tooltip and the tray menu
- Closing the window hides it to the tray; the tray menu has Open and Quit
- A window with one card per device: name, kind, level bar, and for a device that is off, its last level and how long ago
- A plain "this device does not report its battery level" instead of a guess
- Low-battery warning at a threshold the user sets (default 20%), once per discharge, re-armed when the level rises above the threshold
- "Start with Windows" (current user's Run key, no admin rights)
- Polls in a background thread about once a minute; the window never waits for Bluetooth
- Free: every device's level in the window, tooltip and menu, plus the tray icon and the low-battery warning for one chosen device. A one-off unlock (£2.99, licence key, checked offline) gives low-battery warnings for every device

## Non-goals

- No vendor protocols: no separate levels for each earbud and the case
- No pairing, connecting or audio switching
- No laptop battery
- No wired devices and no 2.4 GHz USB receivers
