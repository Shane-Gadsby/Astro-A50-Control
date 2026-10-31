> [!WARNING]
> **Warning this app contains slop**
>
> Although it was built somewhat from prior works (see [Credits](#credits)), it was very
> much made by AI. Read the code before you trust it, and use it at your own risk.

# A50 Control

Linux companion app for the **Astro A50 Gen 4** headset and base station: headset
settings, battery in the system battery menu, and automatic audio output switching.
A Tauri (Rust + web UI) app, packaged as an AppImage.

Not affiliated with or endorsed by Astro Gaming or Logitech.

## Features

- **Battery** — shown in the tray, in GNOME's battery UI / UPower (as a headset), with
  low-battery notifications at 15% and 5%.
- **Audio switching** — default sink goes to the A50 when it's switched on and off the
  dock, otherwise to the first connected headset in your Bluetooth priority list,
  otherwise to your speakers. Pick from your paired devices, reorder them, and give them
  nicknames. The speakers are detected automatically, and you can pick them yourself at
  first start or later. Sinks are matched by stable device properties, so PipeWire
  renames don't break it.
- **Headset settings** — auto power-off (10–30 min), 3 EQ presets (5-band gain,
  frequency, Q, names), mic level, sidetone, noise gate, mic EQ, stream port mix,
  default game/chat balance, alert volume. Changes apply live and are saved to the
  headset's flash a few seconds after the last change.
- Runs in the tray; closing the window keeps it running. Optional start at login.

## Known limitations

- **Tested on one setup:** Zorin OS 18.1 with GNOME and PipeWire. The base station is
  matched as USB `9886:002c`, so other Astro generations won't work.
- **Looking down while sitting still turns the headset off** in about 15 seconds. This is
  hard-coded in the headset firmware and the app can't change it. See
  [docs/firmware-notes.md](docs/firmware-notes.md).

## Install

There are no prebuilt releases, so start by [building the AppImage](#build).

1. Make `A50 Control_<version>_amd64.AppImage` executable and put it somewhere permanent.
2. Run it and use **This computer → Set up system permissions…** (or install
   `packaging/70-a50-control.rules` into `/etc/udev/rules.d/` yourself). Your user must
   be in the `plugdev` group.
3. Choose your speakers when asked. This is where audio goes when nothing else is in
   use, and you can change it later under **This computer**.
4. Optionally add paired headsets under **This computer → Bluetooth headsets**, highest
   priority first. Nicknames only apply in this app.
5. Tick **Start at login**. If you later move the AppImage, just run it once from the
   new location and the login entry updates itself.

Audio switching needs `pactl` (`pulseaudio-utils` on Debian and Ubuntu), the Bluetooth
list needs BlueZ and `busctl` (part of systemd), and the in-app permission setup needs
`pkexec`.

Quit any other program that talks to the base station (such as a Wine prefix with HID
access): hidraw delivers every reply to every open handle.

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
[eh-fifty](https://github.com/tdryer/eh-fifty) project. Auto power-off was found by
disassembling Astro Command Center: read `0x78`, set `0x74`, value in minutes, UI
choices 10/15/20/25/30.

## Optional: GNOME Power Statistics patch

GNOME Power Statistics 43.0 labels devices only by kind. `contrib/power-statistics`
patches it to show the model name ("Astro A50 Headset") instead. The script is pinned to
Ubuntu's `43.0-2build2` source package, so it needs `deb-src` entries plus `meson`,
`ninja` and the package's build dependencies. It needs no sudo and leaves the system
package alone:

```sh
DEST=/absolute/install/dir contrib/power-statistics/build.sh
```

This installs the binary to `DEST` and a launcher to `~/.local/share/applications` that
overrides the system one.

## Credits

- [eh-fifty](https://github.com/tdryer/eh-fifty) by tdryer (MIT): most of the base
  station's command numbers and encodings.
- `a50-watcher`, an earlier unpublished Node daemon that this app replaces: the headset
  state and audio switching logic.
- Astro Command Center: the auto power-off commands, found by disassembling it.

## License

MIT, see [LICENSE](LICENSE). The exception is
`contrib/power-statistics/model-names.patch`, which modifies GNOME Power Manager and so
stays under that project's GPL-2.0-or-later.
