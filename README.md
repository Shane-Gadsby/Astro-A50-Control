# A50 Control

Linux companion app for the **Astro A50 Gen 4** headset and base station: headset
settings, battery in the system battery menu, and automatic audio output switching.
A Tauri (Rust + web UI) app, packaged as an AppImage. It replaces the older
`a50-watcher` Node daemon.

## Features

- **Battery** — shown in the tray, in GNOME's battery UI / UPower (as a headset), with
  low-battery notifications at 15% and 5%.
- **Audio switching** — default sink goes to the A50 when it's switched on and off the
  dock, otherwise to the Sony WH-CH720N if connected, otherwise to the speakers. Sinks
  are matched by stable device properties, so PipeWire renames don't break it.
- **Headset settings** — auto power-off (10–30 min), 3 EQ presets (5-band gain,
  frequency, Q, names), mic level, sidetone, noise gate, mic EQ, stream port mix,
  default game/chat balance, alert volume. Changes apply live and are saved to the
  headset's flash a few seconds after the last change.
- Runs in the tray; closing the window keeps it running. Optional start at login.

## Install

1. Build or download `A50 Control_<version>_amd64.AppImage`, make it executable, and
   put it somewhere permanent (e.g. `~/Applications`).
2. Run it and use **This computer → Set up system permissions…** (or install
   `packaging/70-a50-control.rules` into `/etc/udev/rules.d/` yourself). Your user must
   be in the `plugdev` group.
3. Tick **Start at login**. If you later move the AppImage, just run it once from the
   new location and the login entry updates itself.

Quit any other program that talks to the base station (the old `a50-watcher`, or a Wine
prefix with HID access): hidraw delivers every reply to every open handle.

## Build

```sh
npm install
npm run build        # → src-tauri/target/release/bundle/appimage/
npm run dev          # development run
```

Needs Rust, Node, and the WebKitGTK 4.1 development package.

## How it works

- **One owner for the device.** A single service thread is the only code that opens the
  base's hidraw node. It drains stale reports before each request and matches replies to
  requests (echoed command byte, or expected length), which keeps it immune to other
  processes' traffic.
- **Headset state** comes from `GET_HEADSET_STATUS` (0x54): bit 0 docked, bit 1 on.
  A state must hold for 3 s before audio switches (lifting a switched-off headset reports
  "on" for ~2 s).
- **Battery in the OS.** UPower has no API for adding batteries, so the app creates a
  virtual HID device through `/dev/uhid` with a Battery Strength (and Charging) input.
  The kernel registers it as a peripheral `power_supply` that UPower lists. The udev
  rules tag its sibling input device `SOUND_INITIALIZED=1` / `SOUND_FORM_FACTOR=headset`,
  which is how UPower 1.90 decides the battery is a headset.

## Protocol notes

Requests are `02 <cmd> [<len> <payload…>]`, replies `02 <result> <len> <data…>`
(result 0x02 OK, 0x01 error with a message). Most command numbers come from the
[eh-fifty](https://pypi.org/project/eh-fifty) project. Auto power-off was found by
disassembling Astro Command Center: read `0x78`, set `0x74`, value in minutes, UI
choices 10/15/20/25/30.

From the headset firmware (`mimasrx` v39964): while the headset is still, its
accelerometer is checked every 5 s. Upright (within roughly ±15°), it powers off after
the auto power-off minutes. **Tilted beyond that, it powers off after ~15 s**, a limit
that is hard-coded and not reachable from the base protocol or Command Center.
