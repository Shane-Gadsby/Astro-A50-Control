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

Needs Rust, Node, the WebKitGTK 4.1 development package (`libwebkit2gtk-4.1-dev`) and,
for the AppImage's tray icon, the Ayatana AppIndicator development package:

```sh
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev
```

The bundler only uses `pkg-config` to locate the AppIndicator library. Without the dev
package it aborts with "Can't detect any appindicator library". If installing it isn't an
option, a `.pc` file pointing at the runtime library works too:

```sh
mkdir -p /tmp/pc && cat > /tmp/pc/ayatana-appindicator3-0.1.pc <<'PC'
libdir=/usr/lib/x86_64-linux-gnu
Name: ayatana-appindicator3-0.1
Description: runtime-only shim for the Tauri bundler
Version: 0.5.93
Libs: -L${libdir} -layatana-appindicator3
PC
PKG_CONFIG_PATH=/tmp/pc TAURI_LINUX_AYATANA_APPINDICATOR=true npm run build
```

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

## Tilt auto-off (hard-coded in headset firmware)

Looking down while sitting still turns the headset off in about 15 seconds. This is not a
setting: it is fixed in the headset firmware and cannot be changed through the base
station protocol or Astro Command Center. Notes kept here in case we ever try patching the
firmware.

**Source analysed:** `mimasrx_v39964_6dec2021.bin` (headset/"Rx" firmware) from
`mimas_220420021220_tm-40372_tr-43_rm-39964_rr-43.afw`. The `.afw` is a zip archive with
its `PK` signatures replaced by `AG`; it also holds the base firmware
(`mimastx_v40372`, STM32, loads at `0x0800C000`) and two Avnera radio images.

| | SHA-256 |
|---|---|
| `.afw` | `376edcad419339586878572d5e1619a693f46b795cf113b35806dc079764cd16` |
| `mimasrx_v39964_6dec2021.bin` | `8e7b656b57287ecea24b0266259a8c1b5f0f8a2f21342d0d84944585caa0b85c` |

**How it works.** ARM Thumb (Cortex-M0-class), loads at `0x08005000` (file offset =
address − `0x08005000`). The accelerometer thread (LIS2DH) runs a state machine
`UNINIT → START → ACTIVE → TIMEOUT → SLEEP`. Motion resets the counters. While still, a
5-second timer reads X/Y/Z and classifies the orientation:

- **Upright ("tick V")**: X negative and |Y|, |Z| ≤ 250 (about ±15°). Powers off when
  the V count reaches `auto_power_off_minutes × 12`. The minutes byte is the value set
  with command `0x74`, synced from the base into RAM `0x200004CD`.
- **Tilted ("tick H")**: anything else. Powers off when the H count exceeds 2, i.e. on
  the third still tick, ~15 s.

**Patch points** (address / file offset / bytes as shipped / instruction):

| Address | Offset | Bytes | Instruction | Meaning |
|---|---|---|---|---|
| `0x080071CE` | `0x21CE` | `02 2A` | `cmp r2, #2` | tilted ticks allowed before power-off |
| `0x0800716C` | `0x216C` | `FA 35` | `adds r5, #0xfa` | Y lower bound (−250) |
| `0x08007170` | `0x2170` | `FA 29` | `cmp r1, #0xfa` | Y upper bound (+250) |
| `0x0800717C` | `0x217C` | `FA 32` | `adds r2, #0xfa` | Z lower bound (−250) |
| `0x08007182` | `0x2182` | `FA 23` | `movs r3, #0xfa` | Z upper bound (+250) |
| `0x080071A0` | `0x21A0` | `0C 22` | `movs r2, #0xc` | upright ticks per minute |
| `0x080072A4` | `0x22A4` | `88 13 00 00` | literal `5000` | tick period in ms |

Raising the `cmp r2, #2` immediate (one byte, max 255 ticks ≈ 21 min) is the smallest
change. Widening the ±250 window instead would make "upright" tolerate more tilt.

**Unknowns and risks (why it hasn't been tried):**

- The update path isn't understood: Command Center sends the image to the base, which
  forwards it to the headset (over the dock contacts or radio). Any checksum, signature
  or version checks in the base or headset bootloader haven't been analysed, so a
  modified image may be rejected or brick the headset.
- There is no known recovery path if a flash fails.
- The base firmware contains a `HID_ERROR_*` family and restricted-message checks for
  radio traffic. The firmware-update commands themselves haven't been identified.

A safe first step would be reverse-engineering the update protocol and integrity checks
from Command Center and the base firmware, before touching the headset.
