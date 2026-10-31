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
- **Updates** — looks for new GitHub releases, asks before installing one, then replaces
  the AppImage and restarts. Downloads are checked against a signature built into the
  app. Turn off **Check for updates** to stop it contacting GitHub.
- Runs in the tray; closing the window keeps it running. Optional start at login.

## Known limitations

- **Tested on one setup:** Zorin OS 18.1 with GNOME and PipeWire. The base station is
  matched as USB `9886:002c`, so other Astro generations won't work.
- **Looking down while sitting still turns the headset off** in about 15 seconds. This is
  hard-coded in the headset firmware and the app can't change it. See
  [the research write-up](docs/firmware-notes.md) for how it works and why patching it
  wasn't pursued.

## Install

Download the AppImage from [Releases](https://github.com/Shane-Gadsby/Astro-A50-Control/releases)
if there is one, or [build it](#build).

1. Make the AppImage executable and put it somewhere permanent that your user can write
   to, so it can update itself.
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
git clone https://github.com/Shane-Gadsby/Astro-A50-Control.git
cd Astro-A50-Control
./build.sh
```

`build.sh` checks each prerequisite below and asks before installing anything that's
missing, showing the exact command first. Saying no to any of them stops the script, and
so does running it with no one to answer. When it finishes it prints the AppImage's path,
under `src-tauri/target/release/bundle/appimage/`. It doesn't install the app itself, so
carry on with [Install](#install).

The first build takes several minutes and downloads the Rust crates plus the AppImage
bundling tools. Later builds are quicker.

Installing system packages uses `sudo` with `apt-get`, `dnf` or `pacman`. Only the
`apt-get` path has been tried; the `dnf` and `pacman` package names are untested. On any
other distro, install the prerequisites yourself and run `./build.sh` again.

### Prerequisites

| Prerequisite | Debian / Ubuntu | Fedora | Arch |
|---|---|---|---|
| C compiler and build tools | `build-essential` | `gcc gcc-c++ make` | `base-devel` |
| pkg-config | `pkg-config` | `pkgconf-pkg-config` | `pkgconf` |
| file | `file` | `file` | `file` |
| GTK 3 development files | `libgtk-3-dev` | `gtk3-devel` | `gtk3` |
| WebKitGTK 4.1 development files | `libwebkit2gtk-4.1-dev` | `webkit2gtk4.1-devel` | `webkit2gtk-4.1` |
| D-Bus development files | `libdbus-1-dev` | `dbus-devel` | `dbus` |
| AppIndicator development files | `libayatana-appindicator3-dev` | `libayatana-appindicator-gtk3-devel` | `libayatana-appindicator` |
| Node.js and npm | `nodejs npm` | `nodejs npm` | `nodejs npm` |

Rust is installed with [rustup](https://rustup.rs) into your home folder, because distro
packages are often too old. `curl` is only needed for that step. The Tauri build tool is
an npm package that goes in this folder's `node_modules`.

### Building by hand

With the prerequisites in place:

```sh
npm ci
npm run build        # → src-tauri/target/release/bundle/appimage/
npm run dev          # development run
```

On Debian and Ubuntu, `npm run build` needs `TAURI_LINUX_AYATANA_APPINDICATOR=true` set
when only the Ayatana library is installed. `build.sh` sets this for you.

### Without the AppIndicator development package

The bundler only uses `pkg-config` to locate the AppIndicator library. Without the dev
package it aborts with "Can't detect any appindicator library". If installing it isn't an
option, a `.pc` file pointing at the runtime library works too, and `build.sh` accepts it:

```sh
mkdir -p /tmp/pc && cat > /tmp/pc/ayatana-appindicator3-0.1.pc <<'PC'
libdir=/usr/lib/x86_64-linux-gnu
Name: ayatana-appindicator3-0.1
Description: runtime-only shim for the Tauri bundler
Version: 0.5.93
Libs: -L${libdir} -layatana-appindicator3
PC
PKG_CONFIG_PATH=/tmp/pc ./build.sh
```

## Releasing

Updates only reach apps once a release is published, and they need the signing key.

1. Set the new version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
   `package.json`, then commit and push.
2. Under **Actions → Draft release → Run workflow**, run it on the branch to release. It
   builds and signs the AppImage and creates a draft release. Running it again refreshes
   the same draft.
3. Check the draft, then publish it. Apps find it at their next check.

The workflow needs the updater's private key in the `TAURI_SIGNING_PRIVATE_KEY` repository
secret. The matching public key is in `src-tauri/tauri.conf.json`. Builds from `build.sh`
don't need the key: they just skip the updater's signature files. If the private key is
lost, installed apps can't accept any further updates, and users have to download a new
AppImage by hand.

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
